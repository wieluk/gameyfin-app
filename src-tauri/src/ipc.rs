//! Tauri commands, the typed surface the webview calls.
//!
//! Kept thin on purpose: each command validates, delegates, and maps errors. Anything
//! with real logic belongs in a crate under `crates/` so it can be tested without a GUI.

use std::path::{Path, PathBuf};

use gameyfin_api::{Game, Library};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::auth_flow::{self, ServerProbe};
use crate::error::{CommandError, CommandResult};
use crate::library_state::{Activity, GameState, Stage};
use crate::settings::Settings;
use crate::state::AppState;

/// A game plus the client-side state the UI needs to render its tile.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub game: Game,
    pub state: GameState,
    pub minutes_played: u32,
    pub last_played_at: Option<String>,
    /// Whether a downloaded archive is still on disk, so the UI can offer to reclaim it.
    pub archive_present: bool,
    /// Artwork URLs on the app's own `gfimg://` scheme.
    ///
    /// Not the server's URL directly: `/images/**` is access-controlled on instances that
    /// disallow anonymous browsing, and an `<img>` tag cannot present our session.
    pub cover_url: Option<String>,
    pub header_url: Option<String>,
    pub screenshot_urls: Vec<String>,
}

/// What the UI needs to decide between the wizard and the library.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub configured: bool,
    pub authenticated: bool,
    /// The server did not answer. The session is kept and the cached library is shown.
    pub offline: bool,
    pub server_url: Option<String>,
    pub username: Option<String>,
    pub library_root: Option<String>,
}

#[tauri::command]
pub async fn connection_status(state: State<'_, AppState>) -> CommandResult<ConnectionStatus> {
    let settings = state.settings().await;

    // Three outcomes, not two. "The server said you are not signed in" sends the user to
    // the wizard; "the server did not answer" must not, because signing in again is
    // exactly what an offline user cannot do, and their installed games still work.
    let (authenticated, offline) = match state.client().await {
        None => (false, false),
        Some(client) => match client.user_info().await {
            Ok(user) => {
                state.set_reachable(true);
                (user.is_some(), false)
            }
            Err(e) if e.is_auth() => {
                state.set_reachable(true);
                (false, false)
            }
            Err(e) => {
                tracing::debug!("could not reach the server: {e}");
                state.set_reachable(false);
                (settings.has_session(), true)
            }
        },
    };

    Ok(ConnectionStatus {
        configured: settings.is_configured(),
        authenticated,
        offline,
        server_url: settings.server_url.clone(),
        username: settings.username.clone(),
        library_root: settings.library_root.clone(),
    })
}

/// Check an address before committing to it.
#[tauri::command]
pub async fn probe_server(state: State<'_, AppState>, url: String) -> CommandResult<ServerProbe> {
    let http = state.http().await;
    Ok(auth_flow::probe(&http, &url).await)
}

/// Read settings, apply a change, and persist them.
async fn mutate_settings(
    state: &State<'_, AppState>,
    change: impl FnOnce(&mut Settings),
) -> CommandResult<()> {
    let mut settings = state.settings().await;
    change(&mut settings);
    state
        .update_settings(settings)
        .await
        .map_err(|e| CommandError::Message(format!("could not save settings: {e}")))
}

/// Remember the server the user chose.
#[tauri::command]
pub async fn set_server_url(state: State<'_, AppState>, url: String) -> CommandResult<String> {
    let normalized = auth_flow::normalize_url(&url)
        .ok_or_else(|| CommandError::Message("That does not look like a valid address.".into()))?;

    let switching = state.settings().await.server_url.as_deref() != Some(normalized.as_str());

    mutate_settings(&state, |s| {
        // Switching servers invalidates the old session.
        if switching {
            s.clear_session();
        }
        s.server_url = Some(normalized.clone());
    })
    .await?;

    // A different server has a different library, so the mirrored one is not merely
    // stale, it is wrong.
    if switching {
        state.forget_catalog().await;
    }
    Ok(normalized)
}

/// Open the sign-in window.
///
/// `direct` demands the username/password form even when the server has SSO configured.
#[tauri::command]
pub async fn begin_login(
    app: AppHandle,
    state: State<'_, AppState>,
    direct: Option<bool>,
) -> CommandResult<()> {
    let settings = state.settings().await;
    let url = settings
        .server_url
        .ok_or_else(|| CommandError::Message("No server configured yet.".into()))?;
    auth_flow::open_login_window(&app, &url, direct.unwrap_or(false)).map_err(CommandError::Message)
}

/// What a poll of the sign-in window found.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginPoll {
    /// The session works; the wizard can move on.
    pub signed_in: bool,
    /// False once the user has closed the sign-in window, so the wizard can stop waiting
    /// instead of spinning forever.
    pub window_open: bool,
    /// Set when cookies exist but do not authenticate, usually a proxy in front of the
    /// server that has not been satisfied yet.
    pub detail: Option<String>,
}

/// Check whether the sign-in window has produced a usable session.
///
/// Reports success only once the harvested cookies actually authenticate against the
/// server, not merely once cookies exist, since an identity provider sets plenty of its
/// own along the way, and a forward-auth proxy sets more still.
#[tauri::command]
pub async fn poll_login(app: AppHandle, state: State<'_, AppState>) -> CommandResult<LoginPoll> {
    let window_open = auth_flow::login_window_open(&app);
    let settings = state.settings().await;

    let Some(url) = settings.server_url.clone() else {
        return Ok(LoginPoll {
            signed_in: false,
            window_open,
            detail: Some("No server configured.".into()),
        });
    };

    let cookies = auth_flow::harvest_cookies(&app, &url);
    if cookies.is_empty() {
        return Ok(LoginPoll {
            signed_in: false,
            window_open,
            detail: None,
        });
    }

    state.connect_with_cookies(&url, cookies.clone()).await?;
    let user = match state.client().await {
        Some(client) => client.user_info().await.ok().flatten(),
        None => None,
    };

    let Some(user) = user else {
        // Deliberately does nothing but report. Navigating the window here, which an
        // earlier version did, sends it to `/`, which on an SSO server starts a second
        // authorization request and makes the user's real callback fail with
        // `/login?error`. Waiting is correct; the window completes the flow itself.
        return Ok(LoginPoll {
            signed_in: false,
            window_open,
            detail: Some(format!(
                "Waiting for the server to accept the session ({} cookies so far).",
                cookies.len()
            )),
        });
    };

    let mut settings = settings;
    settings.cookies = cookies;
    settings.username = Some(user.username);
    state
        .update_settings(settings)
        .await
        .map_err(|e| CommandError::Message(format!("could not save the session: {e}")))?;

    auth_flow::close_login_window(&app);
    Ok(LoginPoll {
        signed_in: true,
        window_open: false,
        detail: None,
    })
}

/// Clear the sign-in window's stored profile, for a login that has got stuck.
#[tauri::command]
pub async fn reset_login(app: AppHandle) -> CommandResult<()> {
    auth_flow::reset_login_profile(&app)
        .await
        .map_err(CommandError::Message)
}

#[tauri::command]
pub async fn cancel_login(app: AppHandle) -> CommandResult<()> {
    auth_flow::close_login_window(&app);
    Ok(())
}

/// Forget the stored session, keeping the server address.
#[tauri::command]
pub async fn sign_out(state: State<'_, AppState>) -> CommandResult<()> {
    state.disconnect().await;
    // The cached catalogue belongs to the account that was signed in, not to the machine.
    state.forget_catalog().await;
    mutate_settings(&state, Settings::clear_session).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CommandResult<Settings> {
    let mut settings = state.settings().await;
    // Never hand the cookie jar to the webview; it has no use for it.
    settings.cookies.clear();
    Ok(settings)
}

/// Where games should go, unless the user picks somewhere else.
#[tauri::command]
pub async fn suggest_library_root(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    if let Some(existing) = state.settings().await.library_root {
        return Ok(existing);
    }
    let home = app
        .path()
        .home_dir()
        .map_err(|e| CommandError::Message(format!("could not find your home directory: {e}")))?;
    Ok(Settings::default_library_root(&home)
        .to_string_lossy()
        .into_owned())
}

/// Change how much detail is written to the log.
#[tauri::command]
pub async fn set_log_level(state: State<'_, AppState>, level: String) -> CommandResult<()> {
    let parsed = crate::settings::LogLevel::from_key(&level)
        .ok_or_else(|| CommandError::Message(format!("{level} is not a log level.")))?;

    crate::set_log_level(parsed).map_err(CommandError::Message)?;

    mutate_settings(&state, |s| s.log_level = parsed).await
}

/// Change the download speed cap, in KiB/s. Zero means unlimited.
#[tauri::command]
pub async fn set_download_limit(state: State<'_, AppState>, kib: u32) -> CommandResult<()> {
    mutate_settings(&state, |s| s.download_limit_kib = kib).await?;
    // Applied to the shared handle as well as stored, so transfers already running pick
    // it up on their next chunk rather than at the next download.
    state.download_limit().set(u64::from(kib) * 1024);
    tracing::info!(kib, "download limit changed");
    Ok(())
}

/// Stop a download that is in progress.
///
/// The partial file and its checkpoint are kept. Against a server that supports `Range`
/// the next attempt resumes from there; Gameyfin 2.4 does not, so today it restarts, and
/// cancelling a nearly-finished download means downloading it again.
#[tauri::command]
pub async fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    if state.cancel_download(game_id).await {
        tracing::info!(game_id, "download cancellation requested");
    } else {
        // Not an error: the transfer may have finished between the click and this call.
        tracing::debug!(game_id, "nothing to cancel");
    }
    let _ = app.emit("library-changed", ());
    Ok(())
}

