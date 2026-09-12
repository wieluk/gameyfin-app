//! DXVK and vkd3d-proton: without them Wine renders Direct3D over OpenGL and D3D12 games
//! do not start.

use gameyfin_core::graphics::{self, Component, ComponentRelease, InstalledGraphics};
use gameyfin_core::vulkan::{GraphicsTier, VulkanSupport};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{CommandError, CommandResult, Context};
use crate::state::AppState;

/// The probe spawns a process and its answer cannot change while the app runs.
static VULKAN: tokio::sync::OnceCell<VulkanSupport> = tokio::sync::OnceCell::const_new();

pub async fn vulkan_support() -> &'static VulkanSupport {
    VULKAN
        .get_or_init(|| async {
            let Ok(program) = std::env::current_exe() else {
                return VulkanSupport::default();
            };
            let support = gameyfin_core::vulkan::detect_with(&program).await;
            tracing::info!(vulkan = %support.label(), tier = support.tier().label(), "Vulkan probed");
            support
        })
        .await
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphicsStatus {
    pub installed: InstalledGraphics,
    pub vulkan: VulkanSupport,
    pub vulkan_label: String,
    pub latest_dxvk: Option<ComponentRelease>,
    pub latest_vkd3d: Option<ComponentRelease>,
    /// What this driver should run, which is not always the newest release.
    pub recommended_dxvk: Option<String>,
    pub recommended_vkd3d: Option<String>,
    pub enabled: bool,
}

/// An unreachable feed is empty, so what is installed still shows.
async fn releases(state: &AppState, component: Component) -> Vec<ComponentRelease> {
    graphics::releases(&state.http(), component)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(component = component.as_str(), error = %e, "could not check for an update");
            Vec::new()
        })
}

fn pick(
    releases: Vec<ComponentRelease>,
    component: Component,
    tier: GraphicsTier,
) -> Option<ComponentRelease> {
    let versions: Vec<String> = releases.iter().map(|r| r.version.clone()).collect();
    let wanted = match component {
        Component::Dxvk => graphics::pick_dxvk(&versions, tier),
        Component::Vkd3d => graphics::pick_vkd3d(&versions, tier),
    }?;
    releases.into_iter().find(|r| r.version == wanted)
}

#[tauri::command]
pub async fn graphics_status(state: State<'_, AppState>) -> CommandResult<GraphicsStatus> {
    let support = vulkan_support().await.clone();
    let tier = support.tier();
    let (dxvk, vkd3d) = tokio::join!(
        releases(&state, Component::Dxvk),
        releases(&state, Component::Vkd3d)
    );
    let latest_dxvk = pick(dxvk, Component::Dxvk, tier);
    let latest_vkd3d = pick(vkd3d, Component::Vkd3d, tier);
    Ok(GraphicsStatus {
        installed: graphics::installed(&state.config_dir()),
        vulkan_label: support.label(),
        recommended_dxvk: latest_dxvk.as_ref().map(|r| r.version.clone()),
        recommended_vkd3d: latest_vkd3d.as_ref().map(|r| r.version.clone()),
        latest_dxvk,
        latest_vkd3d,
        enabled: state.settings().graphics_components,
        vulkan: support,
    })
}

/// Downloads a component, replacing whatever version is there.
#[tauri::command]
pub async fn install_graphics(
    app: AppHandle,
    state: State<'_, AppState>,
    component: String,
    version: Option<String>,
) -> CommandResult<InstalledGraphics> {
    let component = match component.as_str() {
        "dxvk" => Component::Dxvk,
        "vkd3d" => Component::Vkd3d,
        other => {
            return Err(CommandError::msg(format!(
                "{other} is not a graphics component."
            )))
        }
    };
    let all = graphics::releases(&state.http(), component)
        .await
        .context(format!(
            "could not find a {} build to download",
            component.label()
        ))?;
    let release = match version {
        Some(version) => all.into_iter().find(|r| r.version == version),
        None => pick(all, component, vulkan_support().await.tier()),
    }
    .ok_or_else(|| {
        CommandError::msg(format!(
            "No {} build works with this system's Vulkan driver.",
            component.label()
        ))
    })?;

    let _runtime_lock = state.runtime_lock().await;
    download(&app, &state, &release).await?;
    let _ = app.emit("graphics-changed", ());
    Ok(graphics::installed(&state.config_dir()))
}

/// Prefixes keep the copies already inside them.
#[tauri::command]
pub async fn remove_graphics(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    graphics::remove(&state.config_dir())
        .await
        .context("could not remove the components")?;
    let _ = app.emit("graphics-changed", ());
    Ok(())
}

/// Installs only what is missing, so an ordinary launch makes no request. The caller holds
/// the runtime lock.
pub async fn ensure_installed(
    app: &AppHandle,
    state: &AppState,
) -> CommandResult<InstalledGraphics> {
    let config_dir = state.config_dir();
    let installed = graphics::installed(&config_dir);
    if !state.settings().graphics_components {
        return Ok(InstalledGraphics::default());
    }
    let tier = vulkan_support().await.tier();
    if !tier.supports_dxvk() {
        tracing::warn!("no usable Vulkan, so Direct3D falls back to WineD3D");
        return Ok(InstalledGraphics::default());
    }
    // Below Vulkan 1.3 no vkd3d-proton build runs, so it is moot rather than missing.
    let missing: Vec<Component> = [Component::Dxvk, Component::Vkd3d]
        .into_iter()
        .filter(|c| installed.get(*c).is_none() && (*c == Component::Dxvk || tier.supports_d3d12()))
        .collect();
    if missing.is_empty() {
        return Ok(installed);
    }
    for component in missing {
        let Some(release) = pick(releases(state, component).await, component, tier) else {
            continue;
        };
        // Not fatal: the prefix still runs, and its marker makes the next launch retry.
        if let Err(e) = download(app, state, &release).await {
            tracing::warn!(component = component.as_str(), "{e}");
        }
    }
    let _ = app.emit("graphics-changed", ());
    Ok(graphics::installed(&config_dir))
}

async fn download(
    app: &AppHandle,
    state: &AppState,
    release: &ComponentRelease,
) -> CommandResult<()> {
    tracing::info!(component = release.component.as_str(), version = %release.version, "downloading a graphics component");
    let downloader = gameyfin_core::Downloader::new(state.transfer_http());
    graphics::install(
        &state.config_dir(),
        release,
        &downloader,
        crate::progress::emitter(app, "graphics-progress"),
    )
    .await
    .context(format!("Could not install {}", release.component.label()))?;
    Ok(())
}
