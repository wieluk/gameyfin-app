//! DXVK and vkd3d-proton, downloaded once and copied into each prefix since Wine ships
//! neither. The two go together: vkd3d-proton has no DXGI of its own and uses DXVK's.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::vulkan::GraphicsTier;

/// Marker recording what is installed, so versions can be shown without unpacking.
const MARKER: &str = ".gameyfin-graphics.json";

/// The last DXVK release for hardware below Vulkan 1.3. Upstream never stabilised a
/// successor on that branch, so it is named rather than discovered.
const LEGACY_DXVK: &str = "1.10.3";

/// GitHub page size. Both projects release often enough that this reaches well back.
const FEED_PAGE: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Component {
    /// Direct3D 8, 9, 10 and 11, plus the DXGI both layers share.
    Dxvk,
    /// Direct3D 12.
    Vkd3d,
}

impl Component {
    /// Directory name under the graphics root, and the key in the marker.
    pub fn as_str(self) -> &'static str {
        match self {
            Component::Dxvk => "dxvk",
            Component::Vkd3d => "vkd3d",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Component::Dxvk => "DXVK",
            Component::Vkd3d => "vkd3d-proton",
        }
    }

    fn releases_api(self) -> &'static str {
        match self {
            Component::Dxvk => "https://api.github.com/repos/doitsujin/dxvk/releases",
            Component::Vkd3d => {
                "https://api.github.com/repos/HansKristian-Work/vkd3d-proton/releases"
            }
        }
    }

    /// The asset published for a version. Matched exactly, so DXVK's `dxvk-native-*`
    /// builds, which are Linux binaries and useless inside a prefix, never match.
    fn asset_for(self, version: &str) -> String {
        match self {
            Component::Dxvk => format!("dxvk-{version}.tar.gz"),
            Component::Vkd3d => format!("vkd3d-proton-{version}.tar.zst"),
        }
    }

    /// The DLLs this component provides, without the `.dll`.
    pub fn dlls(self) -> &'static [&'static str] {
        match self {
            Component::Dxvk => &["d3d8", "d3d9", "d3d10core", "d3d11", "dxgi"],
            Component::Vkd3d => &["d3d12", "d3d12core"],
        }
    }

    /// The archive's 32-bit directory. The projects disagree on the name.
    fn dir32(self) -> &'static str {
        match self {
            Component::Dxvk => "x32",
            Component::Vkd3d => "x86",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComponentRelease {
    pub component: Component,
    pub version: String,
    pub asset: String,
    pub url: String,
    pub size_bytes: u64,
    /// From GitHub's own asset digest. `None` when it is absent, which is reported
    /// rather than silently accepted.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledComponent {
    pub component: Component,
    pub version: String,
    /// The directory holding `x64` and its 32-bit sibling.
    pub root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct InstalledGraphics {
    pub dxvk: Option<InstalledComponent>,
    pub vkd3d: Option<InstalledComponent>,
}

impl InstalledGraphics {
    pub fn get(&self, component: Component) -> Option<&InstalledComponent> {
        match component {
            Component::Dxvk => self.dxvk.as_ref(),
            Component::Vkd3d => self.vkd3d.as_ref(),
        }
    }

    fn set(&mut self, installed: InstalledComponent) {
        match installed.component {
            Component::Dxvk => self.dxvk = Some(installed),
            Component::Vkd3d => self.vkd3d = Some(installed),
        }
    }

    /// The version strings a prefix records, so an upgrade re-prepares it.
    pub fn versions(&self) -> (Option<String>, Option<String>) {
        (
            self.dxvk.as_ref().map(|c| c.version.clone()),
            self.vkd3d.as_ref().map(|c| c.version.clone()),
        )
    }
}

pub fn graphics_root(config_dir: &Path) -> PathBuf {
    config_dir.join("graphics")
}

/// What is installed, trusting the marker only as far as the files it names: a
/// half-extracted component reports as absent rather than failing when a game starts.
pub fn installed(config_dir: &Path) -> InstalledGraphics {
    let path = graphics_root(config_dir).join(MARKER);
    let Ok(text) = std::fs::read_to_string(path) else {
        return InstalledGraphics::default();
    };
    let record: InstalledGraphics = serde_json::from_str(&text).unwrap_or_default();

    let alive = |entry: Option<InstalledComponent>| {
        entry.filter(|c| {
            c.component
                .dlls()
                .iter()
                .any(|dll| c.root.join("x64").join(format!("{dll}.dll")).is_file())
        })
    };

    InstalledGraphics {
        dxvk: alive(record.dxvk),
        vkd3d: alive(record.vkd3d),
    }
}

/// Versions available for a component, newest first.
pub async fn available_versions(
    http: &reqwest::Client,
    component: Component,
) -> CoreResult<Vec<String>> {
    Ok(releases(http, component)
        .await?
        .into_iter()
        .map(|r| r.version)
        .collect())
}

/// Every release that publishes a usable asset, newest first.
pub async fn releases(
    http: &reqwest::Client,
    component: Component,
) -> CoreResult<Vec<ComponentRelease>> {
    // GitHub answers 403 without a user agent, which reads as a permissions problem.
    let feed: Vec<serde_json::Value> = http
        .get(format!("{}?per_page={FEED_PAGE}", component.releases_api()))
        .header(reqwest::header::USER_AGENT, "Gameyfin-Desktop")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(feed
        .into_iter()
        .filter_map(|entry| release_from(component, &entry))
        .collect())
}

/// One release entry from the feed, if it publishes the asset we want.
fn release_from(component: Component, entry: &serde_json::Value) -> Option<ComponentRelease> {
    if entry.get("prerelease").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }

    let tag = entry.get("tag_name").and_then(serde_json::Value::as_str)?;
    let version = tag.trim_start_matches('v').to_string();
    let wanted = component.asset_for(&version);

    let asset = entry
        .get("assets")?
        .as_array()?
        .iter()
        .find(|a| a.get("name").and_then(serde_json::Value::as_str) == Some(wanted.as_str()))?;

    Some(ComponentRelease {
        component,
        version,
        url: asset
            .get("browser_download_url")
            .and_then(serde_json::Value::as_str)?
            .to_string(),
        size_bytes: asset
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        sha256: asset
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(str::to_string),
        asset: wanted,
    })
}

/// The newest DXVK a driver can run. Upstream's floors: 3.x needs Vulkan 1.4, 2.x needs
/// 1.3, below that the 1.10 branch.
pub fn pick_dxvk(available: &[String], tier: GraphicsTier) -> Option<String> {
    let newest = |series: &str| available.iter().find(|v| v.starts_with(series)).cloned();
    let legacy = || {
        available
            .iter()
            .find(|v| v.as_str() == LEGACY_DXVK)
            .cloned()
    };

    match tier {
        GraphicsTier::Current => newest("3.").or_else(|| newest("2.")).or_else(legacy),
        GraphicsTier::Vulkan13 => newest("2.").or_else(legacy),
        GraphicsTier::Legacy => legacy(),
        GraphicsTier::Unsupported => None,
    }
}

/// The vkd3d-proton version to use, which is simply the newest when the driver reaches
/// Vulkan 1.3. Below that no build works and Direct3D 12 stays unavailable.
pub fn pick_vkd3d(available: &[String], tier: GraphicsTier) -> Option<String> {
    tier.supports_d3d12()
        .then(|| available.first().cloned())
        .flatten()
}

/// Download and unpack one component, replacing any version already there.
pub async fn install<F>(
    config_dir: &Path,
    release: &ComponentRelease,
    downloader: &crate::download::Downloader,
    on_progress: F,
) -> CoreResult<InstalledComponent>
where
    F: FnMut(crate::download::Progress) + Send,
{
    let root = graphics_root(config_dir);
    let component_root = root.join(release.component.as_str());
    std::fs::create_dir_all(&component_root)?;

    let archive = root.join(&release.asset);
    downloader
        .download(&release.url, &archive, |r| r, on_progress)
        .await?;

    if let Some(expected) = &release.sha256 {
        let actual = sha256_of(&archive).await?;
        if !actual.eq_ignore_ascii_case(expected) {
            // A kept partial would be resumed onto by the next attempt.
            let _ = std::fs::remove_file(&archive);
            return Err(CoreError::Other(format!(
                "the downloaded {} build did not match its checksum, so it was discarded",
                release.component.label()
            )));
        }
    } else {
        tracing::warn!(
            component = release.component.as_str(),
            version = %release.version,
            "this release publishes no checksum, so the download was not verified"
        );
    }

    let staging = component_root.join(".unpacking");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let unpack_from = archive.clone();
    let unpack_to = staging.clone();
    let payload = tokio::task::spawn_blocking(move || {
        crate::extract::extract(&unpack_from, &unpack_to, |_| {})?;
        find_payload(&unpack_to)
            .ok_or_else(|| CoreError::Other("the archive held no x64 directory to install".into()))
    })
    .await
    .map_err(|e| CoreError::Other(format!("unpacking did not finish: {e}")))??;

    let destination = component_root.join(&release.version);
    let _ = std::fs::remove_dir_all(&destination);
    std::fs::rename(&payload, &destination)?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);

    // Only this version is kept: the cache is tens of megabytes and nothing reads an old one.
    prune_other_versions(&component_root, &release.version);

    let record = InstalledComponent {
        component: release.component,
        version: release.version.clone(),
        root: destination,
    };
    write_marker(config_dir, &record)?;
    Ok(record)
}