/// Where the app keeps its own files, so Settings can show and open it.
#[tauri::command]
pub async fn config_directory(state: State<'_, AppState>) -> CommandResult<String> {
    Ok(state.config_dir().await.to_string_lossy().into_owned())
}

/// Where compatibility prefixes live, and how much space they occupy.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefixInfo {
    pub path: String,
    pub count: usize,
    pub bytes: u64,
}

#[tauri::command]
pub async fn prefix_info(state: State<'_, AppState>) -> CommandResult<Option<PrefixInfo>> {
    if cfg!(windows) {
        // Nothing on Windows runs through a compatibility layer.
        return Ok(None);
    }

    let Some(root) = state.settings().await.library_root else {
        return Ok(None);
    };
    let dir = gameyfin_core::InstallLayout::new(&root)
        .prefix_dir(0)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    let probe = dir.clone();
    let (count, bytes) = tokio::task::spawn_blocking(move || {
        let Ok(entries) = std::fs::read_dir(&probe) else {
            return (0usize, 0u64);
        };
        let dirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        let bytes = dirs
            .iter()
            .filter_map(|e| gameyfin_core::extract::directory_size(&e.path()).ok())
            .sum();
        (dirs.len(), bytes)
    })
    .await
    .unwrap_or((0, 0));

    Ok(Some(PrefixInfo {
        path: dir.to_string_lossy().into_owned(),
        count,
        bytes,
    }))
}

/// Delete every compatibility prefix.
///
/// They are rebuilt on demand, so this is a safe way to recover from one that has got
/// into a bad state, at the cost of any per-game Wine configuration.
#[tauri::command]
pub async fn clear_prefixes(state: State<'_, AppState>) -> CommandResult<()> {
    let Some(root) = state.settings().await.library_root else {
        return Ok(());
    };
    let Some(dir) = gameyfin_core::InstallLayout::new(&root)
        .prefix_dir(0)
        .parent()
        .map(Path::to_path_buf)
    else {
        return Ok(());
    };

    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(fs_err("remove", &dir))?;
        tracing::info!(?dir, "cleared compatibility prefixes");
    }
    Ok(())
}

/// Change the address-space cap applied to installers.
#[tauri::command]
pub async fn set_installer_memory_limit(
    state: State<'_, AppState>,
    megabytes: u32,
) -> CommandResult<()> {
    mutate_settings(&state, |s| s.installer_memory_limit_mb = megabytes).await?;
    tracing::info!(megabytes, "installer memory cap changed");
    Ok(())
}

/// Progress of a Wine download, for the settings screen.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WineProgressEvent {
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: f64,
}

/// What Wine is installed, and what is available.
///
/// The two halves are independent on purpose: the release lookup needs the network and
/// the installed build does not, so a machine that is offline still reports its own Wine
/// correctly instead of failing the whole call.
#[tauri::command]
pub async fn wine_status(
    state: State<'_, AppState>,
) -> CommandResult<gameyfin_core::wine::WineStatus> {
    let config_dir = state.config_dir().await;
    let variant = state.settings().await.wine_variant;
    let installed = gameyfin_core::wine::installed(&config_dir);

    let latest = match gameyfin_core::wine::latest_release(&state.http().await, variant).await {
        Ok(release) => Some(release),
        Err(e) => {
            // Not an error for the caller: an unreachable feed must not stop the settings
            // screen from showing what is already installed.
            tracing::warn!(error = %e, "could not check for a Wine update");
            None
        }
    };

    Ok(gameyfin_core::wine::WineStatus { installed, latest })
}

/// Download and install the current Wine build, replacing any existing one.
///
/// Also used for "update" and "redownload": all three are the same operation, and having
/// one path means a repair cannot behave differently from a first install.
#[tauri::command]
pub async fn install_wine(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<gameyfin_core::wine::InstalledWine> {
    let config_dir = state.config_dir().await;
    let variant = state.settings().await.wine_variant;
    let http = state.http().await;

    let release = gameyfin_core::wine::latest_release(&http, variant)
        .await
        .map_err(|e| {
            CommandError::Message(format!("could not find a Wine build to download: {e}"))
        })?;

    tracing::info!(
        version = %release.version,
        variant = variant.as_str(),
        size_bytes = release.size_bytes,
        verified = release.sha256.is_some(),
        "downloading Wine"
    );

    // The transfer pool, not the general one: that has a whole-request timeout which a
    // 100 MB download would trip, surfacing as a decode error rather than a timeout.
    let downloader = gameyfin_core::Downloader::new(state.transfer_http().await);

    // Progress crosses threads, so it is pumped through a channel rather than emitted
    // from inside the download callback.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let pump = {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            while let Some((received, total, rate)) = rx.recv().await {
                if last.elapsed() >= std::time::Duration::from_millis(200) {
                    last = std::time::Instant::now();
                    let _ = app.emit(
                        "wine-progress",
                        WineProgressEvent {
                            received_bytes: received,
                            total_bytes: total,
                            bytes_per_second: rate,
                        },
                    );
                }
            }
        })
    };

    let result = gameyfin_core::wine::install(&config_dir, &release, &downloader, |p| {
        let _ = tx.send((
            p.received_bytes,
            p.total_bytes.unwrap_or(0),
            p.bytes_per_second,
        ));
    })
    .await;

    drop(tx);
    let _ = pump.await;

    match result {
        Ok(installed) => {
            tracing::info!(version = %installed.version, "Wine installed");
            let _ = app.emit("wine-changed", ());
            Ok(installed)
        }
        Err(e) => {
            tracing::error!(error = %e, "could not install Wine");
            Err(CommandError::Message(format!(
                "Could not install Wine: {e}"
            )))
        }
    }
}

/// Delete the downloaded Wine.
///
/// Game prefixes are left alone: they are built by Wine but owned by the games, and
/// throwing them away because the runtime was reinstalled would lose save data living
/// inside them.
#[tauri::command]
pub async fn remove_wine(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    gameyfin_core::wine::remove(&state.config_dir().await)
        .await
        .map_err(|e| CommandError::Message(format!("could not remove Wine: {e}")))?;
    tracing::info!("downloaded Wine removed");
    let _ = app.emit("wine-changed", ());
    Ok(())
}

/// Choose which Wine build to download.
#[tauri::command]
pub async fn set_wine_variant(state: State<'_, AppState>, variant: String) -> CommandResult<()> {
    let parsed = gameyfin_core::wine::WineVariant::parse(&variant);
    mutate_settings(&state, |s| s.wine_variant = parsed).await?;
    tracing::info!(variant = parsed.as_str(), "Wine build changed");
    Ok(())
}

/// How much disk the artwork cache is using.
#[tauri::command]
pub async fn image_cache_size(state: State<'_, AppState>) -> CommandResult<u64> {
    let cache = state.image_cache().await;
    Ok(tokio::task::spawn_blocking(move || cache.size())
        .await
        .unwrap_or(0))
}

/// Empty the artwork cache.
#[tauri::command]
pub async fn clear_image_cache(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .image_cache()
        .await
        .clear()
        .await
        .map_err(|e| CommandError::Message(format!("could not clear the image cache: {e}")))?;
    tracing::info!("image cache cleared");
    Ok(())
}

