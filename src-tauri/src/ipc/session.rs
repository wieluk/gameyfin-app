//! Connecting, signing in, settings and games folders.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::auth_flow::{self, ServerProbe};
use crate::error::{blocking, CommandError, CommandResult, Context};
use crate::settings::{PublicSettings, Settings, SettingsPatch};
use crate::state::AppState;

/// What the stored login is, which decides whether it can outlive the server's session.
#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum LoginKind {
    DeviceToken,
    Session,
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionStatus {
    pub configured: bool,
    pub authenticated: bool,
    /// The server did not answer; the session is kept and the cached library shown.
    pub offline: bool,
    pub server_url: Option<String>,
    pub username: Option<String>,
    /// None when nothing is stored to sign in with.
    pub login: Option<LoginKind>,
}

#[tauri::command]
pub async fn connection_status(state: State<'_, AppState>) -> CommandResult<ConnectionStatus> {
    // Answering from the defaults startup has not overwritten yet would report a signed-in
    // user as configured-from-scratch, and the window would show the wizard.
    if !state.wait_until_restored().await {
        tracing::warn!("startup has not read the settings yet; reporting what is in memory");
    }
    let settings = state.settings();
    let (authenticated, offline) = state.check_session(settings.has_session()).await;
    let login = if settings.device_token.is_some() {
        Some(LoginKind::DeviceToken)
    } else if !settings.cookies.is_empty() {
        Some(LoginKind::Session)
    } else {
        None
    };
    Ok(ConnectionStatus {
        configured: settings.is_configured(),
        authenticated,
        offline,
        server_url: settings.server_url,
        username: settings.username,
        login,
    })
}

/// None when the server does not report one, which older servers do not.
#[tauri::command]
pub async fn server_version(state: State<'_, AppState>) -> CommandResult<Option<String>> {
    let Some(client) = state.client() else {
        return Ok(None);
    };
    Ok(client.server_version().await.unwrap_or_else(|e| {
        tracing::debug!("the server does not report its version: {e}");
        None
    }))
}

#[tauri::command]
pub async fn probe_server(state: State<'_, AppState>, url: String) -> CommandResult<ServerProbe> {
    Ok(auth_flow::probe(&state.http(), &url).await)
}

#[tauri::command]
pub async fn set_server_url(state: State<'_, AppState>, url: String) -> CommandResult<String> {
    let normalized = auth_flow::normalize_url(&url)
        .ok_or_else(|| CommandError::msg("That does not look like a valid address."))?;
    let switching = state.settings().server_url.as_deref() != Some(normalized.as_str());
    if switching {
        // The old session and catalogue belong to the other server.
        state.revoke_device_token().await;
        state.disconnect();
        state.forget_catalog().await;
    }
    state
        .set_settings(|s| {
            if switching {
                s.clear_session();
            }
            s.server_url = Some(normalized.clone());
        })
        .await?;
    Ok(normalized)
}