/// The directory holding `x64`: DXVK nests it under `dxvk-<version>/` and vkd3d-proton puts
/// it at the top, so this probes rather than assuming.
fn find_payload(root: &Path) -> Option<PathBuf> {
    if root.join("x64").is_dir() {
        return Some(root.to_path_buf());
    }
    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("x64").is_dir())
}

fn prune_other_versions(component_root: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(component_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && entry.file_name().to_string_lossy() != keep {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Merge one component into the marker, leaving the other alone.
fn write_marker(config_dir: &Path, record: &InstalledComponent) -> CoreResult<()> {
    let mut current = installed(config_dir);
    current.set(record.clone());
    let path = graphics_root(config_dir).join(MARKER);
    let text = serde_json::to_string_pretty(&current)
        .map_err(|e| CoreError::Other(format!("could not record the graphics install: {e}")))?;
    std::fs::write(path, text)?;
    Ok(())
}

async fn sha256_of(path: &Path) -> CoreResult<String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Remove everything this module downloaded. Prefixes keep the copies they already have.
pub async fn remove(config_dir: &Path) -> CoreResult<()> {
    let root = graphics_root(config_dir);
    if root.exists() {
        tokio::fs::remove_dir_all(&root).await?;
    }
    Ok(())
}

/// Copies the components' DLLs into a prefix. Copies, not symlinks: a link into the shared
/// cache breaks when that cache is upgraded, which surfaces as a game that stops starting.
pub fn install_into_prefix(prefix: &Path, graphics: &InstalledGraphics) -> CoreResult<Vec<String>> {
    let root = crate::prefix::wine_root(prefix);
    let system32 = root.join("drive_c").join("windows").join("system32");
    let syswow64 = root.join("drive_c").join("windows").join("syswow64");

    if !system32.is_dir() {
        return Err(CoreError::Other(format!(
            "the prefix at {} has no system32 to install into",
            prefix.display()
        )));
    }

    let mut installed_dlls = Vec::new();
    for component in [Component::Dxvk, Component::Vkd3d] {
        let Some(source) = graphics.get(component) else {
            continue;
        };
        for dll in component.dlls() {
            let file = format!("{dll}.dll");
            let from64 = source.root.join("x64").join(&file);
            if !from64.is_file() {
                // DXVK gained d3d8 late, so an older build simply has fewer files.
                tracing::debug!(component = component.as_str(), dll, "not in this build");
                continue;
            }
            std::fs::copy(&from64, system32.join(&file))?;
            installed_dlls.push((*dll).to_string());

            // A 64-bit-only prefix has no syswow64, which is not a failure.
            let from32 = source.root.join(component.dir32()).join(&file);
            if syswow64.is_dir() && from32.is_file() {
                std::fs::copy(&from32, syswow64.join(&file))?;
            }
        }
    }

    installed_dlls.sort();
    installed_dlls.dedup();
    Ok(installed_dlls)
}

/// The overrides a prefix needs, from its own marker, which is the authority on what
/// actually went in.
pub fn overrides_for_prefix(state: &crate::prefix::PrefixState) -> crate::prefix::DllOverrides {
    let mut names: Vec<&str> = Vec::new();
    if state.dxvk.is_some() {
        names.extend(Component::Dxvk.dlls());
    }
    if state.vkd3d.is_some() {
        names.extend(Component::Vkd3d.dlls());
    }
    crate::prefix::DllOverrides::native(&names)
}

/// The overrides that make Wine load the copies just installed.
pub fn overrides_for(dlls: &[String]) -> crate::prefix::DllOverrides {
    let names: Vec<&str> = dlls.iter().map(String::as_str).collect();
    crate::prefix::DllOverrides::native(&names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-graphics-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn dxvk_versions() -> Vec<String> {
        ["3.1", "3.0.2", "2.7", "2.6.2", "1.10.3"]
            .iter()
            .map(|v| (*v).to_string())
            .collect()
    }

    #[test]
    fn the_current_dxvk_line_is_only_offered_to_a_vulkan_1_4_driver() {
        let available = dxvk_versions();
        assert_eq!(
            pick_dxvk(&available, GraphicsTier::Current).as_deref(),
            Some("3.1")
        );
        assert_eq!(
            pick_dxvk(&available, GraphicsTier::Vulkan13).as_deref(),
            Some("2.7")
        );
        assert_eq!(
            pick_dxvk(&available, GraphicsTier::Legacy).as_deref(),
            Some("1.10.3")
        );
        assert_eq!(pick_dxvk(&available, GraphicsTier::Unsupported), None);
    }

    #[test]
    fn a_driver_falls_back_when_its_own_series_is_missing_from_the_feed() {
        // A feed page that has scrolled past the 2.x releases must not leave a 1.3 driver
        // with nothing, and must never hand it a 3.x build it cannot run.
        let available: Vec<String> = ["3.1", "1.10.3"].iter().map(|v| (*v).to_string()).collect();
        assert_eq!(
            pick_dxvk(&available, GraphicsTier::Vulkan13).as_deref(),
            Some("1.10.3")
        );
    }

    #[test]
    fn direct3d_12_is_skipped_below_vulkan_1_3() {
        let available = vec!["3.0.1".to_string(), "2.14.1".to_string()];
        assert_eq!(
            pick_vkd3d(&available, GraphicsTier::Vulkan13).as_deref(),
            Some("3.0.1")
        );
        assert_eq!(pick_vkd3d(&available, GraphicsTier::Legacy), None);
        assert_eq!(pick_vkd3d(&available, GraphicsTier::Unsupported), None);
    }

    #[test]
    fn the_asset_name_never_matches_the_native_build() {
        // `dxvk-native-2.7.tar.gz` sits in the same release and holds Linux binaries.
        assert_eq!(Component::Dxvk.asset_for("2.7"), "dxvk-2.7.tar.gz");
        assert_ne!(Component::Dxvk.asset_for("2.7"), "dxvk-native-2.7.tar.gz");
        assert_eq!(
            Component::Vkd3d.asset_for("3.0.1"),
            "vkd3d-proton-3.0.1.tar.zst"
        );
    }

    #[test]
    fn a_release_entry_is_read_with_its_digest_and_leading_v_stripped() {
        let entry: serde_json::Value = serde_json::from_str(
            r#"{
                "tag_name": "v3.1",
                "assets": [
                    {"name": "dxvk-native-3.1.tar.gz", "browser_download_url": "http://n", "size": 1},
                    {"name": "dxvk-3.1.tar.gz", "browser_download_url": "http://x",
                     "size": 4096, "digest": "sha256:abc123"}
                ]
            }"#,
        )
        .unwrap();

        let release = release_from(Component::Dxvk, &entry).unwrap();
        assert_eq!(release.version, "3.1");
        assert_eq!(release.url, "http://x");
        assert_eq!(release.size_bytes, 4096);
        assert_eq!(release.sha256.as_deref(), Some("abc123"));
    }

    #[test]
    fn a_prerelease_is_never_offered() {
        let entry: serde_json::Value = serde_json::from_str(
            r#"{"tag_name": "v3.2", "prerelease": true,
                "assets": [{"name": "dxvk-3.2.tar.gz", "browser_download_url": "http://x"}]}"#,
        )
        .unwrap();
        assert_eq!(release_from(Component::Dxvk, &entry), None);
    }

    #[test]
    fn the_payload_is_found_whether_or_not_the_archive_nests_it() {
        let dir = scratch("payload");

        let flat = dir.join("flat");
        std::fs::create_dir_all(flat.join("x64")).unwrap();
        assert_eq!(find_payload(&flat), Some(flat.clone()));

        let nested = dir.join("nested");
        std::fs::create_dir_all(nested.join("dxvk-3.1").join("x64")).unwrap();
        assert_eq!(find_payload(&nested), Some(nested.join("dxvk-3.1")));

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(find_payload(&empty), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A component whose files exist, so `install_into_prefix` has something to copy.
    fn fake_component(root: &Path, component: Component, version: &str) -> InstalledComponent {
        let dir = root.join(component.as_str()).join(version);
        for arch in ["x64", component.dir32()] {
            std::fs::create_dir_all(dir.join(arch)).unwrap();
            for dll in component.dlls() {
                std::fs::write(dir.join(arch).join(format!("{dll}.dll")), arch).unwrap();
            }
        }
        InstalledComponent {
            component,
            version: version.to_string(),
            root: dir,
        }
    }

    #[test]
    fn the_32_bit_dlls_land_in_syswow64_and_the_rest_in_system32() {
        let dir = scratch("copy");
        let prefix = dir.join("pfx");
        let windows = prefix.join("drive_c").join("windows");
        std::fs::create_dir_all(windows.join("system32")).unwrap();
        std::fs::create_dir_all(windows.join("syswow64")).unwrap();

        let graphics = InstalledGraphics {
            dxvk: Some(fake_component(&dir, Component::Dxvk, "3.1")),
            vkd3d: Some(fake_component(&dir, Component::Vkd3d, "3.0.1")),
        };

        let installed_dlls = install_into_prefix(&prefix, &graphics).unwrap();
        assert!(installed_dlls.contains(&"d3d11".to_string()));
        assert!(installed_dlls.contains(&"d3d12".to_string()));
        assert!(installed_dlls.contains(&"dxgi".to_string()));

        // The right build has to land in each directory, not merely some file.
        assert_eq!(
            std::fs::read_to_string(windows.join("system32").join("d3d11.dll")).unwrap(),
            "x64"
        );
        assert_eq!(
            std::fs::read_to_string(windows.join("syswow64").join("d3d11.dll")).unwrap(),
            "x32"
        );
        assert_eq!(
            std::fs::read_to_string(windows.join("syswow64").join("d3d12.dll")).unwrap(),
            "x86"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_prefix_without_syswow64_is_installed_into_rather_than_refused() {
        let dir = scratch("no-wow64");
        let prefix = dir.join("pfx");
        let system32 = prefix.join("drive_c").join("windows").join("system32");
        std::fs::create_dir_all(&system32).unwrap();

        let graphics = InstalledGraphics {
            dxvk: Some(fake_component(&dir, Component::Dxvk, "3.1")),
            vkd3d: None,
        };

        let installed_dlls = install_into_prefix(&prefix, &graphics).unwrap();
        assert!(installed_dlls.contains(&"dxgi".to_string()));
        assert!(system32.join("dxgi.dll").is_file());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_proton_prefix_gets_its_dlls_under_pfx() {
        // Proton builds the real prefix in `pfx`, so installing at the root would put the
        // DLLs somewhere nothing loads them from.
        let dir = scratch("proton");
        let prefix = dir.join("compatdata");
        let system32 = prefix
            .join("pfx")
            .join("drive_c")
            .join("windows")
            .join("system32");
        std::fs::create_dir_all(&system32).unwrap();

        let graphics = InstalledGraphics {
            dxvk: Some(fake_component(&dir, Component::Dxvk, "3.1")),
            vkd3d: None,
        };

        install_into_prefix(&prefix, &graphics).unwrap();
        assert!(system32.join("d3d11.dll").is_file());
        assert!(!prefix.join("drive_c").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_component_whose_files_are_gone_reports_as_absent() {
        let dir = scratch("marker");
        let root = graphics_root(&dir);
        std::fs::create_dir_all(&root).unwrap();

        let record = InstalledGraphics {
            dxvk: Some(InstalledComponent {
                component: Component::Dxvk,
                version: "3.1".to_string(),
                root: root.join("dxvk").join("3.1"),
            }),
            vkd3d: None,
        };
        std::fs::write(
            root.join(MARKER),
            serde_json::to_string_pretty(&record).unwrap(),
        )
        .unwrap();

        assert_eq!(installed(&dir).dxvk, None, "the files were never unpacked");

        std::fs::create_dir_all(root.join("dxvk").join("3.1").join("x64")).unwrap();
        std::fs::write(
            root.join("dxvk").join("3.1").join("x64").join("dxgi.dll"),
            "x",
        )
        .unwrap();
        assert_eq!(installed(&dir).dxvk.unwrap().version, "3.1");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_prefix_prepared_without_vkd3d_never_overrides_d3d12() {
        // Pointing d3d12 at a native copy that was never installed would leave Direct3D 12
        // worse off than Wine's own builtin.
        let state = crate::prefix::PrefixState {
            dxvk: Some("3.1".to_string()),
            vkd3d: None,
            ..crate::prefix::PrefixState::default()
        };
        let env = overrides_for_prefix(&state).to_env();
        assert!(env.contains("dxgi=native,builtin"), "{env}");
        assert!(!env.contains("d3d12"), "{env}");
    }

    #[test]
    fn a_prefix_prepared_with_nothing_sets_no_overrides_at_all() {
        assert!(overrides_for_prefix(&crate::prefix::PrefixState::default()).is_empty());
    }

    #[test]
    fn the_overrides_name_every_dll_that_was_copied() {
        let dlls = vec!["d3d11".to_string(), "dxgi".to_string()];
        assert_eq!(
            overrides_for(&dlls).to_env(),
            "d3d11=native,builtin;dxgi=native,builtin"
        );
    }
}