/// Where log files are written, so Settings can show it.
#[tauri::command]
pub async fn log_directory() -> CommandResult<String> {
    Ok(crate::log_directory().to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn set_library_root(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<()> {
    mutate_settings(&state, |s| s.library_root = Some(path.clone())).await?;

    // Adopt whatever is already in the new folder, so a reinstall or a moved library is
    // recognised rather than appearing empty.
    let found = state.library().rescan(Path::new(&path)).await;
    tracing::info!("rescanned {path}: {found} entries");
    notify(&app);
    Ok(())
}

#[tauri::command]
pub async fn list_libraries(state: State<'_, AppState>) -> CommandResult<Vec<Library>> {
    if state.client().await.is_none() {
        return Err(CommandError::NotConnected);
    }
    Ok(state.libraries().await?)
}

#[tauri::command]
pub async fn list_entries(state: State<'_, AppState>) -> CommandResult<Vec<LibraryEntry>> {
    // Checked explicitly so a signed-out client reports `not-connected`, which the UI
    // uses to send the user back to the wizard, rather than a generic failure. A cached
    // catalogue is enough to render without a client, so it is not sent back either.
    if state.client().await.is_none() && !state.has_cached_catalog().await {
        return Err(CommandError::NotConnected);
    }
    let games = state.games().await?;
    let library = state.library();

    let mut entries = Vec::with_capacity(games.len());
    for game in games {
        let record = library.record(game.id).await;
        entries.push(LibraryEntry {
            cover_url: game
                .cover
                .as_ref()
                .map(|i| crate::images::url_for(&i.path())),
            header_url: game
                .header
                .as_ref()
                .map(|i| crate::images::url_for(&i.path())),
            screenshot_urls: game
                .images
                .iter()
                .map(|i| crate::images::url_for(&i.path()))
                .collect(),
            minutes_played: record.minutes_played,
            last_played_at: record.last_played_at.clone(),
            archive_present: record.archive_path.as_ref().is_some_and(|p| p.exists()),
            state: library.state_from(game.id, record).await,
            game,
        });
    }
    Ok(entries)
}

/// Begin downloading a game.
///
/// Returns as soon as the transfer is under way; progress arrives as `library-changed`
/// events rather than being polled.
#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    if state.library().is_busy(game_id).await {
        // Two transfers would write to the same file.
        return Ok(());
    }

    let client = state.client().await.ok_or(CommandError::NotConnected)?;
    let settings = state.settings().await;

    let library_root = library_root(&state).await?;

    let game = state
        .games()
        .await?
        .into_iter()
        .find(|g| g.id == game_id)
        .ok_or_else(|| CommandError::Message("That game is no longer in the library.".into()))?;

    let providers = client.download_providers().await.map_err(|e| {
        if e.is_unreachable() {
            state.set_reachable(false);
            CommandError::Message(
                "Cannot reach the server, so there is nothing to download from. \
                 Your installed games still work."
                    .into(),
            )
        } else {
            CommandError::Api(e)
        }
    })?;
    let provider = providers.into_iter().next().ok_or_else(|| {
        CommandError::Message("The server has no download provider enabled.".into())
    })?;

    let url = client.download_url(&game, &provider.key);
    let filename = crate::downloads::provisional_filename(&game.title);
    let destination =
        crate::downloads::download_path(&library_root, game.id, &game.title, &filename);

    // `/download/**` is session-protected unless the server allows public access.
    let cookie_header = cookie_header(&settings.cookies);

    let library = state.library_handle();
    // Deliberately not the shared client: its total-request timeout would abort the
    // transfer part-way through a large game.
    let http = state.transfer_http().await;
    // The stored value seeds the shared handle, which is what the transfer actually reads
    // on every chunk, so a change made mid-download takes effect immediately.
    let rate_limit = state.download_limit();
    rate_limit.set(u64::from(settings.download_limit_kib) * 1024);
    if settings.download_limit_kib > 0 {
        tracing::info!(
            game_id,
            kib_per_second = settings.download_limit_kib,
            "limiting download speed"
        );
    }
    let cancel = state.register_download(game_id).await;
    let cancels = state.cancel_handle();

    tauri::async_runtime::spawn(async move {
        library
            .set_activity(
                game_id,
                Activity::Downloading {
                    received_bytes: 0,
                    total_bytes: 0,
                    bytes_per_second: 0.0,
                },
            )
            .await;
        notify(&app);

        let downloader = gameyfin_core::Downloader::new(http)
            .with_shared_rate_limit(rate_limit)
            .with_cancel(cancel);

        // Progress arrives per chunk. Both the shared state and the UI have to be kept
        // current, updating only the event left the library showing 0 B forever, because
        // that is what a refetch read back.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let pump = {
            let library = library.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut last_notify = std::time::Instant::now();
                while let Some((received, total, rate)) = rx.recv().await {
                    library
                        .set_activity(
                            game_id,
                            Activity::Downloading {
                                received_bytes: received,
                                total_bytes: total,
                                bytes_per_second: rate,
                            },
                        )
                        .await;
                    // Throttle only the UI notification; the state itself stays exact.
                    if last_notify.elapsed() >= std::time::Duration::from_millis(300) {
                        last_notify = std::time::Instant::now();
                        notify(&app);
                    }
                }
            })
        };

        let result = downloader
            .download(
                &url,
                &destination,
                |req| {
                    if cookie_header.is_empty() {
                        req
                    } else {
                        req.header(reqwest::header::COOKIE, cookie_header.clone())
                    }
                },
                |progress| {
                    let _ = tx.send((
                        progress.received_bytes,
                        progress.total_bytes.unwrap_or(0),
                        progress.bytes_per_second,
                    ));
                },
            )
            .await;

        drop(tx);
        let _ = pump.await;

        match result {
            Ok(outcome) => {
                tracing::info!(
                    "downloaded game {game_id} to {:?} ({} bytes)",
                    outcome.path,
                    outcome.bytes
                );
                library
                    .update_record(game_id, |r| {
                        r.archive_path = Some(outcome.path.clone());
                        r.archive_bytes = outcome.bytes;
                    })
                    .await;
                // A finished download is not an install: it rests in Downloaded until the
                // user chooses to install it.
                library.clear_activity(game_id).await;
            }
            // Cancelling is something the user asked for, so it returns the game to its
            // previous state rather than flagging a failure they would have to dismiss.
            Err(gameyfin_core::CoreError::Cancelled) => {
                tracing::info!(game_id, "download cancelled");
                library.clear_activity(game_id).await;
            }
            Err(e) => {
                tracing::error!("download of game {game_id} failed: {e}");
                library
                    .set_activity(
                        game_id,
                        Activity::Failed {
                            message: e.to_string(),
                            stage: Stage::Download,
                        },
                    )
                    .await;
            }
        }

        cancels.finish(game_id).await;
        notify(&app);
    });

    Ok(())
}

/// Explain a game that exited immediately, or `None` when it merely ended quickly.
///
/// A clean exit is not a failure however short it was, a launcher that hands off to
/// another process legitimately does this, so only a non-zero code is reported.
fn failed_to_start(session: &gameyfin_core::Session) -> Option<String> {
    let code = match session.end {
        gameyfin_core::SessionEnd::Exited { code: Some(code) } if code != 0 => code,
        _ => return None,
    };

    // Wine writes pages of `fixme:` lines on a normal run, so only the end is worth
    // showing: whatever actually went wrong is the last thing it said.
    let tail: Vec<&str> = session
        .error_output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(12)
        .collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");

    Some(if tail.is_empty() {
        format!("The game exited immediately with code {code} and wrote nothing that explains why.")
    } else {
        format!("The game exited immediately with code {code}:\n{tail}")
    })
}

/// One way of installing a download.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOption {
    pub key: String,
    pub label: String,
    pub description: String,
    /// Hands control to a third-party setup program, so the user picks the location.
    pub interactive: bool,
    /// Present when the option cannot be used yet, explaining what is missing.
    pub blocked_by: Option<String>,
}

/// What can be done with a download or its unpacked files.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    /// What the downloaded file turned out to be.
    pub payload: String,
    pub options: Vec<InstallOption>,
    /// Where a "move into place" install would put the game.
    pub default_install_dir: String,
    /// The path to type into a setup wizard, when one will ask.
    ///
    /// `None` when nothing will ask, moving files into place needs no path, and `None`
    /// on Windows, where `default_install_dir` is already a usable Windows path.
    pub windows_install_path: Option<String>,
    /// Whether any offered option hands over to a setup wizard.
    pub needs_install_path: bool,
    /// Setup programs found in the unpacked files, relative to the staging directory.
    pub setup_candidates: Vec<String>,
    /// Where a "choose it yourself" file picker should open: the unpacked files if they
    /// exist, otherwise the download folder. Opening at the installations root would make
    /// the user navigate back to where they already are.
    pub browse_dir: Option<String>,
}

/// Inspect a download and report what can be done with it next.
#[tauri::command]
pub async fn install_options(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<InstallPlan> {
    let record = state.library().record(game_id).await;
    let install_dir = install_dir_for(&state, game_id).await?;
    let windows_host = cfg!(windows);
    let config_dir = state.config_dir().await;
    let runtime = if windows_host {
        None
    } else {
        gameyfin_core::detect_windows_runtime_in(Some(&config_dir))
    };

    // Already unpacked: the choice is now what to do with the files.
    if record.is_extracted() {
        let mut options = vec![InstallOption {
            key: "move".into(),
            label: "Move into your games folder".into(),
            description: "For a game that runs straight from its files, with no setup step.".into(),
            interactive: false,
            blocked_by: None,
        }];

        for setup in &record.setup_candidates {
            options.insert(
                0,
                InstallOption {
                    key: format!("setup:{setup}"),
                    label: format!("Run {setup}"),
                    description: "Start the game's own setup program. Paste the install path \
                                  when it asks."
                        .into(),
                    interactive: true,
                    blocked_by: if windows_host || runtime.is_some() {
                        None
                    } else {
                        Some(gameyfin_core::windows_runtime_hint())
                    },
                },
            );
        }

        // Only meaningful when a setup program will ask where to install. Moving files
        // into place needs no path from the user at all.
        let needs_path = options.iter().any(|o| o.interactive);
        return Ok(InstallPlan {
            payload: "unpacked files".into(),
            windows_install_path: needs_path
                .then(|| windows_install_path(&install_dir))
                .flatten(),
            needs_install_path: needs_path,
            options,
            default_install_dir: install_dir.to_string_lossy().into_owned(),
            browse_dir: browse_dir_for(&record),
            setup_candidates: record.setup_candidates,
        });
    }

    // Cloned rather than moved: the record is still needed below for the browse folder.
    let archive = record
        .existing_archive()
        .ok_or_else(|| CommandError::Message("This game has not been downloaded yet.".into()))?;

    let payload = tokio::task::spawn_blocking({
        let archive = archive.clone();
        move || gameyfin_core::classify(&archive)
    })
    .await
    .map_err(|e| CommandError::Message(format!("could not inspect the download: {e}")))?
    .map_err(|e| CommandError::Message(format!("could not inspect the download: {e}")))?;

    let options = if payload.is_archive() {
        vec![InstallOption {
            key: "extract".into(),
            label: "Extract".into(),
            description: "Unpack the archive so its contents can be inspected and installed."
                .into(),
            interactive: false,
            blocked_by: None,
        }]
    } else {
        gameyfin_core::methods_for(payload, windows_host)
            .into_iter()
            .map(|method| {
                let blocked_by = match method {
                    gameyfin_core::InstallMethod::RunWindowsInstallerViaProton
                        if runtime.is_none() =>
                    {
                        Some(gameyfin_core::windows_runtime_hint())
                    }
                    _ => None,
                };
                InstallOption {
                    key: method.key().to_string(),
                    label: method_label(method, runtime.as_ref()).to_string(),
                    description: method_description(method).to_string(),
                    interactive: method.is_interactive(),
                    blocked_by,
                }
            })
            .collect()
    };

    let needs_path = options.iter().any(|o| o.interactive);
    Ok(InstallPlan {
        payload: payload.label().to_string(),
        windows_install_path: needs_path
            .then(|| windows_install_path(&install_dir))
            .flatten(),
        needs_install_path: needs_path,
        options,
        default_install_dir: install_dir.to_string_lossy().into_owned(),
        browse_dir: browse_dir_for(&record),
        setup_candidates: Vec::new(),
    })
}

fn method_label(
    method: gameyfin_core::InstallMethod,
    runtime: Option<&gameyfin_core::WindowsRuntime>,
) -> &'static str {
    use gameyfin_core::InstallMethod::*;
    match method {
        Extract => "Extract",
        RunWindowsInstaller => "Run the installer",
        RunWindowsInstallerViaProton => match runtime.map(|r| r.kind()) {
            Some("wine") => "Run the installer with Wine",
            _ => "Run the installer with Proton",
        },
        CopyExecutable => "Move into your games folder",
    }
}