/// `direct` demands the password form even when the server has SSO.
#[tauri::command]
pub async fn begin_login(
    app: AppHandle,
    state: State<'_, AppState>,
    direct: Option<bool>,
) -> CommandResult<()> {
    let url = state
        .settings()
        .server_url
        .ok_or_else(|| CommandError::msg("No server configured yet."))?;
    auth_flow::open_login_window(&app, &url, direct.unwrap_or(false)).map_err(CommandError::Message)
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LoginPoll {
    pub signed_in: bool,
    pub window_open: bool,
    /// Cookies exist but do not authenticate yet, usually a proxy still in the way.
    pub detail: Option<String>,
}

/// Succeeds only once the harvested cookies actually authenticate.
#[tauri::command]
pub async fn poll_login(app: AppHandle, state: State<'_, AppState>) -> CommandResult<LoginPoll> {
    let window_open = auth_flow::login_window_open(&app);
    let waiting = |detail: Option<String>| LoginPoll {
        signed_in: false,
        window_open,
        detail,
    };

    let Some(url) = state.settings().server_url else {
        return Ok(waiting(Some("No server configured.".into())));
    };
    let cookies = auth_flow::harvest_cookies(&app, &url);
    if cookies.is_empty() {
        return Ok(waiting(None));
    }

    state.connect_with_cookies(&url, cookies.clone()).await?;
    let user = match state.client() {
        Some(client) => client.user_info().await.ok().flatten(),
        None => None,
    };
    // Only report: navigating the window here restarts an SSO flow and breaks its callback.
    let Some(user) = user else {
        return Ok(waiting(Some(format!(
            "Waiting for the server to accept the session ({} cookies so far).",
            cookies.len()
        ))));
    };

    state
        .set_settings(|s| {
            s.cookies = cookies;
            // A token that stopped working is what sent the user here.
            s.device_token = None;
            s.username = Some(user.username);
        })
        .await?;
    state.adopt_device_token(&url).await;
    auth_flow::close_login_window(&app);
    Ok(LoginPoll {
        signed_in: true,
        window_open: false,
        detail: None,
    })
}

/// Clears the sign-in data and its session, or the app stays signed in as the account being
/// left. Unlike signing out, a failure to clear is reported.
#[tauri::command]
pub async fn reset_login(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    forget_session(&state).await?;
    auth_flow::reset_login_profile(&app)
        .await
        .map_err(CommandError::Message)
}

#[tauri::command]
pub async fn cancel_login(app: AppHandle) -> CommandResult<()> {
    auth_flow::close_login_window(&app);
    Ok(())
}

/// Forgets the session, the catalogue and the sign-in profile, keeping the server address.
#[tauri::command]
pub async fn sign_out(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    forget_session(&state).await?;
    // Otherwise the next sign-in silently reuses this account's provider session. Only
    // logged: the user is signed out either way.
    if let Err(e) = auth_flow::reset_login_profile(&app).await {
        tracing::warn!("{e}");
    }
    Ok(())
}

/// Drops the session and everything fetched with it, keeping the server address.
async fn forget_session(state: &AppState) -> CommandResult<()> {
    // Otherwise the server keeps listing a device that has signed out.
    state.revoke_device_token().await;
    state.disconnect();
    state.forget_catalog().await;
    state.set_settings(Settings::clear_session).await
}

/// The close button may only hide the window, so the UI needs a way to quit.
#[tauri::command]
pub async fn quit_app(app: AppHandle) -> CommandResult<()> {
    crate::tray::quit_now(&app);
    Ok(())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CommandResult<PublicSettings> {
    // Same reason as `connection_status`: defaults are not this user's settings.
    state.wait_until_restored().await;
    Ok(state.settings().into())
}

/// Applies a partial change, and the live counterpart of settings that have one.
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> CommandResult<()> {
    if let Some(enabled) = patch.autostart {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .context("could not change the startup setting")?;
    }

    let (settings, store_changed) = state
        .update_settings(|s| {
            let before = s.save_store_identity();
            patch.apply(s);
            Ok((s.clone(), s.save_store_identity() != before))
        })
        .await?;

    if store_changed {
        // Otherwise the old store's ids reach the new one as the base of the next upload.
        state.library().forget_synced_save_ids().await;
        tracing::info!("the save store changed, so every game's last synced version was forgotten");
    }

    if let Some(level) = patch.log_level {
        crate::set_log_level(level).map_err(CommandError::Message)?;
    }
    if patch.touches_gamepad() {
        state.apply_gamepad_settings();
    }
    if let Some(kib) = patch.download_limit_kib {
        state.download_limit().set(u64::from(kib) * 1024);
    }
    if let Some(root) = patch
        .library_root
        .as_deref()
        .filter(|_| settings.library_root.is_some())
    {
        let found = state.library().rescan(Path::new(root)).await;
        tracing::info!(root, found, "rescanned the new games folder");
        super::notify(&app);
    }
    Ok(())
}

/// Settings back to defaults, and the live counterparts with them. Per-game options stay.
#[tauri::command]
pub async fn reset_settings(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    if state.settings().autostart {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch()
            .disable()
            .context("could not change the startup setting")?;
    }
    let settings = state
        .update_settings(|s| {
            s.reset_preferences();
            Ok(s.clone())
        })
        .await?;
    crate::set_log_level(settings.log_level).map_err(CommandError::Message)?;
    state.apply_gamepad_settings();
    state
        .download_limit()
        .set(u64::from(settings.download_limit_kib) * 1024);
    tracing::info!("settings reset to defaults");
    Ok(())
}

#[tauri::command]
pub async fn suggest_library_root(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    if let Some(existing) = state.settings().library_root {
        return Ok(existing);
    }
    let home = app
        .path()
        .home_dir()
        .context("could not find your home directory")?;
    Ok(Settings::default_library_root(&home)
        .to_string_lossy()
        .into_owned())
}

#[tauri::command]
pub async fn config_directory(state: State<'_, AppState>) -> CommandResult<String> {
    Ok(state.config_dir().to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn log_directory() -> CommandResult<String> {
    Ok(crate::log_directory().to_string_lossy().into_owned())
}

/// A packaged build has no console, so a render error would otherwise leave no trace.
#[tauri::command]
pub async fn report_crash(details: String) -> CommandResult<()> {
    let details: String = details.chars().take(4000).collect();
    tracing::error!(details = details.trim(), "the interface crashed");
    Ok(())
}

#[tauri::command]
pub async fn image_cache_size(state: State<'_, AppState>) -> CommandResult<u64> {
    let Some(cache) = state.image_cache() else {
        return Ok(0);
    };
    Ok(tokio::task::spawn_blocking(move || cache.size())
        .await
        .unwrap_or(0))
}

#[tauri::command]
pub async fn clear_image_cache(state: State<'_, AppState>) -> CommandResult<()> {
    if let Some(cache) = state.image_cache() {
        cache
            .clear()
            .await
            .context("could not clear the image cache")?;
    }
    Ok(())
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MemoryInfo {
    pub automatic_mib: Option<u64>,
}

#[tauri::command]
pub async fn memory_info() -> CommandResult<MemoryInfo> {
    let total = gameyfin_core::process::total_memory_bytes();
    Ok(MemoryInfo {
        automatic_mib: crate::settings::InstallerMemoryLimit::Auto.resolve(total),
    })
}

/// The stored choice, else the server's first. A provider disabled server-side falls back.
pub fn choose_provider<'a>(
    providers: &'a [gameyfin_api::DownloadProvider],
    preferred: Option<&str>,
) -> Option<&'a gameyfin_api::DownloadProvider> {
    preferred
        .and_then(|key| providers.iter().find(|p| p.key == key))
        .or_else(|| providers.first())
}

#[derive(Serialize, Clone, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderChoice {
    pub key: String,
    pub name: String,
    pub description: String,
    pub selected: bool,
    pub needs_torrent_client: bool,
}

#[tauri::command]
pub async fn download_providers(state: State<'_, AppState>) -> CommandResult<Vec<ProviderChoice>> {
    let providers = state.require_client()?.download_providers().await?;
    let preferred = state.settings().download_provider;
    let chosen = choose_provider(&providers, preferred.as_deref()).map(|p| p.key.clone());
    Ok(providers
        .into_iter()
        .map(|p| ProviderChoice {
            selected: Some(&p.key) == chosen.as_ref(),
            needs_torrent_client: serves_torrent(&p),
            description: p.short_description.unwrap_or(p.description),
            key: p.key,
            name: p.name,
        })
        .collect())
}

/// The server describes providers only in prose, so this is a guess that drives a warning.
fn serves_torrent(provider: &gameyfin_api::DownloadProvider) -> bool {
    format!("{} {}", provider.key, provider.name)
        .to_ascii_lowercase()
        .contains("torrent")
}

/// `None` restores the server's order. Mirrored to the server so the web UI agrees.
#[tauri::command]
pub async fn set_download_provider(
    state: State<'_, AppState>,
    key: Option<String>,
) -> CommandResult<()> {
    state
        .set_settings(|s| s.download_provider = key.clone())
        .await?;
    if let (Some(key), Some(client)) = (key, state.client()) {
        if let Err(e) = client
            .set_user_preference("preferred-download-method", &key)
            .await
        {
            tracing::debug!(error = %e, "could not mirror the provider choice to the server");
        }
    }
    Ok(())
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryRoot {
    pub path: String,
    pub is_default: bool,
    pub free_bytes: Option<u64>,
    pub exists: bool,
}

#[tauri::command]
pub async fn list_library_roots(state: State<'_, AppState>) -> CommandResult<Vec<LibraryRoot>> {
    let roots = state.settings().library_roots();
    blocking("could not read your games folders", move || {
        Ok::<_, std::convert::Infallible>(
            roots
                .into_iter()
                .enumerate()
                .map(|(index, path)| {
                    let dir = PathBuf::from(&path);
                    LibraryRoot {
                        is_default: index == 0,
                        // An unmounted drive reports nothing, not its mount point's space.
                        free_bytes: dir
                            .exists()
                            .then(|| gameyfin_core::download::available_space(&dir))
                            .flatten(),
                        exists: dir.is_dir(),
                        path,
                    }
                })
                .collect(),
        )
    })
    .await
}

#[tauri::command]
pub async fn add_library_root(state: State<'_, AppState>, path: String) -> CommandResult<()> {
    let path = path.trim().to_string();
    let dir = PathBuf::from(&path);
    if path.is_empty() || !dir.is_absolute() {
        return Err(CommandError::msg("Choose a folder by its full path."));
    }
    if state.settings().is_library_root(&path) {
        return Ok(());
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .context(format!("could not use {path}"))?;
    state
        .set_settings(|s| {
            let mut roots = s.library_roots();
            roots.push(path.clone());
            s.set_library_roots(roots);
        })
        .await
}

/// Only forgets the folder; the games in it stay on disk.
#[tauri::command]
pub async fn remove_library_root(state: State<'_, AppState>, path: String) -> CommandResult<()> {
    state
        .update_settings(|s| {
            let remaining: Vec<String> = s
                .library_roots()
                .into_iter()
                .filter(|root| root != &path)
                .collect();
            if remaining.is_empty() {
                return Err(
                    "At least one games folder is needed. Add another before removing this one."
                        .into(),
                );
            }
            s.set_library_roots(remaining);
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn set_default_library_root(
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<()> {
    state
        .update_settings(|s| {
            if !s.is_library_root(&path) {
                return Err("That folder is not one of your games folders.".into());
            }
            let roots = std::iter::once(path.clone())
                .chain(s.library_roots().into_iter().filter(|r| r != &path))
                .collect();
            s.set_library_roots(roots);
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn list_libraries(
    state: State<'_, AppState>,
) -> CommandResult<Vec<gameyfin_api::Library>> {
    state.require_client()?;
    Ok(state.libraries().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(key: &str, name: &str) -> gameyfin_api::DownloadProvider {
        gameyfin_api::DownloadProvider {
            key: key.to_string(),
            name: name.to_string(),
            priority: 1,
            description: String::new(),
            short_description: None,
        }
    }

    #[test]
    fn the_stored_provider_wins_and_a_missing_one_falls_back() {
        let providers = [
            provider("org.gameyfin.direct", "Direct Download"),
            provider("org.gameyfin.torrent", "Torrent"),
        ];
        let pick = |key| choose_provider(&providers, key).map(|p| p.key.as_str());
        assert_eq!(
            pick(Some("org.gameyfin.torrent")),
            Some("org.gameyfin.torrent")
        );
        assert_eq!(pick(None), Some("org.gameyfin.direct"));
        assert_eq!(
            pick(Some("org.gameyfin.removed")),
            Some("org.gameyfin.direct")
        );
        assert!(choose_provider(&[], Some("x")).is_none());
    }

    #[test]
    fn torrent_providers_are_recognised_by_key_or_name() {
        assert!(serves_torrent(&provider("org.gameyfin.torrent", "Seed")));
        assert!(serves_torrent(&provider("plugin.x", "Torrent Download")));
        assert!(!serves_torrent(&provider("org.gameyfin.direct", "Direct")));
    }
}
