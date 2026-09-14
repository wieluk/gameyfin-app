//! The library list, downloads, extraction, folders and per-game options.

use std::path::{Path, PathBuf};

use gameyfin_api::Game;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use super::{notify, notify_state, LibraryFolder};
use crate::error::{blocking, CommandError, CommandResult, Context};
use crate::library_state::{scan_setups, Activity, GameState, Stage, EXTRACT_DIR};
use crate::progress::Throttle;
use crate::state::AppState;

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryEntry {
    pub game: Game,
    pub state: GameState,
    pub minutes_played: u32,
    pub last_played_at: Option<String>,
    pub archive_present: bool,
    /// On the `gfimg://` scheme, since an `<img>` cannot present our session.
    pub cover_url: Option<String>,
    pub header_url: Option<String>,
    pub screenshot_urls: Vec<String>,
    /// Third-party links, passed through untouched.
    pub video_urls: Vec<String>,
}

#[tauri::command]
pub async fn list_entries(state: State<'_, AppState>) -> CommandResult<Vec<LibraryEntry>> {
    // A cached catalogue renders without a client; otherwise the UI returns to the wizard.
    if state.client().is_none() && !state.has_cached_catalog() {
        return Err(CommandError::NotConnected);
    }
    let games = state.games().await?;
    let library = state.library().clone();
    blocking("could not read the library", move || {
        let image = |i: &gameyfin_api::Image| crate::images::url_for(&i.path());
        Ok::<_, std::convert::Infallible>(
            games
                .into_iter()
                .map(|game| {
                    let record = library.record(game.id);
                    LibraryEntry {
                        cover_url: game.cover.as_ref().map(image),
                        header_url: game.header.as_ref().map(image),
                        screenshot_urls: game.images.iter().map(image).collect(),
                        video_urls: game.video_urls.clone(),
                        minutes_played: record.minutes_played,
                        last_played_at: record.last_played_at.clone(),
                        archive_present: record.existing_archive().is_some(),
                        state: library.state_from(game.id, record),
                        game,
                    }
                })
                .collect(),
        )
    })
    .await
}