fn method_description(method: gameyfin_core::InstallMethod) -> &'static str {
    use gameyfin_core::InstallMethod::*;
    match method {
        Extract => "Unpack the archive so its contents can be inspected and installed.",
        RunWindowsInstaller => {
            "Start the game's own setup program. Paste the install path when it asks."
        }
        RunWindowsInstallerViaProton => {
            "Start the game's own setup program in a Windows compatibility layer. \
             Paste the install path when it asks."
        }
        CopyExecutable => "Put the program in your games folder and make it runnable.",
    }
}

/// Where a file picker should start for this game.
fn browse_dir_for(record: &crate::library_state::GameRecord) -> Option<String> {
    record
        .extracted_dir
        .clone()
        .filter(|d| d.exists())
        .or_else(|| {
            record
                .archive_path
                .as_ref()
                .and_then(|a| a.parent().map(Path::to_path_buf))
        })
        .map(|d| d.to_string_lossy().into_owned())
}

/// A filesystem error naming what was attempted and where.
fn fs_err(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> CommandError {
    let path = path.to_path_buf();
    move |e| CommandError::Message(format!("could not {action} {path:?}: {e}"))
}

/// Whether a directory holds nothing, treating an unreadable directory as non-empty so
/// nothing is deleted on the strength of a failed read.
async fn dir_is_empty(dir: &Path) -> bool {
    match tokio::fs::read_dir(dir).await {
        Ok(mut entries) => entries.next_entry().await.ok().flatten().is_none(),
        Err(_) => false,
    }
}

/// The configured games folder, or the message telling the user to pick one.
async fn library_root(state: &State<'_, AppState>) -> CommandResult<String> {
    state.settings().await.library_root.ok_or_else(|| {
        CommandError::Message("No games folder configured yet. Set one in Settings.".into())
    })
}

/// Where a game's compatibility prefix lives.
async fn prefix_dir_for(state: &State<'_, AppState>, game_id: i64) -> CommandResult<PathBuf> {
    let root = library_root(state).await?;
    Ok(gameyfin_core::InstallLayout::new(&root).prefix_dir(game_id))
}

/// Where a game is installed, given the configured library root.
async fn install_dir_for(state: &State<'_, AppState>, game_id: i64) -> CommandResult<PathBuf> {
    let root = library_root(state).await?;

    let title = state
        .games()
        .await?
        .into_iter()
        .find(|g| g.id == game_id)
        .map(|g| g.title)
        .unwrap_or_else(|| format!("Game {game_id}"));

    Ok(gameyfin_core::InstallLayout::new(&root).install_dir(game_id, &title))
}

/// Unpack a download, staging the files beside the archive.
///
/// Extraction is not installation. The unpacked files stay in the Downloads folder until
/// the user decides what should happen to them, run a setup program, or move them into
/// the games folder, because an archive may contain either.
#[tauri::command]
pub async fn extract_download(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    delete_archive: Option<bool>,
) -> CommandResult<()> {
    if state.library().is_busy(game_id).await {
        return Ok(());
    }

    let record = state.library().record(game_id).await;
    let archive = record
        .existing_archive()
        .ok_or_else(|| CommandError::Message("This game has not been downloaded yet.".into()))?;

    let staging = archive
        .parent()
        .ok_or_else(|| CommandError::Message("The download has no folder.".into()))?
        .join(crate::library_state::EXTRACT_DIR);

    let library = state.library_handle();
    let delete_archive = delete_archive.unwrap_or(false);

    tracing::info!(game_id, ?staging, delete_archive, "extracting download");

    tauri::async_runtime::spawn(async move {
        library
            .set_activity(game_id, Activity::Extracting { percent: 0.0 })
            .await;
        notify_state(&app, &library, game_id).await;

        let dir = staging.clone();
        let progress_library = library.clone();
        let progress_app = app.clone();
        let archive_for_extract = archive.clone();

        // Extraction is synchronous and CPU-bound, so it runs on the blocking pool rather
        // than stalling the async runtime for the duration of a large archive.
        let outcome = tokio::task::spawn_blocking(move || {
            // Starts in the past so the first update is emitted immediately rather than
            // being swallowed by the throttle.
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            gameyfin_core::extract::extract(&archive_for_extract, &dir, |progress| {
                if last.elapsed() < std::time::Duration::from_millis(200) {
                    return;
                }
                last = std::time::Instant::now();
                let percent = progress.percent();
                let library = progress_library.clone();
                let app = progress_app.clone();
                tauri::async_runtime::spawn(async move {
                    library
                        .set_activity(game_id, Activity::Extracting { percent })
                        .await;
                    notify_state(&app, &library, game_id).await;
                });
            })
        })
        .await;

        match outcome {
            Ok(Ok(bytes)) => {
                tracing::info!(game_id, bytes, ?staging, "extraction finished");

                let scan_dir = staging.clone();
                let setups = tokio::task::spawn_blocking(move || {
                    gameyfin_core::executable::find_installers(&scan_dir).unwrap_or_default()
                })
                .await
                .unwrap_or_default();

                let relative: Vec<String> = setups
                    .iter()
                    .filter_map(|p| {
                        p.strip_prefix(&staging)
                            .ok()
                            .map(|r| r.to_string_lossy().into_owned())
                    })
                    .collect();

                if !relative.is_empty() {
                    tracing::info!(
                        game_id,
                        candidates = ?relative,
                        "unpacked files contain a setup program"
                    );
                }

                if delete_archive {
                    match tokio::fs::remove_file(&archive).await {
                        Ok(()) => tracing::info!(game_id, "removed the archive after extracting"),
                        Err(e) => {
                            tracing::warn!(game_id, error = %e, "could not remove the archive")
                        }
                    }
                }

                library
                    .update_record(game_id, |r| {
                        r.extracted_dir = Some(staging.clone());
                        r.staging_setups = relative.clone();
                        r.setup_candidates = relative;
                        if delete_archive {
                            r.archive_path = None;
                            r.archive_bytes = 0;
                        }
                    })
                    .await;
                library.clear_activity(game_id).await;
            }
            Ok(Err(e)) => {
                tracing::error!(game_id, error = %e, "extraction failed");
                library
                    .set_activity(
                        game_id,
                        Activity::Failed {
                            message: e.to_string(),
                            stage: Stage::Extract,
                        },
                    )
                    .await;
            }
            Err(e) => {
                tracing::error!(game_id, error = %e, "extraction task stopped unexpectedly");
                library
                    .set_activity(
                        game_id,
                        Activity::Failed {
                            message: "Extraction stopped unexpectedly.".into(),
                            stage: Stage::Extract,
                        },
                    )
                    .await;
            }
        }

        notify_state(&app, &library, game_id).await;
    });

    Ok(())
}

/// Install a game, by whichever route suits what is on disk.
///
/// `method` is `extract`, `move`, `setup:<relative path>`, or one of the payload methods
/// for a download that is not an archive.
#[tauri::command]
pub async fn install_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    method: Option<String>,
    delete_archive: Option<bool>,
) -> CommandResult<()> {
    if state.library().is_busy(game_id).await {
        return Ok(());
    }

    let method = method.unwrap_or_else(|| "extract".to_string());
    let install_dir = install_dir_for(&state, game_id).await?;
    let record = state.library().record(game_id).await;

    if method == "extract" {
        return extract_download(app, state, game_id, delete_archive).await;
    }

    if method == "move" {
        let source = record
            .existing_staging()
            .ok_or_else(|| CommandError::Message("Nothing has been unpacked yet.".into()))?;
        return move_into_place(&app, &state, game_id, &source, &install_dir).await;
    }

    if let Some(relative) = method.strip_prefix("setup:") {
        let base = record
            .existing_staging()
            .ok_or_else(|| CommandError::Message("Nothing has been unpacked yet.".into()))?;
        state
            .library()
            .update_record(game_id, |r| r.used_setup = Some(relative.to_string()))
            .await;
        return run_program_as_installer(&app, &state, game_id, &base.join(relative), &install_dir)
            .await;
    }

    // Not an archive: act on the downloaded file itself.
    let archive = record
        .existing_archive()
        .ok_or_else(|| CommandError::Message("This game has not been downloaded yet.".into()))?;

    match gameyfin_core::InstallMethod::from_key(&method) {
        Some(gameyfin_core::InstallMethod::CopyExecutable) => {
            copy_executable(&app, &state, game_id, &archive, &install_dir).await
        }
        Some(
            gameyfin_core::InstallMethod::RunWindowsInstaller
            | gameyfin_core::InstallMethod::RunWindowsInstallerViaProton,
        ) => run_program_as_installer(&app, &state, game_id, &archive, &install_dir).await,
        _ => Err(CommandError::Message(format!(
            "{method} is not a way to install this."
        ))),
    }
}

