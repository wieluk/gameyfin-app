//! DXVK and vkd3d-proton for Gameyfin's Wine: without them Wine renders Direct3D over OpenGL
//! and D3D12 games do not start. Proton brings its own.

use gameyfin_core::graphics::{self, Component, ComponentRelease, InstalledGraphics};
use gameyfin_core::vulkan::{GraphicsTier, VulkanSupport};
use tauri::AppHandle;

use crate::error::{CommandResult, Context};
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

/// An unreachable feed is empty, so a launch still runs with what is installed.
async fn releases(state: &AppState, component: Component) -> Vec<ComponentRelease> {
    graphics::releases(&state.http(), component)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(component = component.as_str(), error = %e, "could not check for a release");
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

/// Downloads what the driver supports and is missing, so an ordinary launch makes no request.
/// The caller holds the runtime lock.
pub async fn ensure_installed(
    app: &AppHandle,
    state: &AppState,
    game_id: i64,
) -> CommandResult<InstalledGraphics> {
    let config_dir = state.config_dir();
    let installed = graphics::installed(&config_dir);
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
    for component in missing {
        let Some(release) = pick(releases(state, component).await, component, tier) else {
            continue;
        };
        // Not fatal: the prefix still runs, and its marker makes the next launch retry.
        if let Err(e) = download(app, state, &release, game_id).await {
            tracing::warn!(component = component.as_str(), "{e}");
        }
    }
    Ok(graphics::installed(&config_dir))
}

async fn download(
    app: &AppHandle,
    state: &AppState,
    release: &ComponentRelease,
    game_id: i64,
) -> CommandResult<()> {
    tracing::info!(component = release.component.as_str(), version = %release.version, "downloading a graphics component");
    let downloader = gameyfin_core::Downloader::new(state.transfer_http());
    graphics::install(
        &state.config_dir(),
        release,
        &downloader,
        crate::progress::emitter(app, "graphics-progress", Some(game_id)),
    )
    .await
    .context(format!("Could not install {}", release.component.label()))?;
    Ok(())
}