/// Returns once the transfer is under way; progress arrives as events.
#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    root: Option<String>,
) -> CommandResult<()> {
    let library = state.library().clone();
    let idle = Activity::Downloading {
        received_bytes: 0,
        total_bytes: 0,
        bytes_per_second: 0.0,
    };
    let Some(claim) = library.claim(game_id, idle) else {
        return Ok(());
    };
    notify(&app);

    let client = state.require_client()?;
    let settings = state.settings();
    // Without a named folder a retry resumes beside the partial file.
    let root = match root {
        Some(root) => settings
            .require_root(Some(root))
            .map_err(CommandError::Message)?,
        None => super::root_for_game(&state, game_id)?,
    };
    let game = state.game(game_id).await?;
    let providers = client.download_providers().await.map_err(|e| {
        if e.is_unreachable() {
            state.set_reachable(false);
            CommandError::msg(
                "Cannot reach the server to download from. Installed games still work.",
            )
        } else {
            e.into()
        }
    })?;
    let provider =
        super::session::choose_provider(&providers, settings.download_provider.as_deref())
            .ok_or_else(|| CommandError::msg("The server has no download provider enabled."))?
            .clone();

    // Refused up front rather than after gigabytes: the server knows the size.
    if game.metadata.file_size > 0 {
        gameyfin_core::download::check_space(Path::new(&root), game.metadata.file_size)
            .map_err(|e| CommandError::msg(e.to_string()))?;
    }

    let url = client.download_url(&game, &provider.key);
    let destination = crate::downloads::download_path(
        &root,
        game.id,
        &game.title,
        &crate::downloads::provisional_filename(&game.title),
    );
    // A folder game arrives as a zip built on the fly, unpacked as it comes in when allowed.
    let unpack_into = settings
        .unpack_while_downloading
        .then(|| destination.parent().map(|dir| dir.join(EXTRACT_DIR)))
        .flatten();
    let expected_bytes = game.metadata.file_size;
    let cookie_header = gameyfin_api::cookie_header(&settings.cookies);
    let rate_limit = state.download_limit();
    rate_limit.set(u64::from(settings.download_limit_kib) * 1024);
    let downloader = gameyfin_core::Downloader::new(state.transfer_http())
        .with_shared_rate_limit(rate_limit)
        .with_cancel(state.downloads.register(game_id));

    tauri::async_runtime::spawn(async move {
        let _claim = claim;
        let on_progress = {
            let (library, app) = (library.clone(), app.clone());
            let mut throttle = Throttle::new(300);
            move |p: gameyfin_core::Progress| {
                library.set_activity(
                    game_id,
                    Activity::Downloading {
                        received_bytes: p.received_bytes,
                        total_bytes: p.total_bytes.unwrap_or(expected_bytes),
                        bytes_per_second: p.bytes_per_second,
                    },
                );
                if throttle.ready() {
                    notify(&app);
                }
            }
        };
        let authorize = |req: reqwest::RequestBuilder| {
            if cookie_header.is_empty() {
                req
            } else {
                req.header(reqwest::header::COOKIE, &cookie_header)
            }
        };
        let result = downloader
            .download_or_unpack(
                &url,
                &destination,
                unpack_into.as_deref(),
                authorize,
                on_progress,
            )
            .await;
        app.state::<AppState>().downloads.finish(game_id);

        use gameyfin_core::download::Outcome;
        match result {
            // The torrent provider answers with metainfo, which is not a game to install.
            Ok(Outcome::File(outcome))
                if gameyfin_core::extract::is_torrent_metainfo(&outcome.path) =>
            {
                let _ = tokio::fs::remove_file(&outcome.path).await;
                let message = format!(
                    "{} hands back a torrent, which Gameyfin cannot download from. Pick another provider under Downloads.",
                    provider.name
                );
                fail(&app, game_id, Stage::Download, &game.title, message).await;
            }
            Ok(Outcome::File(outcome)) => {
                tracing::info!(game_id, path = ?outcome.path, bytes = outcome.bytes, "download finished");
                library
                    .update_record(game_id, |r| {
                        r.archive_path = Some(outcome.path.clone());
                        r.archive_bytes = outcome.bytes;
                    })
                    .await;
                library.clear_activity(game_id);
                if app.state::<AppState>().settings().auto_install {
                    notify(&app);
                    let installed =
                        super::install::auto_install_download(&app, game_id, &game.title).await;
                    if let Err(e) = installed {
                        fail(&app, game_id, Stage::Install, &game.title, e.to_string()).await;
                    }
                } else {
                    crate::notify::download_finished(&app, &game.title).await;
                }
            }
            Ok(Outcome::Unpacked { dir, bytes }) => {
                tracing::info!(game_id, ?dir, bytes, "download unpacked as it arrived");
                let setups = {
                    let dir = dir.clone();
                    tokio::task::spawn_blocking(move || scan_setups(&dir))
                        .await
                        .unwrap_or_default()
                };
                record_extracted(&app, game_id, &game.title, dir, setups, true).await;
                if !app.state::<AppState>().settings().auto_install {
                    crate::notify::download_finished(&app, &game.title).await;
                }
            }
            Err(gameyfin_core::CoreError::Cancelled) => {
                tracing::info!(game_id, "download cancelled");
                library.clear_activity(game_id);
            }
            Err(e) => fail(&app, game_id, Stage::Download, &game.title, e.to_string()).await,
        }
        notify(&app);
    });
    Ok(())
}

/// Records a failure on the game's row and tells the desktop.
pub async fn fail(app: &AppHandle, game_id: i64, stage: Stage, title: &str, message: String) {
    tracing::error!(game_id, ?stage, "{message}");
    app.state::<AppState>()
        .library()
        .fail(game_id, stage, message.clone());
    crate::notify::failed(app, stage.label(), title, &message).await;
}

/// A saved file keeps its checkpoint for a resume; a download being unpacked starts over.
#[tauri::command]
pub async fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    if let Some(cancel) = state.downloads.get(game_id) {
        cancel.cancel();
    }
    notify(&app);
    Ok(())
}