/// Move staged files into the games folder.
async fn move_into_place(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
    source: &Path,
    install_dir: &Path,
) -> CommandResult<()> {
    tracing::info!(
        game_id,
        ?source,
        ?install_dir,
        "moving unpacked files into place"
    );

    if let Some(parent) = install_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(fs_err("create", parent))?;
    }
    if install_dir.exists() {
        tokio::fs::remove_dir_all(install_dir)
            .await
            .map_err(fs_err("clear", install_dir))?;
    }

    // A rename is instant within a filesystem, but Downloads and Installations may sit on
    // different mounts, in which case it has to be a copy.
    let moved = tokio::fs::rename(source, install_dir).await.is_ok();
    if !moved {
        tracing::debug!(game_id, "rename failed; copying across filesystems instead");
        let (from, to) = (source.to_path_buf(), install_dir.to_path_buf());
        tokio::task::spawn_blocking(move || copy_tree(&from, &to))
            .await
            .map_err(|e| CommandError::Message(e.to_string()))?
            .map_err(|e| CommandError::Message(format!("could not copy the files: {e}")))?;
        let _ = tokio::fs::remove_dir_all(source).await;
    }

    // The download folder now holds nothing but, possibly, the archive. If even that is
    // gone, leaving an empty directory behind just accumulates clutter.
    if let Some(download_dir) = source.parent() {
        if dir_is_empty(download_dir).await {
            match tokio::fs::remove_dir(download_dir).await {
                Ok(()) => {
                    tracing::info!(game_id, ?download_dir, "removed the empty download folder")
                }
                Err(e) => {
                    tracing::debug!(game_id, error = %e, "could not remove the download folder")
                }
            }
        }
    }

    state
        .library()
        .update_record(game_id, |r| {
            r.extracted_dir = None;
            r.staging_setups.clear();
        })
        .await;
    finish_install(&state.library_handle(), game_id, install_dir).await;
    notify_state(app, state.library(), game_id).await;
    Ok(())
}

/// Recursive copy, for a move that crosses filesystems.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Remove an installed game's files, keeping the downloaded archive.
#[tauri::command]
pub async fn uninstall_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    run_uninstaller: Option<bool>,
) -> CommandResult<()> {
    let record = state.library().record(game_id).await;
    let Some(dir) = record.install_dir else {
        return Ok(());
    };

    // A game installed by a setup program usually ships its own uninstaller. Running it
    // clears registry entries and shortcuts that deleting the folder would leave behind.
    if run_uninstaller.unwrap_or(true) {
        if let Some(uninstaller) = gameyfin_core::find_uninstaller(&dir) {
            tracing::info!(game_id, ?uninstaller, "running the game's uninstaller");
            match run_uninstaller_program(&state, game_id, &uninstaller).await {
                Ok(()) => tracing::info!(game_id, "uninstaller finished"),
                Err(e) => tracing::warn!(game_id, "{e}; removing the files directly instead"),
            }
        }
    }

    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(fs_err("remove", &dir))?;
    }

    state
        .library()
        .update_record(game_id, |r| {
            r.install_dir = None;
            r.executable = None;
            r.installed_at = None;
            r.setup_candidates.clear();
        })
        .await;
    state.library().clear_activity(game_id).await;
    notify(&app);
    Ok(())
}

/// Run a game's own uninstaller and wait for it.
async fn run_uninstaller_program(
    state: &State<'_, AppState>,
    game_id: i64,
    uninstaller: &Path,
) -> Result<(), String> {
    let config = if gameyfin_core::needs_proton(uninstaller) {
        let config_dir = state.config_dir().await;
        let runtime = gameyfin_core::detect_windows_runtime_in(Some(&config_dir))
            .ok_or_else(|| "no Windows runtime available".to_string())?;
        let prefix = prefix_dir_for(state, game_id)
            .await
            .map_err(|e| e.to_string())?;
        gameyfin_core::LaunchConfig::for_windows_program_unattended(uninstaller, prefix, &runtime)
    } else {
        gameyfin_core::LaunchConfig::native(uninstaller)
    };

    let command = gameyfin_core::resolve_command(&config).map_err(|e| e.to_string())?;
    let run = gameyfin_core::run_capturing(&command)
        .await
        .map_err(|e| e.to_string())?;

    if run.success() {
        Ok(())
    } else {
        Err(format!(
            "the uninstaller exited with code {}",
            run.status.unwrap_or(-1)
        ))
    }
}

/// Delete the unpacked files left in Downloads after installing.
#[tauri::command]
pub async fn delete_staging(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    let record = state.library().record(game_id).await;
    if let Some(dir) = record.extracted_dir.filter(|d| d.exists()) {
        tracing::info!(game_id, ?dir, "removing unpacked files");
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(fs_err("remove", &dir))?;

        // The download folder may now be empty too.
        if let Some(parent) = dir.parent() {
            if dir_is_empty(parent).await {
                let _ = tokio::fs::remove_dir(parent).await;
            }
        }
    }

    state
        .library()
        .update_record(game_id, |r| {
            r.extracted_dir = None;
            r.staging_setups.clear();
        })
        .await;
    notify(&app);
    Ok(())
}

/// Delete a downloaded archive.
#[tauri::command]
pub async fn delete_download(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    let record = state.library().record(game_id).await;
    let mut removed = false;
    if let Some(archive) = record.archive_path {
        // Remove the containing folder, which also clears any resume checkpoint.
        let target = archive.parent().unwrap_or(&archive).to_path_buf();
        if target.exists() {
            match tokio::fs::remove_dir_all(&target).await {
                Ok(()) => {
                    tracing::info!(game_id, ?target, "deleted the download");
                    removed = true;
                }
                // Reported rather than swallowed: a delete that failed on permissions used
                // to look exactly like one that worked, with the row disappearing and the
                // files still on disk.
                Err(e) => {
                    tracing::error!(game_id, ?target, error = %e, "could not delete the download");
                    return Err(CommandError::Message(format!(
                        "Could not delete {}: {e}",
                        target.display()
                    )));
                }
            }
        }
    }

    if !removed {
        tracing::info!(
            game_id,
            "nothing on disk to delete; clearing the record only"
        );
    }

    state
        .library()
        .update_record(game_id, |r| {
            r.archive_path = None;
            r.archive_bytes = 0;
        })
        .await;
    // Also drop any failure left over from a previous attempt: it describes an archive
    // that no longer exists, and `state_of` prefers activity over what is on disk, so
    // leaving it would keep the row visible and unchanged.
    state.library().clear_activity(game_id).await;
    notify(&app);
    Ok(())
}

/// Run a setup program the user picked themselves.
///
/// Detection is heuristic and can miss an oddly named installer, so this takes an absolute
/// path rather than one of the detected candidates.
#[tauri::command]
pub async fn run_setup_path(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    path: String,
) -> CommandResult<()> {
    let program = PathBuf::from(&path);
    if !program.is_file() {
        return Err(CommandError::Message(format!("{path} is not a file.")));
    }

    let install_dir = install_dir_for(&state, game_id).await?;
    tracing::info!(game_id, %path, "running a user-chosen setup program");
    run_program_as_installer(&app, &state, game_id, &program, &install_dir).await
}

/// Reveal any path in the desktop file manager.
#[tauri::command]
pub async fn open_path(app: AppHandle, path: String) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_path(&path, None::<&str>)
        .map_err(|e| CommandError::Message(format!("could not open {path}: {e}")))
}

/// Reveal a game's folder in the desktop file manager.
#[tauri::command]
pub async fn open_game_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let record = state.library().record(game_id).await;
    let target = record
        .install_dir
        .or_else(|| {
            record
                .archive_path
                .and_then(|p| p.parent().map(Path::to_path_buf))
        })
        .ok_or_else(|| CommandError::Message("There is nothing on disk for this game.".into()))?;

    // The recorded folder can be gone: a failed install may never have created it, and a
    // finished one has its download folder removed. Opening a path that is not there is a
    // silent no-op through the desktop portal, the click appears to do nothing at all,
    // so fall back to the nearest folder that does exist and say what happened.
    let opened = if target.is_dir() {
        target.clone()
    } else {
        let parent = target
            .ancestors()
            .skip(1)
            .find(|p| p.is_dir())
            .ok_or_else(|| {
                CommandError::Message(format!(
                    "{} no longer exists, and neither does the folder that contained it.",
                    target.display()
                ))
            })?;
        tracing::warn!(game_id, ?target, fallback = ?parent, "the game's folder is gone; opening its parent");
        parent.to_path_buf()
    };

    // Logged because the failure mode is invisible otherwise: through the desktop portal,
    // opening a path that does not exist reports success and does nothing at all, so
    // without this there is no record that the click ever reached us.
    tracing::info!(game_id, path = ?opened, "opening a game folder");

    app.opener()
        .open_path(opened.to_string_lossy(), None::<&str>)
        .map_err(|e| CommandError::Message(format!("could not open the folder: {e}")))
}

/// Which of the library's own folders to reveal.
#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LibraryFolder {
    Downloads,
    Installations,
}

