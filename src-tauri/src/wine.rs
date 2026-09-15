//! Gameyfin's Wine, downloaded the first time a program needs it.

use gameyfin_core::wine;
use tauri::AppHandle;

use crate::error::{CommandResult, Context};
use crate::state::AppState;

/// Downloads Wine when there is none, shown on the game waiting for it. The caller holds the
/// runtime lock.
pub async fn ensure(app: &AppHandle, state: &AppState, game_id: i64) -> CommandResult<()> {
    let config_dir = state.config_dir();
    if wine::installed(&config_dir).is_some() {
        return Ok(());
    }
    let release = wine::latest_release(&state.http())
        .await
        .context("could not find a Wine build to download")?;
    tracing::info!(version = %release.version, "downloading Wine");
    let downloader = gameyfin_core::Downloader::new(state.transfer_http());
    let installed = wine::install(
        &config_dir,
        &release,
        &downloader,
        crate::progress::emitter(app, "wine-progress", Some(game_id)),
    )
    .await
    .context("Could not install Wine")?;
    tracing::info!(version = %installed.version, "Wine installed");
    Ok(())
}
