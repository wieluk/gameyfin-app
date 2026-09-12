//! The Wine build Gameyfin downloads for games Proton cannot run.

use gameyfin_core::wine::{self, InstalledWine, WineChannel, WineStatus};
use tauri::{AppHandle, Emitter, State};

use crate::error::{CommandResult, Context};
use crate::state::AppState;

/// How many past releases the version pickers offer.
pub const RELEASE_CHOICES: usize = 10;

/// The installed half works offline even when the release lookup does not.
#[tauri::command]
pub async fn wine_status(state: State<'_, AppState>) -> CommandResult<WineStatus> {
    let config_dir = state.config_dir();
    let variant = state.settings().wine_variant;
    let installed = wine::installed(&config_dir);
    let installed_channel = installed.as_ref().map(|i| WineChannel::of(&i.version));
    let http = state.http();

    let versions = wine::available_versions(&http, variant)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not check for a Wine update");
            Vec::new()
        });
    let latest = match wine::newest_on(&versions, installed_channel.unwrap_or(WineChannel::Stable))
    {
        Some(version) => wine::release_for(&http, variant, version)
            .await
            .inspect_err(
                |e| tracing::warn!(error = %e, version, "could not look up a Wine release"),
            )
            .ok(),
        None => None,
    };
    let (stable, development): (Vec<String>, Vec<String>) = versions
        .into_iter()
        .partition(|v| WineChannel::of(v) == WineChannel::Stable);

    Ok(WineStatus {
        installed,
        installed_channel,
        latest,
        stable,
        development: development.into_iter().take(RELEASE_CHOICES).collect(),
    })
}

/// Installs, updates and repairs alike, so a repair cannot behave differently.
#[tauri::command]
pub async fn install_wine(
    app: AppHandle,
    state: State<'_, AppState>,
    version: Option<String>,
) -> CommandResult<InstalledWine> {
    let _runtime_lock = state.runtime_lock().await;
    download(&app, &state, version).await
}

/// Downloads Wine when there is none. The caller holds the runtime lock.
pub async fn ensure(app: &AppHandle, state: &AppState) -> CommandResult<()> {
    if wine::installed(&state.config_dir()).is_none() {
        download(app, state, None).await?;
    }
    Ok(())
}

async fn download(
    app: &AppHandle,
    state: &AppState,
    version: Option<String>,
) -> CommandResult<InstalledWine> {
    let config_dir = state.config_dir();
    let variant = state.settings().wine_variant;
    let http = state.http();
    let release = match version.as_deref() {
        Some(version) => wine::release_for(&http, variant, version).await,
        // An update stays on the installed channel; a first install gets stable.
        None => {
            let channel = wine::installed(&config_dir)
                .map_or(WineChannel::Stable, |i| WineChannel::of(&i.version));
            wine::latest_release(&http, variant, channel).await
        }
    }
    .context("could not find a Wine build to download")?;

    tracing::info!(version = %release.version, variant = variant.as_str(), "downloading Wine");
    let downloader = gameyfin_core::Downloader::new(state.transfer_http());
    let installed = wine::install(
        &config_dir,
        &release,
        &downloader,
        crate::progress::emitter(app, "wine-progress"),
    )
    .await
    .context("Could not install Wine")?;
    tracing::info!(version = %installed.version, "Wine installed");
    let _ = app.emit("wine-changed", ());
    Ok(installed)
}

/// Prefixes are left alone, since some hold save data.
#[tauri::command]
pub async fn remove_wine(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    wine::remove(&state.config_dir())
        .await
        .context("could not remove Wine")?;
    let _ = app.emit("wine-changed", ());
    Ok(())
}