/// Reveal the Downloads or Installations folder in the desktop file manager.
///
/// Created if it is not there yet. Both folders are ours to make, and opening a path that
/// does not exist is a silent no-op through the desktop portal, which reads as the button
/// being broken.
#[tauri::command]
pub async fn open_library_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: LibraryFolder,
) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let root = library_root(&state).await?;
    let layout = gameyfin_core::InstallLayout::new(&root);
    let dir = match folder {
        LibraryFolder::Downloads => layout.downloads_root(),
        LibraryFolder::Installations => layout.installs_root(),
    };

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| CommandError::Message(format!("could not create {}: {e}", dir.display())))?;

    tracing::info!(path = ?dir, "opening a library folder");
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| CommandError::Message(format!("could not open the folder: {e}")))
}

/// Choose which executable launches a game.
#[tauri::command]
pub async fn set_game_executable(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    executable: String,
) -> CommandResult<()> {
    state
        .library()
        .update_record(game_id, |r| r.executable = Some(executable))
        .await;
    notify(&app);
    Ok(())
}

/// Launch candidates found under a game's install directory.
#[tauri::command]
pub async fn list_executables(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<Vec<String>> {
    let record = state.library().record(game_id).await;
    let Some(dir) = record.install_dir.filter(|d| d.exists()) else {
        return Ok(Vec::new());
    };

    let scan_dir = dir.clone();
    let detected =
        tokio::task::spawn_blocking(move || gameyfin_core::executable::detect(&scan_dir, ""))
            .await
            .map_err(|e| CommandError::Message(format!("could not scan for executables: {e}")))?
            .map_err(|e| CommandError::Message(format!("could not scan for executables: {e}")))?;

    let paths = match detected {
        gameyfin_core::Detection::Confident(path) => vec![path],
        gameyfin_core::Detection::Ambiguous(candidates) => {
            candidates.into_iter().map(|c| c.path).collect()
        }
        gameyfin_core::Detection::None => Vec::new(),
    };

    Ok(paths
        .into_iter()
        .filter_map(|p| {
            p.strip_prefix(&dir)
                .ok()
                .map(|rel| rel.to_string_lossy().into_owned())
        })
        .collect())
}

/// Put a native program in place and mark it runnable.
async fn copy_executable(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
    source: &Path,
    install_dir: &Path,
) -> CommandResult<()> {
    tokio::fs::create_dir_all(install_dir)
        .await
        .map_err(fs_err("create", install_dir))?;

    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "game".to_string());
    let target = install_dir.join(&name);

    tokio::fs::copy(source, &target)
        .await
        .map_err(|e| CommandError::Message(format!("could not copy the program: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A downloaded file is not executable, and the game will not start without this.
        let _ = tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).await;
    }

    tracing::info!(game_id, ?target, "installed a native program");
    state
        .library()
        .update_record(game_id, |r| {
            r.install_dir = Some(install_dir.to_path_buf());
            r.executable = Some(name);
            r.installed_at = Some(now_iso8601());
        })
        .await;
    state.library().clear_activity(game_id).await;
    notify_state(app, state.library(), game_id).await;
    Ok(())
}

/// Run a setup program and adopt whatever it installs.
///
/// The app cannot drive a setup wizard, so it cannot know where files will land. It runs
/// the program, then checks the suggested folder; if the user chose elsewhere they can
/// point the app at it afterwards.
async fn run_program_as_installer(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
    program: &Path,
    install_dir: &Path,
) -> CommandResult<()> {
    if !program.exists() {
        return Err(CommandError::Message(format!(
            "{} is no longer there.",
            program.display()
        )));
    }

    // Set when the installer will be run through a mapped drive rather than its path.
    let mut windows_program: Option<String> = None;

    // Checked before the prefix is built: a file that is not a program fails whatever
    // Wine does with it, and thirteen seconds of prefix preparation only delays the
    // answer. Wine reports `Bad format`, buried under unrelated startup chatter.
    if gameyfin_core::needs_proton(program) && !gameyfin_core::looks_like_windows_program(program) {
        tracing::error!(game_id, program = ?program, "not a Windows program");
        return Err(CommandError::Message(format!(
            "{} is not a Windows program. The download may be incomplete or corrupt.",
            program
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| program.display().to_string())
        )));
    }

    let mut config = if gameyfin_core::needs_proton(program) {
        // Order matters: the prefix is initialised before any drive is mapped. Wine
        // decides a prefix is already set up by looking for its own directories, so a
        // `dosdevices` created first makes `wineboot` skip building `drive_c`, after
        // which everything fails with "Cannot set the dir to C:\windows".
        let (runtime, prefix) = ready_windows_prefix(app, state, game_id).await?;

        // Map the *installations root* to a drive letter, not the game's own folder: a
        // setup wizard needs a path with a subdirectory, and pointing the drive at the
        // game folder would leave only the rejected `G:\` to offer.
        let installations_root = install_dir.parent().unwrap_or(install_dir);
        if let Err(e) = gameyfin_core::map_drive(&prefix, installations_root) {
            tracing::warn!(game_id, error = %e, "could not map the games drive");
        }

        // Map the installer's own folder too, and run it through that drive rather than
        // by its Linux path. Our download folders are named `(id) Title`, and an
        // installer that shells out to a batch script, which repack installers do
        // constantly, chokes on the unquoted parentheses.
        if let Some(source_dir) = program.parent() {
            match gameyfin_core::map_drive_letter(
                &prefix,
                gameyfin_core::prefix::SOURCE_DRIVE,
                source_dir,
                false,
            ) {
                Ok(_) => {
                    if let Some(name) = program.file_name() {
                        windows_program = Some(format!(
                            "{}:\\{}",
                            gameyfin_core::prefix::SOURCE_DRIVE,
                            name.to_string_lossy()
                        ));
                    }
                }
                Err(e) => tracing::warn!(game_id, error = %e, "could not map the installer drive"),
            }
        }

        tracing::info!(
            game_id,
            runtime = runtime.kind(),
            detail = runtime.description(),
            ?prefix,
            ?program,
            games_drive = %windows_install_path(install_dir).unwrap_or_default(),
            "running installer"
        );
        let launch_target = windows_program
            .clone()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| program.to_path_buf());
        let mut config = gameyfin_core::LaunchConfig::for_windows_program_unattended(
            launch_target,
            prefix,
            &runtime,
        );
        // Run from the installer's own folder so its relative data files resolve.
        config.working_dir = program.parent().map(std::path::Path::to_path_buf);
        config
    } else {
        tracing::info!(game_id, ?program, "running installer natively");
        gameyfin_core::LaunchConfig::native(program)
    };

    // Hand the destination to the installer where its toolkit accepts one, so the user
    // does not have to paste a path at all. The path is still offered for copying: an
    // unrecognised toolkit will ask regardless.
    let kind = gameyfin_core::identify(program).unwrap_or(gameyfin_core::InstallerKind::Unknown);
    // A destination the installer's helper scripts can handle: the app's own `(id) Title`
    // naming is legal on Windows but hostile to `cmd`. The folder is renamed to the
    // expected name once the installer has finished.
    let safe_folder = install_dir
        .file_name()
        .map(|n| gameyfin_core::windows_safe_name(&n.to_string_lossy()))
        .unwrap_or_else(|| "Game".to_string());
    let destination = if cfg!(windows) {
        install_dir.to_string_lossy().into_owned()
    } else {
        gameyfin_core::games_drive_path(&safe_folder)
    };
    let extra = kind.destination_args(&destination);
    if extra.is_empty() {
        tracing::info!(
            game_id,
            kind = kind.label(),
            "installer destination must be entered by hand"
        );
    } else {
        tracing::info!(game_id, kind = kind.label(), %destination, "passing the destination to the installer");
    }
    config.arguments.extend(extra);

    let mut command = gameyfin_core::resolve_command(&config)
        .map_err(|e| CommandError::Message(e.to_string()))?;
    tracing::debug!(game_id, program = ?command.program, args = ?command.args, "installer command");

    // The directory has to exist before the wizard is pointed at it.
    tokio::fs::create_dir_all(install_dir)
        .await
        .map_err(fs_err("create", install_dir))?;

    state
        .library()
        .set_activity(game_id, Activity::Installing { percent: 0.0 })
        .await;
    notify_state(app, state.library(), game_id).await;

    // Cap the installer's address space. Repack installers decompress through FreeArc's
    // `unarc.dll`, whose `LargestMemoryBlock` search overflows once a 32-bit process can
    // obtain a contiguous 2 GB block, after which it spins on one core forever, with the
    // progress bar frozen and nothing written to any log. A 64-bit host is exactly where
    // that block is available.
    //
    // How the cap reaches the installer depends on who spawns it: we set it directly on a
    // Wine we run ourselves, and hand it to the host inside the command when the Flatpak
    // reaches Wine through `flatpak-spawn`. Same limit, same process, either way.
    let limit_mb = state.settings().await.installer_memory_limit_mb;
    let address_space = if limit_mb > 0 && !cfg!(windows) {
        let cap = command.cap_address_space(u64::from(limit_mb) * 1024 * 1024);
        tracing::info!(
            game_id,
            limit_mb,
            via = cap.label(),
            "capping the installer's address space"
        );
        cap.before_exec()
    } else {
        None
    };

    let library = state.library_handle();
    let install_dir = install_dir.to_path_buf();
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        // Captured rather than supervised: when a setup program fails immediately, the
        // reason is in its output, and discarding that leaves nothing to diagnose.
        let run = gameyfin_core::run_capturing_limited(&command, address_space).await;
        let elapsed = started.elapsed();

        let failure = match &run {
            Ok(run) if run.success() => {
                tracing::info!(game_id, ?elapsed, "installer exited cleanly");
                None
            }
            Ok(run) => {
                // The full output goes to the log at debug level; the message the user
                // sees is the filtered tail, because Wine's chatter would otherwise be
                // all that fits.
                tracing::debug!(game_id, stderr = %run.stderr, stdout = %run.stdout, "installer output");
                let tail = run.diagnostic_tail(12);
                tracing::error!(
                    game_id,
                    status = ?run.status,
                    ?elapsed,
                    output = %tail,
                    "installer exited with an error"
                );
                Some(if tail.trim().is_empty() {
                    format!(
                        "The installer exited with code {} and no output.",
                        run.status.unwrap_or(-1)
                    )
                } else {
                    format!(
                        "The installer exited with code {}:\n{tail}",
                        run.status.unwrap_or(-1)
                    )
                })
            }
            Err(e) => {
                tracing::error!(game_id, error = %e, "could not run the installer");
                Some(e.to_string())
            }
        };

        if let Some(message) = failure {
            library
                .set_activity(
                    game_id,
                    Activity::Failed {
                        message,
                        stage: Stage::Install,
                    },
                )
                .await;
            notify_state(&app, &library, game_id).await;
            return;
        }

        // A setup program that ran for a couple of seconds cannot have installed a game;
        // it almost certainly failed to open at all.
        if elapsed < std::time::Duration::from_secs(3) {
            tracing::warn!(
                game_id,
                ?elapsed,
                "installer exited almost immediately; it probably never opened"
            );
        }

        // The installer was pointed at a shell-safe folder name, so move the result to
        // the name the rest of the app expects.
        if let Some(root) = install_dir.parent() {
            let written_to = root.join(&safe_folder);
            if written_to != install_dir && written_to.is_dir() && !dir_is_empty(&written_to).await
            {
                let _ = tokio::fs::remove_dir(&install_dir).await;
                match tokio::fs::rename(&written_to, &install_dir).await {
                    Ok(()) => tracing::info!(
                        game_id,
                        from = ?written_to,
                        to = ?install_dir,
                        "adopted the installed folder"
                    ),
                    Err(e) => {
                        tracing::warn!(game_id, error = %e, "could not rename the installed folder")
                    }
                }
            }
        }

        let populated = tokio::task::spawn_blocking({
            let dir = install_dir.clone();
            move || {
                gameyfin_core::extract::directory_size(&dir)
                    .map(|size| size > 0)
                    .unwrap_or(false)
            }
        })
        .await
        .unwrap_or(false);

        if populated {
            tracing::info!(
                game_id,
                ?install_dir,
                "installer populated the games folder"
            );
            finish_install(&library, game_id, &install_dir).await;
        } else {
            tracing::warn!(
                game_id,
                ?install_dir,
                ?elapsed,
                "installer left the games folder empty"
            );
            library
                .set_activity(
                    game_id,
                    Activity::Failed {
                        message: "The installer finished without putting anything in the \
                                  suggested folder. If you chose a different location, use \
                                  \"I installed it myself\" to point the app at it."
                            .into(),
                        stage: Stage::Install,
                    },
                )
                .await;
        }

        notify_state(&app, &library, game_id).await;
    });

    Ok(())
}