/// Unpacks beside the archive; the files stay in Downloads until the user decides.
pub async fn extract_download(
    app: &AppHandle,
    game_id: i64,
    delete_archive: Option<bool>,
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let library = state.library().clone();
    let archive = library
        .record(game_id)
        .existing_archive()
        .ok_or_else(|| CommandError::msg("This game has not been downloaded yet."))?;
    let staging = archive
        .parent()
        .ok_or_else(|| CommandError::msg("The download has no folder."))?
        .join(EXTRACT_DIR);
    let Some(claim) = library.claim(game_id, Activity::Extracting { percent: 0.0 }) else {
        return Ok(());
    };
    let settings = state.settings();
    let password = settings.extraction_password;
    let delete_archive = delete_archive.unwrap_or(settings.delete_archive_after_extract);
    tracing::info!(game_id, ?staging, delete_archive, "extracting download");
    notify_state(app, game_id);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _claim = claim;
        let unpack = {
            let (library, app, archive, staging) = (
                library.clone(),
                app.clone(),
                archive.clone(),
                staging.clone(),
            );
            tokio::task::spawn_blocking(move || {
                let mut throttle = Throttle::new(200);
                let bytes = gameyfin_core::extract::extract_with(
                    &archive,
                    &staging,
                    password.as_deref(),
                    |p| {
                        if throttle.ready() {
                            library.set_activity(
                                game_id,
                                Activity::Extracting {
                                    percent: p.percent(),
                                },
                            );
                            notify_state(&app, game_id);
                        }
                    },
                )?;
                Ok::<_, gameyfin_core::CoreError>((bytes, scan_setups(&staging)))
            })
            .await
        };
        let title = app.state::<AppState>().title(game_id).await;

        let setups = match unpack
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
        {
            Ok((bytes, setups)) => {
                tracing::info!(game_id, bytes, ?setups, "extraction finished");
                setups
            }
            Err(e) => return fail(&app, game_id, Stage::Extract, &title, e).await,
        };

        if delete_archive {
            if let Err(e) = tokio::fs::remove_file(&archive).await {
                tracing::warn!(game_id, error = %e, "could not remove the archive");
            }
        }
        record_extracted(&app, game_id, &title, staging, setups, delete_archive).await;
    });
    Ok(())
}

/// Records unpacked files, then installs them when that is switched on. Shared by the extract
/// step and a download that unpacked as it arrived.
async fn record_extracted(
    app: &AppHandle,
    game_id: i64,
    title: &str,
    staging: PathBuf,
    setups: Vec<String>,
    archive_gone: bool,
) {
    let library = app.state::<AppState>().library().clone();
    library
        .update_record(game_id, |r| {
            r.extracted_dir = Some(staging.clone());
            r.staging_setups = setups.clone();
            r.setup_candidates = setups.clone();
            if archive_gone {
                r.archive_path = None;
                r.archive_bytes = 0;
            }
        })
        .await;
    library.clear_activity(game_id);

    // A setup wizard asks questions a checkbox should not answer for the user.
    if app.state::<AppState>().settings().auto_install {
        if setups.is_empty() {
            super::install::finish_auto_install(app, game_id).await;
        } else {
            crate::notify::send(
                app,
                crate::notify::Category::Transfer,
                "Setup needed",
                &format!("{title} came with an installer. Run it from Downloads."),
            )
            .await;
        }
    }
    notify_state(app, game_id);
}

/// Adopts games restored from a backup or copied from another machine, in every games folder.
#[tauri::command]
pub async fn rescan_library(app: AppHandle, state: State<'_, AppState>) -> CommandResult<usize> {
    let roots = state.settings().library_roots();
    if roots.is_empty() {
        return Err(CommandError::msg(
            "No games folder configured yet. Set one in Settings.",
        ));
    }
    // Every view rescans on open; a known-down server would only add a connect timeout.
    if !state.is_unreachable() {
        state.invalidate_catalog();
    }
    let mut found = 0;
    for root in &roots {
        found += state.library().rescan(Path::new(root)).await;
    }
    notify(&app);
    Ok(found)
}

