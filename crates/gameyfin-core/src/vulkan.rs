//! What Vulkan this machine can actually do, which is what decides the DXVK generation a
//! prefix gets. Probed in a child process: a broken ICD segfaults whoever loads it.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The argument that turns a copy of this app into a one-shot Vulkan probe.
pub const PROBE_FLAG: &str = "--vulkan-probe";

/// How long the probe gets before it is treated as a broken driver.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What the driver reports, or nothing at all when there is no usable Vulkan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VulkanSupport {
    /// Highest `apiVersion` across the physical devices, as major and minor.
    pub api_version: Option<(u32, u32)>,
    /// The device that reported it, for the log.
    pub device: Option<String>,
}

impl VulkanSupport {
    pub fn tier(&self) -> GraphicsTier {
        match self.api_version {
            Some((major, minor)) if (major, minor) >= (1, 4) => GraphicsTier::Current,
            Some((major, minor)) if (major, minor) >= (1, 3) => GraphicsTier::Vulkan13,
            Some((major, minor)) if (major, minor) >= (1, 1) => GraphicsTier::Legacy,
            _ => GraphicsTier::Unsupported,
        }
    }

    /// What to show the user, e.g. `1.4 (AMD Radeon RX 7800 XT)`.
    pub fn label(&self) -> String {
        match (self.api_version, &self.device) {
            (Some((major, minor)), Some(device)) => format!("{major}.{minor} ({device})"),
            (Some((major, minor)), None) => format!("{major}.{minor}"),
            _ => "not available".to_string(),
        }
    }
}

/// Upstream's floors: DXVK 3.x needs Vulkan 1.4, 2.x needs 1.3, older hardware the 1.10.3
/// branch. vkd3d-proton needs 1.3 whatever DXVK is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GraphicsTier {
    /// Vulkan 1.4 or newer: the current DXVK line.
    Current,
    /// Vulkan 1.3: DXVK 2.x, still with working Direct3D 12.
    Vulkan13,
    /// Vulkan 1.1: the DXVK 1.10.3 branch, and no Direct3D 12.
    Legacy,
    /// Nothing usable, so WineD3D stays in place.
    Unsupported,
}

impl GraphicsTier {
    /// Whether vkd3d-proton is worth installing, which needs Vulkan 1.3.
    pub fn supports_d3d12(self) -> bool {
        matches!(self, GraphicsTier::Current | GraphicsTier::Vulkan13)
    }

    pub fn supports_dxvk(self) -> bool {
        !matches!(self, GraphicsTier::Unsupported)
    }

    /// What to show the user when explaining why a component was skipped.
    pub fn label(self) -> &'static str {
        match self {
            GraphicsTier::Current => "Vulkan 1.4",
            GraphicsTier::Vulkan13 => "Vulkan 1.3",
            GraphicsTier::Legacy => "Vulkan 1.1",
            GraphicsTier::Unsupported => "no usable Vulkan",
        }
    }
}

/// Ask a copy of this program to probe Vulkan. Out of process, since loading a driver can abort.
pub async fn detect_with(program: &Path) -> VulkanSupport {
    let run = tokio::process::Command::new(program)
        .arg(PROBE_FLAG)
        .stdin(std::process::Stdio::null())
        .output();

    let output = match tokio::time::timeout(PROBE_TIMEOUT, run).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "could not run the Vulkan probe");
            return VulkanSupport::default();
        }
        Err(_) => {
            tracing::warn!("the Vulkan probe did not finish; treating Vulkan as unavailable");
            return VulkanSupport::default();
        }
    };

    if !output.status.success() {
        tracing::warn!(status = ?output.status, "the Vulkan probe failed");
        return VulkanSupport::default();
    }

    match serde_json::from_slice(&output.stdout) {
        Ok(support) => support,
        Err(e) => {
            tracing::warn!(error = %e, "the Vulkan probe returned nothing readable");
            VulkanSupport::default()
        }
    }
}

/// Only ever called in the probe child process, never in the app's own.
pub fn probe() -> VulkanSupport {
    match unsafe { probe_inner() } {
        Ok(support) => support,
        Err(e) => {
            tracing::debug!(error = %e, "no usable Vulkan");
            VulkanSupport::default()
        }
    }
}

/// # Safety
/// Loads and calls into the system Vulkan loader, which runs driver code we do not control.
unsafe fn probe_inner() -> Result<VulkanSupport, Box<dyn std::error::Error>> {
    use ash::vk;

    let entry = ash::Entry::load()?;

    // A 1.0 instance is accepted by every loader, and physical devices still report their
    // own real apiVersion, which is the number that decides the DXVK generation.
    let app_info = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 0, 0));
    let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
    let instance = entry.create_instance(&create_info, None)?;

    let mut best: Option<(u32, u32, String)> = None;
    for device in instance.enumerate_physical_devices()? {
        let props = instance.get_physical_device_properties(device);
        let version = (
            vk::api_version_major(props.api_version),
            vk::api_version_minor(props.api_version),
        );
        let name = std::ffi::CStr::from_ptr(props.device_name.as_ptr())
            .to_string_lossy()
            .into_owned();

        if best
            .as_ref()
            .is_none_or(|(major, minor, _)| version > (*major, *minor))
        {
            best = Some((version.0, version.1, name));
        }
    }

    instance.destroy_instance(None);

    Ok(match best {
        Some((major, minor, device)) => VulkanSupport {
            api_version: Some((major, minor)),
            device: Some(device),
        },
        None => VulkanSupport::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(major: u32, minor: u32) -> VulkanSupport {
        VulkanSupport {
            api_version: Some((major, minor)),
            device: Some("Test Device".to_string()),
        }
    }

    #[test]
    fn the_tier_follows_upstreams_own_vulkan_floors() {
        for (major, minor, expected) in [
            (1, 5, GraphicsTier::Current),
            (1, 4, GraphicsTier::Current),
            (1, 3, GraphicsTier::Vulkan13),
            (1, 2, GraphicsTier::Legacy),
            (1, 1, GraphicsTier::Legacy),
            (1, 0, GraphicsTier::Unsupported),
        ] {
            assert_eq!(at(major, minor).tier(), expected, "{major}.{minor}");
        }
    }

    #[test]
    fn a_machine_without_vulkan_gets_no_components_at_all() {
        let none = VulkanSupport::default();
        assert_eq!(none.tier(), GraphicsTier::Unsupported);
        assert!(!none.tier().supports_dxvk());
        assert!(!none.tier().supports_d3d12());
        assert_eq!(none.label(), "not available");
    }

    #[test]
    fn direct3d_12_needs_vulkan_1_3_even_when_dxvk_would_run() {
        // The legacy DXVK branch still renders D3D9 through 11, so the two questions are
        // separate: a 1.1 driver gets DXVK and no vkd3d-proton.
        assert!(GraphicsTier::Legacy.supports_dxvk());
        assert!(!GraphicsTier::Legacy.supports_d3d12());
        assert!(GraphicsTier::Vulkan13.supports_d3d12());
    }

    #[test]
    fn the_label_names_the_device_that_reported_the_version() {
        assert_eq!(at(1, 4).label(), "1.4 (Test Device)");
    }
}