/// Record a completed install and choose its launch executable.
async fn finish_install(
    library: &crate::library_state::SharedLibraryState,
    game_id: i64,
    install_dir: &Path,
) {
    let dir = install_dir.to_path_buf();
    let scan = tokio::task::spawn_blocking(move || {
        // No title to match against here; scoring still excludes redistributables and
        // tooling, which is the bulk of the noise.
        let detection = gameyfin_core::executable::detect(&dir, "");
        let installers = gameyfin_core::executable::find_installers(&dir).unwrap_or_default();
        (detection, installers)
    })
    .await;

    let (executable, setup_candidates) = match scan {
        Ok((detection, installers)) => {
            let relative = |path: &Path| {
                path.strip_prefix(install_dir)
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
            };
            let executable = match detection {
                Ok(gameyfin_core::Detection::Confident(path)) => relative(&path),
                // Ambiguous or none: leave it unset so the user chooses.
                _ => None,
            };
            (
                executable,
                installers.iter().filter_map(|p| relative(p)).collect(),
            )
        }
        Err(e) => {
            tracing::warn!(game_id, error = %e, "could not scan the install directory");
            (None, Vec::new())
        }
    };

    tracing::info!(game_id, ?install_dir, ?executable, "install recorded");

    // The staging directory survives an install, so rescan it once here rather than on
    // every library listing.
    let staging = library.record(game_id).await.existing_staging();
    let staging_setups = match staging {
        Some(dir) => {
            tokio::task::spawn_blocking(move || crate::library_state::scan_staging_setups(&dir))
                .await
                .unwrap_or_default()
        }
        None => Vec::new(),
    };

    library
        .update_record(game_id, |r| {
            r.install_dir = Some(install_dir.to_path_buf());
            r.executable = executable;
            r.setup_candidates = setup_candidates;
            r.staging_setups = staging_setups;
            r.installed_at = Some(now_iso8601());
        })
        .await;
    library.clear_activity(game_id).await;
}

/// Point the app at a folder the user installed into themselves.
#[tauri::command]
pub async fn locate_install(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    path: String,
) -> CommandResult<()> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(CommandError::Message(format!("{path} is not a folder.")));
    }

    tracing::info!(game_id, %path, "adopting a user-chosen install folder");
    finish_install(&state.library_handle(), game_id, &dir).await;
    notify(&app);
    Ok(())
}

/// Run a setup program found inside an installed game's folder.
#[tauri::command]
pub async fn run_setup(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    relative: String,
) -> CommandResult<()> {
    let record = state.library().record(game_id).await;
    // Try both roots: a setup program can be among the installed files or still sitting
    // in the unpacked download, and a DLC installer is normally the latter.
    let base = [record.install_dir.clone(), record.extracted_dir.clone()]
        .into_iter()
        .flatten()
        .find(|d| d.join(&relative).is_file())
        .ok_or_else(|| {
            CommandError::Message(format!("{relative} could not be found for this game."))
        })?;

    let install_dir = install_dir_for(&state, game_id).await?;
    state
        .library()
        .update_record(game_id, |r| r.used_setup = Some(relative.clone()))
        .await;
    run_program_as_installer(&app, &state, game_id, &base.join(&relative), &install_dir).await
}

/// Locate a Windows runtime and get this game's prefix ready to run in.
///
/// Reports progress as `Preparing`, because a first run downloads a Proton build and
/// otherwise looks like the app doing nothing for several minutes.
async fn ready_windows_prefix(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
) -> CommandResult<(gameyfin_core::WindowsRuntime, PathBuf)> {
    let config_dir = state.config_dir().await;
    let Some(runtime) = gameyfin_core::detect_windows_runtime_in(Some(&config_dir)) else {
        tracing::warn!(game_id, "no Windows runtime found");
        return Err(CommandError::Message(gameyfin_core::windows_runtime_hint()));
    };

    // Wine fails within milliseconds against a path whose parent is missing, which looks
    // exactly like the program having done nothing.
    let prefix = prefix_dir_for(state, game_id).await?;
    tokio::fs::create_dir_all(&prefix).await.map_err(|e| {
        tracing::error!(game_id, ?prefix, error = %e, "could not create the prefix");
        CommandError::Message(format!("could not create the compatibility prefix: {e}"))
    })?;

    let dpi = gameyfin_core::dpi_for_scale(screen_scale(app));
    if !gameyfin_core::prefix::is_prepared(&prefix, dpi) {
        state
            .library()
            .set_activity(
                game_id,
                Activity::Preparing {
                    message: format!(
                        "Setting up {} for this game. The first run downloads a Proton \
                         build and can take several minutes.",
                        runtime.description()
                    ),
                },
            )
            .await;
        notify_state(app, state.library(), game_id).await;
    }

    if let Err(e) = prepare_prefix(&runtime, &prefix, dpi).await {
        tracing::warn!(game_id, "{e}");
        state.library().clear_activity(game_id).await;
        notify_state(app, state.library(), game_id).await;
        return Err(CommandError::Message(e));
    }

    Ok((runtime, prefix))
}