/// Deletes the unpacked files left in Downloads after installing.
#[tauri::command]
pub async fn delete_staging(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    let _claim = claim(&state, game_id, "Deleting the unpacked files")?;
    if let Some(dir) = state.library().record(game_id).existing_staging() {
        remove_tree(&dir).await?;
        if let Some(parent) = dir.parent() {
            if super::dir_is_empty(parent).await {
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

/// Deletes everything a game has in Downloads and any failure left from it.
#[tauri::command]
pub async fn delete_download(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    let claim = claim(&state, game_id, "Deleting the download")?;
    remove_download(&state, game_id, None).await?;
    drop(claim);
    state.library().clear_activity(game_id);
    notify(&app);
    Ok(())
}

fn claim(state: &AppState, game_id: i64, what: &str) -> CommandResult<crate::library_state::Claim> {
    state
        .library()
        .claim(game_id, Activity::preparing(what))
        .ok_or_else(|| CommandError::msg("Wait for what this game is doing to finish."))
}

async fn remove_tree(dir: &Path) -> CommandResult<()> {
    tracing::info!(?dir, "deleting");
    tokio::fs::remove_dir_all(dir)
        .await
        .context(format!("Could not delete {}", dir.display()))
}

/// Where a game's download files can be, deduplicated so a parent takes its children.
fn download_targets(
    record: &crate::library_state::GameRecord,
    derived: Option<PathBuf>,
) -> Vec<PathBuf> {
    let parent = |p: &PathBuf| p.parent().map(Path::to_path_buf);
    let mut targets: Vec<PathBuf> = Vec::new();
    for candidate in [
        record.archive_path.as_ref().and_then(parent),
        record.extracted_dir.as_ref().and_then(parent),
        record.extracted_dir.clone(),
        derived,
    ]
    .into_iter()
    .flatten()
    {
        if !targets.iter().any(|t| candidate.starts_with(t)) {
            targets.retain(|t| !t.starts_with(&candidate));
            targets.push(candidate);
        }
    }
    targets
}

/// Removes and forgets a game's download files. Keeps everything when `keep` sits inside them.
async fn remove_download(state: &AppState, game_id: i64, keep: Option<&Path>) -> CommandResult<()> {
    let record = state.library().record(game_id);
    let targets = download_targets(&record, super::downloads_dir_for(state, game_id).await.ok());
    if keep.is_some_and(|keep| targets.iter().any(|t| keep.starts_with(t))) {
        tracing::warn!(
            game_id,
            ?keep,
            "not deleting a download the game is installed in"
        );
        return Ok(());
    }
    for target in targets.iter().filter(|t| t.exists()) {
        remove_tree(target).await?;
    }
    state
        .library()
        .update_record(game_id, |r| {
            r.archive_path = None;
            r.archive_bytes = 0;
            r.extracted_dir = None;
            r.staging_setups.clear();
        })
        .await;
    Ok(())
}

/// After an install, deletes the download if the user asked. Failure only logs.
pub async fn discard_download_if_asked(app: &AppHandle, game_id: i64, install_dir: &Path) {
    let state = app.state::<AppState>();
    if !state.settings().delete_download_after_install {
        return;
    }
    if let Err(e) = remove_download(&state, game_id, Some(install_dir)).await {
        tracing::warn!(
            game_id,
            "could not delete the download after installing: {e}"
        );
    }
}

/// Opens a local folder. Files are refused, since the OS handler would run a program.
#[tauri::command]
pub async fn open_folder(app: AppHandle, path: String) -> CommandResult<()> {
    if !Path::new(&path).is_dir() {
        return Err(CommandError::msg(format!("{path} is not a folder.")));
    }
    app.opener()
        .open_path(&path, None::<&str>)
        .context(format!("could not open {path}"))
}

/// Opens a web page. Only http and https, since links can come from the server.
#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> CommandResult<()> {
    let parsed = url::Url::parse(&url).context("That is not a web address")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(CommandError::msg("Only web addresses can be opened."));
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .context("could not open the link")
}

/// `folder` picks which of the game's folders, since Downloads must open the download.
#[tauri::command]
pub async fn open_game_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    folder: Option<LibraryFolder>,
) -> CommandResult<()> {
    let record = state.library().record(game_id);
    let parent = |p: &PathBuf| p.parent().map(Path::to_path_buf);
    let downloads: Vec<PathBuf> = [
        record.extracted_dir.as_ref().and_then(parent),
        record.archive_path.as_ref().and_then(parent),
        super::downloads_dir_for(&state, game_id).await.ok(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let installed: Vec<PathBuf> = record.install_dir.into_iter().collect();
    let candidates: Vec<PathBuf> = match folder {
        Some(LibraryFolder::Downloads) => downloads,
        Some(LibraryFolder::Installations) => installed,
        None => installed.into_iter().chain(downloads).collect(),
    };

    // The portal silently ignores a missing path, so fall back to the library folder.
    let dir = match candidates.into_iter().find(|p| p.is_dir()) {
        Some(existing) => existing,
        None => {
            let layout = super::layout_for(&state, game_id)?;
            let root = folder.unwrap_or(LibraryFolder::Downloads).dir(&layout);
            tokio::fs::create_dir_all(&root)
                .await
                .context(format!("could not create {}", root.display()))?;
            root
        }
    };
    tracing::info!(game_id, ?dir, "opening a game folder");
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .context("could not open the folder")
}

/// A named games folder must be a configured one.
#[tauri::command]
pub async fn open_library_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder: LibraryFolder,
    root: Option<String>,
) -> CommandResult<()> {
    let root = state
        .settings()
        .require_root(root)
        .map_err(CommandError::Message)?;
    let dir = folder.dir(&gameyfin_core::InstallLayout::new(&root));
    tokio::fs::create_dir_all(&dir)
        .await
        .context(format!("could not create {}", dir.display()))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .context("could not open the folder")
}

/// A partial update of a game's own options; a field not sent is left alone.
#[derive(Debug, Default, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct GameOptionsPatch {
    pub launch_arguments: Option<String>,
    pub installer_arguments: Option<String>,
    pub launch_environment: Option<String>,
    /// `"auto"` or empty clears the override.
    pub runtime_override: Option<String>,
    /// Empty clears the pin.
    pub proton_build: Option<String>,
    pub launch_toggles: Option<gameyfin_core::environment::LaunchToggles>,
}

#[tauri::command]
pub async fn set_game_options(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    options: GameOptionsPatch,
) -> CommandResult<()> {
    let chosen = |value: String| Some(value).filter(|v| !v.is_empty() && v != "auto");
    state
        .library()
        .update_record(game_id, |r| {
            if let Some(v) = options.launch_arguments {
                r.launch_arguments = v;
            }
            if let Some(v) = options.installer_arguments {
                r.installer_arguments = v;
            }
            if let Some(v) = options.launch_environment {
                r.launch_environment = v;
            }
            if let Some(v) = options.runtime_override {
                r.runtime_override = chosen(v);
            }
            if let Some(v) = options.proton_build {
                r.proton_build = chosen(v);
            }
            if let Some(v) = options.launch_toggles {
                r.launch_toggles = v;
            }
        })
        .await;
    notify(&app);
    Ok(())
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameOptions {
    pub launch_arguments: String,
    pub installer_arguments: String,
    pub launch_environment: String,
    pub runtime_override: Option<String>,
    /// Only runtimes this machine has, so the picker cannot name one that fails at launch.
    pub available_runtimes: Vec<RuntimeChoice>,
    pub proton_build: Option<String>,
    pub proton_builds: Vec<gameyfin_core::proton::InstalledProton>,
    pub launch_toggles: gameyfin_core::environment::LaunchToggles,
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimeChoice {
    pub kind: String,
    pub label: String,
}

#[tauri::command]
pub async fn game_options(state: State<'_, AppState>, game_id: i64) -> CommandResult<GameOptions> {
    let record = state.library().record(game_id);
    let config_dir = state.config_dir();
    let ctx = crate::proton::runtime_context(&state, Some(game_id), None).await;
    let (available_runtimes, proton_builds) = tokio::task::spawn_blocking(move || {
        let runtimes = ["umu", "bundled-wine", "wine", "host-wine"]
            .into_iter()
            .filter_map(|kind| {
                gameyfin_core::find_windows_runtime(&ctx, kind).map(|runtime| RuntimeChoice {
                    kind: kind.to_string(),
                    // The Proton build is picked separately.
                    label: if runtime.is_wine_family() {
                        runtime.description()
                    } else {
                        "Proton (umu)".to_string()
                    },
                })
            })
            .collect();
        (runtimes, gameyfin_core::proton::available(&config_dir))
    })
    .await
    .unwrap_or_default();

    Ok(GameOptions {
        launch_arguments: record.launch_arguments,
        installer_arguments: record.installer_arguments,
        launch_environment: record.launch_environment,
        runtime_override: record.runtime_override,
        available_runtimes,
        proton_build: record.proton_build,
        proton_builds,
        launch_toggles: record.launch_toggles,
    })
}

/// `executable` must be a file inside the game's install folder, given either relative to
/// it or as the absolute path a file dialog returns.
#[tauri::command]
pub async fn set_game_executable(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    executable: String,
) -> CommandResult<()> {
    let dir = state
        .library()
        .record(game_id)
        .install_dir
        .ok_or_else(|| CommandError::msg("This game is not installed."))?;
    let relative = relative_choice(&dir, &executable)?;
    if !super::contained(&dir, &relative)?.is_file() {
        return Err(CommandError::msg(format!("{executable} is not a file.")));
    }
    state
        .library()
        .update_record(game_id, |r| r.executable = Some(relative))
        .await;
    notify(&app);
    Ok(())
}

/// The path to store for a chosen file. A picker hands back an absolute path, of which only
/// the part inside the game's folder is worth keeping: the folder can move.
fn relative_choice(dir: &Path, chosen: &str) -> CommandResult<String> {
    let path = Path::new(chosen);
    if !path.is_absolute() {
        return Ok(chosen.to_string());
    }
    let outside = || CommandError::msg(format!("{chosen} is not inside this game's folder."));
    // Through the resolved paths, so a symlinked games folder does not read as outside it.
    let (resolved, root) = (
        path.canonicalize().map_err(|_| outside())?,
        dir.canonicalize().map_err(|_| outside())?,
    );
    crate::library_state::relative_to(&resolved, &root).ok_or_else(outside)
}

/// Everything launchable in the game's folder, best first, the user's choice pinned on top. A
/// choice no longer there is left out, which tells the picker to say so.
#[tauri::command]
pub async fn list_executables(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<Vec<String>> {
    let record = state.library().record(game_id);
    let Some(dir) = record.install_dir.filter(|d| d.exists()) else {
        return Ok(Vec::new());
    };
    let scan_dir = dir.clone();
    // Every candidate, not just the one detection would launch unattended: a picker that
    // offers a single file is a picker that cannot change anything.
    let paths = blocking("could not scan for executables", move || {
        gameyfin_core::executable::candidates(&scan_dir, "")
    })
    .await?
    .into_iter()
    .map(|candidate| candidate.path)
    .collect::<Vec<_>>();

    let settings = state.settings();
    let mut listed: Vec<String> = paths
        .iter()
        .filter_map(|p| crate::library_state::relative_to(p, &dir))
        .filter(|relative| !settings.is_ignored_executable(relative))
        .collect();
    if let Some(chosen) = record.executable.filter(|rel| dir.join(rel).exists()) {
        listed.retain(|rel| rel != &chosen);
        listed.insert(0, chosen);
    }
    Ok(listed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browsed_file_is_stored_relative_to_the_game() {
        let root = std::env::temp_dir().join(format!("gameyfin-choice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("game");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("game.exe"), b"MZ").unwrap();
        // Built rather than written out: the separator is the platform's.
        let relative = Path::new("bin").join("game.exe").display().to_string();

        // What a file dialog hands back.
        let absolute = dir.join("bin").join("game.exe").display().to_string();
        assert_eq!(relative_choice(&dir, &absolute).unwrap(), relative);
        // A relative path is already what we store.
        assert_eq!(relative_choice(&dir, &relative).unwrap(), relative);

        // Free choice stops at the game's folder.
        let elsewhere = root.join("other");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("game.exe"), b"MZ").unwrap();
        let outside = elsewhere.join("game.exe").display().to_string();
        assert!(relative_choice(&dir, &outside).is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn download_targets_collapse_into_their_parents() {
        let record = crate::library_state::GameRecord {
            archive_path: Some("/g/Downloads/(1) A/a.zip".into()),
            extracted_dir: Some("/g/Downloads/(1) A/extracted".into()),
            ..Default::default()
        };
        assert_eq!(
            download_targets(&record, Some("/g/Downloads/(1) A".into())),
            vec![PathBuf::from("/g/Downloads/(1) A")]
        );
        assert!(download_targets(&Default::default(), None).is_empty());
    }
}