/// Initialise a compatibility prefix and set its DPI, once.
///
/// Wine's first run in a new prefix updates it and, without the overrides applied here,
/// asks whether to install Mono and Gecko. Those dialogs open behind the update window,
/// so an unattended prefix appears to hang indefinitely.
async fn prepare_prefix(
    runtime: &gameyfin_core::WindowsRuntime,
    prefix: &Path,
    dpi: u32,
) -> Result<(), String> {
    if gameyfin_core::prefix::is_prepared(prefix, dpi) {
        return Ok(());
    }

    // umu downloads a Proton build on first use, hundreds of megabytes, with no output
    // until it finishes. Saying so up front is the difference between "slow" and
    // "broken" from the user's side.
    tracing::info!(
        ?prefix,
        dpi,
        runtime = runtime.kind(),
        "preparing prefix; the first run may download a Proton build and take several minutes"
    );

    let boot = gameyfin_core::prefix::boot_command(runtime, prefix);
    match gameyfin_core::run_capturing(&boot).await {
        Ok(run) if run.success() => tracing::info!(?prefix, "prefix initialised"),
        Ok(run) => tracing::warn!(
            ?prefix,
            status = ?run.status,
            output = %run.tail(8),
            "prefix initialisation reported an error; continuing"
        ),
        Err(e) => {
            return Err(format!(
                "could not initialise the compatibility prefix: {e}"
            ))
        }
    }

    let dpi_cmd = gameyfin_core::prefix::dpi_command(runtime, prefix, dpi);
    match gameyfin_core::run_capturing(&dpi_cmd).await {
        Ok(run) if run.success() => tracing::info!(?prefix, dpi, "prefix DPI applied"),
        Ok(run) => tracing::warn!(?prefix, output = %run.tail(4), "could not set the prefix DPI"),
        Err(e) => tracing::warn!(?prefix, error = %e, "could not set the prefix DPI"),
    }

    // Wine draws the classic Windows 2000 caption and controls unless a theme is active.
    // wine.inf normally turns this on when it creates the prefix; repairing it here covers
    // the prefixes where that did not happen.
    if gameyfin_core::prefix::has_bundled_theme(prefix) {
        for cmd in gameyfin_core::prefix::theme_commands(runtime, prefix) {
            match gameyfin_core::run_capturing(&cmd).await {
                Ok(run) if run.success() => {}
                Ok(run) => {
                    tracing::warn!(?prefix, output = %run.tail(4), "could not set the prefix theme")
                }
                Err(e) => tracing::warn!(?prefix, error = %e, "could not set the prefix theme"),
            }
        }
        tracing::info!(?prefix, "prefix theme applied");
    } else {
        tracing::warn!(
            ?prefix,
            "this Wine build ships no aero.msstyles, so windows keep the classic look"
        );
    }

    // `wineboot` can report success while leaving an unusable prefix, most often when
    // something pre-created a directory it uses to detect an existing install. Checking
    // for `drive_c` turns that into a clear failure instead of a wall of Wine errors from
    // whatever runs next.
    if !gameyfin_core::prefix::wine_root(prefix)
        .join("drive_c")
        .is_dir()
    {
        return Err(format!(
            "The compatibility prefix at {} was not set up correctly: drive_c is missing. \
             Deleting that folder and trying again usually fixes it.",
            prefix.display()
        ));
    }

    if let Err(e) = gameyfin_core::prefix::mark_prepared(prefix, dpi) {
        tracing::debug!(?prefix, error = %e, "could not record prefix preparation");
    }
    Ok(())
}

/// The Windows path a setup wizard should be given for this game.
///
/// `None` on Windows, where the native path is already correct and no drive mapping
/// exists.
fn windows_install_path(install_dir: &Path) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let folder = install_dir.file_name()?.to_string_lossy().into_owned();
    Some(gameyfin_core::games_drive_path(&folder))
}

/// The main window's scale factor, for sizing a compatibility prefix's DPI.
///
/// Falls back to 1.0 rather than guessing: an over-large DPI is as unusable as one that
/// is too small.
fn screen_scale(app: &AppHandle) -> f64 {
    app.get_webview_window("main")
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0)
}

/// Tell the UI that local state changed, without saying what.
///
/// Triggers a full refresh, so it is for structural changes, a game deleted, a rescan,
/// rather than progress.
fn notify(app: &AppHandle) {
    let _ = app.emit("library-changed", ());
}

/// One game's state, pushed so the UI can update in place.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GameStateEvent {
    game_id: i64,
    state: GameState,
}

/// Push a single game's state to the UI.
///
/// Progress must not trigger a full refresh: `list_entries` reads the whole catalogue, so
/// refreshing several times a second left the interface lagging far behind the work it was
/// reporting on, which is why a running extraction appeared stuck at 0%.
async fn notify_state(app: &AppHandle, library: &crate::library_state::LibraryState, game_id: i64) {
    let state = library.state_of(game_id).await;
    let _ = app.emit("game-state", GameStateEvent { game_id, state });
}

/// Re-read the library folder and adopt whatever is there.
///
/// Games installed by an earlier version, restored from a backup, or copied from another
/// machine are invisible until something looks for them.
#[tauri::command]
pub async fn rescan_library(app: AppHandle, state: State<'_, AppState>) -> CommandResult<usize> {
    let root = library_root(&state).await?;

    // A rescan is also the natural moment to re-read the catalogue, but only when there
    // is a server to read it from. Every view rescans as it opens, and against a server
    // that is known to be down each of those would buy nothing but a connect timeout;
    // the local folder walk below is the part that matters offline.
    if state.is_unreachable() {
        tracing::debug!("skipping the catalogue refresh: the server is unreachable");
    } else {
        state.invalidate_catalog().await;
    }

    let found = state.library().rescan(Path::new(&root)).await;
    tracing::info!(root = %root, found, "rescanned the library folder");
    notify(&app);
    Ok(found)
}

fn cookie_header(cookies: &std::collections::HashMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Current time as an ISO-8601 string, for record timestamps.
fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Good enough for display and ordering without pulling in a date library.
    format!("@{secs}")
}

/// Launch an installed game and account for the play session.
#[tauri::command]
pub async fn launch_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    // Every exit from this function is logged. A launch that fails silently is the worst
    // possible outcome: the button appears to do nothing, and there is nothing to read
    // afterwards to find out why.
    tracing::info!(game_id, "launch requested");

    if state.library().is_busy(game_id).await {
        tracing::info!(game_id, "already busy; ignoring the launch request");
        return Ok(());
    }

    let record = state.library().record(game_id).await;
    let Some(install_dir) = record.install_dir.clone().filter(|d| d.exists()) else {
        tracing::warn!(game_id, dir = ?record.install_dir, "not installed, or its folder is gone");
        return Err(CommandError::Message("This game is not installed.".into()));
    };

    let Some(executable) = record
        .executable
        .as_ref()
        .map(|e| install_dir.join(e))
        .filter(|p| p.exists())
    else {
        tracing::warn!(
            game_id,
            executable = ?record.executable,
            ?install_dir,
            "no usable launch executable"
        );
        return Err(CommandError::Message(
            "No launch executable has been chosen for this game yet. Pick one under the \
             game's options in Installed."
                .into(),
        ));
    };

    // A Windows game on Linux runs under Proton (or Wine), each game in its own prefix so
    // one game's configuration cannot disturb another's. The runtime is located rather
    // than assumed: a desktop entry does not inherit a shell's PATH, and reporting
    // "No such file or directory" for a missing umu-run tells the user nothing.
    // Checked before the prefix is built: a file that is not a program fails whatever
    // Wine does with it, and thirteen seconds of prefix preparation only delays the
    // answer. Wine reports `Bad format`, buried under unrelated startup chatter.
    if gameyfin_core::needs_proton(&executable)
        && !gameyfin_core::looks_like_windows_program(&executable)
    {
        tracing::error!(game_id, program = ?&executable, "not a Windows program");
        return Err(CommandError::Message(format!(
            "{} is not a Windows program. The install may be incomplete or corrupt.",
            executable
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| executable.display().to_string())
        )));
    }

    let config = if gameyfin_core::needs_proton(&executable) {
        let (runtime, prefix) = ready_windows_prefix(&app, &state, game_id).await?;
        tracing::info!(
            game_id,
            runtime = runtime.kind(),
            detail = runtime.description(),
            ?executable,
            "launching a Windows game"
        );
        gameyfin_core::LaunchConfig::for_windows_program_unattended(executable, prefix, &runtime)
    } else {
        tracing::info!(game_id, ?executable, "launching a native game");
        gameyfin_core::LaunchConfig::native(executable)
    };

    let command = gameyfin_core::resolve_command(&config).map_err(|e| {
        tracing::error!(game_id, error = %e, "could not build the launch command");
        CommandError::Message(e.to_string())
    })?;
    tracing::debug!(game_id, program = ?command.program, args = ?command.args, "resolved command");

    let library = state.library_handle();
    tauri::async_runtime::spawn(async move {
        let supervisor = match gameyfin_core::Supervisor::spawn(&command) {
            Ok(supervisor) => supervisor,
            Err(e) => {
                tracing::error!("could not launch game {game_id}: {e}");
                library
                    .set_activity(
                        game_id,
                        Activity::Failed {
                            message: e.to_string(),
                            stage: Stage::Launch,
                        },
                    )
                    .await;
                notify(&app);
                return;
            }
        };

        library
            .set_activity(
                game_id,
                Activity::Running {
                    since: now_iso8601(),
                },
            )
            .await;
        notify(&app);

        match supervisor.wait().await {
            Ok(session) => {
                tracing::info!(
                    "game {game_id} ran for {:?} and ended {:?}",
                    session.duration,
                    session.end
                );
                // A launch that dies in seconds is a failure, not a play session, and
                // must not accumulate playtime.
                if session.is_meaningful() {
                    let minutes = session.minutes_played();
                    library
                        .update_record(game_id, |r| {
                            r.minutes_played += minutes;
                            r.last_played_at = Some(now_iso8601());
                        })
                        .await;
                    library.clear_activity(game_id).await;
                } else if let Some(message) = failed_to_start(&session) {
                    // Previously this just cleared the activity, so a game that never
                    // started looked exactly like one the user closed straight away: the
                    // window simply never appeared and nothing said why.
                    tracing::error!(
                        game_id,
                        output = %session.error_output,
                        "the game exited immediately"
                    );
                    library
                        .set_activity(
                            game_id,
                            Activity::Failed {
                                message,
                                stage: Stage::Launch,
                            },
                        )
                        .await;
                } else {
                    library.clear_activity(game_id).await;
                }
            }
            Err(e) => {
                tracing::error!("supervising game {game_id} failed: {e}");
                library
                    .set_activity(
                        game_id,
                        Activity::Failed {
                            message: e.to_string(),
                            stage: Stage::Launch,
                        },
                    )
                    .await;
            }
        }

        notify(&app);
    });

    Ok(())
}
