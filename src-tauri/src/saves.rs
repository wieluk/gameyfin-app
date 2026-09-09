//! Save synchronisation: drives Ludusavi locally and the server's `/saves` routes remotely.
//!
//! The decisions live in `gameyfin_core::save_sync`; this module resolves paths, runs the
//! sidecar, and reports progress to the UI.

use std::path::PathBuf;

use gameyfin_api::saves::{SaveVersion, UploadOutcome};
use gameyfin_core::save_store::{FolderStore, SaveStore, ServerStore, WebDavStore};
use gameyfin_core::save_sync::{self, LocalSaveState, SaveSync, SaveSyncState};
use gameyfin_core::{ConflictChoice, InstallLayout};
use gameyfin_saves::config::{Redirect, RedirectKind};
use gameyfin_saves::{
    BackupFormat, ConfigBuilder, GameIdentity, Ludusavi, RestoreStrategy, SavePlatform, TitleMatch,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::{CommandError, CommandResult};
use crate::settings::SaveBackend;
use crate::state::AppState;

/// One game's sync state, pushed so the UI updates in place rather than refetching.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveStateEvent {
    game_id: i64,
    state: SaveSyncState,
}

fn emit_state(app: &AppHandle, game_id: i64, state: &SaveSyncState) {
    let _ = app.emit(
        "save-state",
        SaveStateEvent {
            game_id,
            state: state.clone(),
        },
    );
}

/// Locates the bundled Ludusavi.
///
/// Tauri renames a sidecar to the plain binary name next to the executable when it is
/// bundled, but a `cargo run` build has it only under `binaries/` with the target triple
/// still attached, so both are tried.
fn ludusavi_binary(app: &AppHandle) -> CommandResult<PathBuf> {
    let name = if cfg!(windows) {
        "ludusavi.exe"
    } else {
        "ludusavi"
    };

    if let Ok(resources) = app.path().resource_dir() {
        let bundled = resources.join(name);
        if bundled.exists() {
            return Ok(bundled);
        }
    }

    // How every shipped format actually lays it out: deb, rpm and Flatpak put the sidecar
    // in the same bin directory as the app, AppImage keeps that layout inside the mount,
    // and the Windows installers drop it beside the exe.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join(name);
            if beside.exists() {
                return Ok(beside);
            }
            let unrenamed = dir.join(sidecar_file_name());
            if unrenamed.exists() {
                return Ok(unrenamed);
            }
        }
    }

    // The development layout, where `fetch-ludusavi.mjs` puts it.
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(sidecar_file_name());
    if checkout.exists() {
        return Ok(checkout);
    }

    Err(CommandError::Message(
        "The save backup helper is missing from this build.".into(),
    ))
}

/// What `fetch-ludusavi.mjs` names the sidecar before Tauri renames it during bundling:
/// `ludusavi-<triple>` plus the platform's executable suffix.
fn sidecar_file_name() -> String {
    let triple = if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    format!("ludusavi-{triple}{suffix}")
}

/// The user's home directory, which the portable redirect maps onto a synthetic path.
fn home() -> Option<PathBuf> {
    #[cfg(windows)]
    let candidate = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let candidate = std::env::var_os("HOME");

    candidate
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Everything one game's sync needs, resolved once.
struct GameContext {
    game_id: i64,
    title: String,
    steam_app_id: Option<u32>,
    /// Where Ludusavi reads and writes this game's backup.
    staging: PathBuf,
    /// Holds every game's staging directory, and the packed archives beside them.
    saves_root: PathBuf,
    install_dir: PathBuf,
    prefix_dir: PathBuf,
    record_saves: LocalSaveState,
    strategy: RestoreStrategy,
    redirects: Vec<Redirect>,
    windows_program: bool,
}

impl GameContext {
    fn platform(&self) -> SavePlatform {
        SavePlatform::for_game(self.windows_program)
    }
}

async fn context(state: &State<'_, AppState>, game_id: i64) -> CommandResult<GameContext> {
    let game = state
        .games()
        .await?
        .into_iter()
        .find(|g| g.id == game_id)
        .ok_or_else(|| CommandError::Message("That game is no longer in the library.".into()))?;

    let root = crate::ipc::root_for_game(state, game_id).await?;
    let layout = InstallLayout::new(&root);
    let record = state.library().record(game_id).await;

    let install_dir = record
        .install_dir
        .clone()
        .unwrap_or_else(|| layout.install_dir(game_id, &game.title));

    let windows_program = record
        .executable
        .as_ref()
        .map(|exe| gameyfin_core::looks_like_windows_program(std::path::Path::new(exe)))
        .unwrap_or(cfg!(windows));

    Ok(GameContext {
        game_id,
        title: game.title.clone(),
        steam_app_id: game.steam_app_id(),
        staging: layout.saves_dir(game_id),
        saves_root: layout.saves_root(),
        install_dir,
        prefix_dir: layout.prefix_dir(game_id),
        record_saves: record.saves.clone(),
        strategy: record.save_restore_strategy.into(),
        redirects: record
            .save_redirects
            .iter()
            .map(|(source, target)| Redirect {
                kind: RedirectKind::Bidirectional,
                source: source.clone(),
                target: target.clone(),
            })
            .collect(),
        windows_program,
    })
}

/// Writes the Ludusavi config for this game and returns a driver bound to it.
/// A save-helper run, holding the shared-config lock until it is dropped.
///
/// One config directory is shared by every game, because each carries a copy of the ~17 MB
/// manifest. The config file is therefore rewritten per game, and the lock is what stops a
/// second game rewriting it underneath a run already in progress.
struct LudusaviSession {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    tool: Ludusavi,
}

impl std::ops::Deref for LudusaviSession {
    type Target = Ludusavi;

    fn deref(&self) -> &Ludusavi {
        &self.tool
    }
}

/// Where the helper keeps its settings and the downloaded game database.
fn ludusavi_config_dir(app_config: &std::path::Path) -> PathBuf {
    app_config.join("ludusavi")
}

/// The helper to run, preferring a version the user installed over the bundled one, which
/// is read-only in a Flatpak and on a system install and so can never be updated in place.
fn ludusavi_binary_for(app: &AppHandle, app_config: &std::path::Path) -> CommandResult<PathBuf> {
    match gameyfin_core::save_tool::installed(app_config) {
        Some(installed) => Ok(installed.binary),
        None => ludusavi_binary(app),
    }
}

async fn ludusavi_for(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
) -> CommandResult<LudusaviSession> {
    let guard = state.ludusavi_lock().lock_owned().await;
    let app_config = state.config_dir().await;
    let binary = ludusavi_binary_for(app, &app_config)?;
    let config_dir = ludusavi_config_dir(&app_config);

    let mut builder = ConfigBuilder::new(&context.staging)
        .strategy(context.strategy)
        .wine_prefix(&context.prefix_dir)
        .manual_redirects(context.redirects.clone())
        .portable_install_dir(&context.install_dir)
        // Registered under the game's own title, which is also what a hand-set path makes
        // the game findable as.
        .custom_save_paths(
            context.title.clone(),
            context.record_saves.custom_paths.clone(),
        );

    // The home a save is recorded against decides whether it travels. For a Windows game
    // running under Proton that is the profile inside the prefix, not this machine's:
    // pointing both at the same synthetic target is what lets a save made on Windows
    // restore on Linux and the other way round.
    let portable_home = if context.windows_program && !cfg!(windows) {
        gameyfin_core::prefix::prefix_home(&context.prefix_dir).or_else(home)
    } else {
        home()
    };
    if let Some(home) = portable_home {
        builder = builder.portable_home(&home);
    }

    // Cross-OS translation needs a single preferred prefix, and Ludusavi only takes one
    // from a custom game entry.
    if context.strategy == RestoreStrategy::CrossOs {
        builder = builder.preferred_wine_prefix(&context.title, &context.prefix_dir);
    }

    builder
        .write(&config_dir)
        .await
        .map_err(|e| CommandError::Message(format!("could not configure save backup: {e}")))?;

    Ok(LudusaviSession {
        _guard: guard,
        tool: Ludusavi::new(binary, config_dir),
    })
}

/// Resolves the title Ludusavi knows this game by, caching it on the record.
/// What identifying a game against the Ludusavi manifest produced.
enum Resolution {
    /// The title to back up under.
    Title(String),
    /// Near misses only. The user has to choose.
    Candidates(Vec<String>),
    /// The manifest has nothing resembling this game.
    Unknown,
}

/// Identifies a game, running the search at most once and remembering the answer.
///
/// The search is four subprocesses at worst. It used to run twice per state check, because
/// the title and the near misses were fetched separately, and an unrecognised game repeated
/// that on every refresh for every game in the library.
async fn resolve_saves(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
) -> CommandResult<Resolution> {
    if let Some(title) = &context.record_saves.ludusavi_title {
        return Ok(Resolution::Title(title.clone()));
    }
    // Paths set by hand are registered as a custom game under this exact title, so there
    // is nothing left to identify: the user has already said what and where.
    if !context.record_saves.custom_paths.is_empty() {
        return Ok(Resolution::Title(context.title.clone()));
    }
    if context.record_saves.match_attempted {
        let cached = context.record_saves.match_candidates.clone();
        return Ok(if cached.is_empty() {
            Resolution::Unknown
        } else {
            Resolution::Candidates(cached)
        });
    }

    let ludusavi = ludusavi_for(app, state, context).await?;
    let identity = GameIdentity {
        title: context.title.clone(),
        steam_app_id: context.steam_app_id,
        gog_id: None,
    };

    let resolved = gameyfin_saves::resolve(&ludusavi, &identity)
        .await
        .map_err(|e| CommandError::Message(format!("could not identify this game: {e}")))?;

    // A fuzzy match is never accepted without the user confirming it, so only a certain one
    // becomes the title; the rest are remembered as candidates to offer them.
    let resolution = match resolved {
        TitleMatch::Certain(title) => Resolution::Title(title),
        TitleMatch::Ambiguous(found) => {
            Resolution::Candidates(found.into_iter().map(|c| c.title).collect())
        }
        TitleMatch::None => Resolution::Unknown,
    };

    let (title, candidates) = match &resolution {
        Resolution::Title(title) => (Some(title.clone()), Vec::new()),
        Resolution::Candidates(found) => (None, found.clone()),
        Resolution::Unknown => (None, Vec::new()),
    };
    tracing::info!(
        game_id = context.game_id,
        title = ?title,
        candidates = candidates.len(),
        "identified game against the save manifest"
    );

    state
        .library()
        .update_record(context.game_id, move |record| {
            record.saves.ludusavi_title = title;
            record.saves.match_candidates = candidates;
            record.saves.match_attempted = true;
        })
        .await;

    Ok(resolution)
}

/// Builds the store the user chose, or explains what is missing.
///
/// The choice is explicit rather than inferred from what happens to be reachable, so it is
/// always clear which one is in use.
async fn store_for(
    state: &State<'_, AppState>,
    settings: &crate::settings::Settings,
) -> CommandResult<Box<dyn SaveStore>> {
    store_of(state, settings, settings.save_backend).await
}

/// A store for one specific backend, which need not be the active one: the settings for
/// all three are kept side by side, so a migration can still reach the previous one.
async fn store_of(
    state: &State<'_, AppState>,
    settings: &crate::settings::Settings,
    backend: SaveBackend,
) -> CommandResult<Box<dyn SaveStore>> {
    let versions = settings.save_max_versions.max(1) as usize;

    match backend {
        SaveBackend::Server => {
            let client = state.client().await.ok_or(CommandError::NotConnected)?;
            Ok(Box::new(ServerStore::new(client)))
        }
        SaveBackend::Folder => {
            let folder = settings.save_folder.as_deref().unwrap_or("").trim();
            if folder.is_empty() {
                return Err(CommandError::Message(
                    "No save folder is set. Choose one under Settings, Saves.".into(),
                ));
            }
            Ok(Box::new(FolderStore::new(folder, versions)))
        }
        SaveBackend::WebDav => {
            let url = settings.webdav_url.as_deref().unwrap_or("").trim();
            if url.is_empty() {
                return Err(CommandError::Message(
                    "No WebDAV address is set. Add one under Settings, Saves.".into(),
                ));
            }
            Ok(Box::new(WebDavStore::new(
                url,
                settings.webdav_username.clone(),
                settings.webdav_password.clone(),
                state.transfer_http().await,
                versions,
            )))
        }
    }
}

async fn sync_for(
    state: &State<'_, AppState>,
    context: &GameContext,
    settings: &crate::settings::Settings,
) -> CommandResult<SaveSync> {
    let store = store_for(state, settings).await?;
    Ok(SaveSync::new(store, context.staging.clone())
        .identified_as(settings.installation_id.clone(), hostname()))
}

/// A human-readable name for this machine, shown in the save history.
///
/// `HOSTNAME` is a shell variable on Linux, not something an application launched from a
/// desktop entry inherits, so `/etc/hostname` is the reliable source there. Without this
/// every Linux save was attributed to "another PC" in the conflict dialog.
fn hostname() -> Option<String> {
    if let Some(name) = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
    {
        return Some(name.trim().to_string());
    }

    #[cfg(not(windows))]
    if let Ok(text) = std::fs::read_to_string("/etc/hostname") {
        let name = text.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    None
}

/// What a backup scan actually captured.
///
/// Reducing this to a bool was why a failed backup could only say "not backed up yet": the
/// difference between "the helper found nothing" and "nothing was ever tried" existed for
/// one line and was then thrown away.
#[derive(Debug, Clone, Copy)]
struct ScanSummary {
    /// Whether the requested title appeared in the reply at all.
    matched: bool,
    /// Files captured, ignoring the ones Ludusavi skipped or failed on.
    files: usize,
    bytes: u64,
}

/// Runs Ludusavi into the staging directory.
///
/// Packing and hashing happen at upload time, so there is exactly one place that decides
/// what bytes the server sees.
async fn back_up(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
    title: &str,
) -> CommandResult<ScanSummary> {
    let ludusavi = ludusavi_for(app, state, context).await?;

    tokio::fs::create_dir_all(&context.staging)
        .await
        .map_err(|e| CommandError::Message(format!("could not create the save folder: {e}")))?;

    let output = ludusavi
        .backup(title, &context.staging, BackupFormat::Zip)
        .await
        .map_err(|e| CommandError::Message(format!("save backup failed: {e}")))?;

    // The requested title specifically, not whatever else the reply mentions.
    let scanned = output.games.get(title);
    let summary = ScanSummary {
        matched: scanned.is_some(),
        files: scanned
            .map(|game| {
                game.files
                    .values()
                    .filter(|file| !file.failed && !file.ignored)
                    .count()
            })
            .unwrap_or(0),
        bytes: scanned.map(|game| game.bytes()).unwrap_or(0),
    };

    tracing::info!(
        game_id = context.game_id,
        title,
        matched = summary.matched,
        files = summary.files,
        bytes = summary.bytes,
        "save backup scan finished"
    );

    Ok(summary)
}

async fn restore(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
    title: &str,
) -> CommandResult<()> {
    let ludusavi = ludusavi_for(app, state, context).await?;
    ludusavi
        .restore(title, &context.staging)
        .await
        .map_err(|e| CommandError::Message(format!("restoring the save failed: {e}")))?;
    Ok(())
}

async fn record_sync(
    state: &State<'_, AppState>,
    game_id: i64,
    save_id: Option<String>,
    hash: Option<String>,
    platform: SavePlatform,
) {
    let now = crate::ipc::now_iso8601();
    state
        .library()
        .update_record(game_id, move |record| {
            if let Some(id) = save_id {
                record.saves.last_synced_save_id = Some(id);
            }
            if hash.is_some() {
                record.saves.last_backup_hash = hash;
                record.saves.last_backup_at = Some(now);
            }
            record.saves.platform = Some(platform);
        })
        .await;
}

/// Works out where one game stands without changing anything.
pub async fn state_of(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
) -> CommandResult<SaveSyncState> {
    let settings = state.settings().await;
    if !settings.save_sync_enabled {
        return Ok(SaveSyncState::Unsupported);
    }

    let context = context(state, game_id).await?;
    match resolve_saves(app, state, &context).await? {
        Resolution::Title(_) => {}
        Resolution::Candidates(candidates) => return Ok(SaveSyncState::Unmatched { candidates }),
        Resolution::Unknown => {
            return Ok(SaveSyncState::Unmatched {
                candidates: Vec::new(),
            })
        }
    }

    let sync = sync_for(state, &context, &settings).await?;
    let remote = match sync.newest_remote(game_id).await {
        Ok(remote) => remote,
        // Both mean the server cannot store saves; the UI tells them apart so it can
        // advise either "ask your admin" or "set up a cloud folder instead".
        Err(gameyfin_api::ApiError::SaveSyncDisabled) => return Ok(SaveSyncState::Disabled),
        Err(gameyfin_api::ApiError::SaveSyncUnsupported) => return Ok(SaveSyncState::Unsupported),
        Err(e) => return Err(e.into()),
    };

    // A staged archive whose hash differs from the last synced one means this machine
    // played since, without needing a fresh backup to find out.
    let staged = save_sync::staged_hash(&context.saves_root, game_id)
        .await
        .unwrap_or(None);
    let local_changed = match (&staged, &context.record_saves.last_backup_hash) {
        (Some(current), Some(synced)) => current != synced,
        (Some(_), None) => true,
        _ => false,
    };

    Ok(save_sync::decide(
        &context.record_saves,
        remote.as_ref(),
        local_changed,
        context.platform(),
    ))
}

// --- Commands ------------------------------------------------------------------------

#[tauri::command]
pub async fn save_state(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<SaveSyncState> {
    state_of(&app, &state, game_id).await
}

#[tauri::command]
pub async fn list_save_versions(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<Vec<SaveVersion>> {
    let settings = state.settings().await;
    let store = store_for(&state, &settings).await?;
    Ok(store.list(game_id).await?)
}

/// Backs up and uploads. Returns the resulting state, including a conflict.
#[tauri::command]
pub async fn backup_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    force: bool,
) -> CommandResult<SaveSyncState> {
    do_backup(&app, &state, game_id, force)
        .await
        .inspect_err(|e| tracing::error!(game_id, error = %e, "backing up saves failed"))
}

async fn do_backup(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
    force: bool,
) -> CommandResult<SaveSyncState> {
    ensure_installation_id(state).await?;

    let settings = state.settings().await;
    let context = context(state, game_id).await?;
    let title = match resolve_saves(app, state, &context).await? {
        Resolution::Title(title) => title,
        Resolution::Candidates(candidates) => return Ok(SaveSyncState::Unmatched { candidates }),
        Resolution::Unknown => {
            return Ok(SaveSyncState::Unmatched {
                candidates: Vec::new(),
            })
        }
    };

    let scan = back_up(app, state, &context, &title).await?;
    if scan.files == 0 {
        // Not an error, but not silence either: the user pressed a button and deserves to
        // know the helper looked and found nothing.
        tracing::warn!(
            game_id,
            title,
            matched = scan.matched,
            "no save files were captured; nothing to upload"
        );
        let next = SaveSyncState::NothingToBackUp;
        emit_state(app, game_id, &next);
        return Ok(next);
    }

    let sync = sync_for(state, &context, &settings).await?;
    let base = context.record_saves.last_synced_save_id.clone();

    let outcome = sync
        .upload(
            game_id,
            base,
            context.platform(),
            Some(title.clone()),
            force,
        )
        .await?;

    let hash = save_sync::staged_hash(&context.saves_root, game_id)
        .await
        .unwrap_or(None);

    let next = match outcome {
        UploadOutcome::Stored(version) => {
            record_sync(
                state,
                game_id,
                Some(version.id.clone()),
                hash,
                context.platform(),
            )
            .await;
            SaveSyncState::InSync {
                last_synced_at: version.created_at.clone(),
            }
        }
        UploadOutcome::Unchanged => {
            record_sync(state, game_id, None, hash, context.platform()).await;
            SaveSyncState::InSync {
                last_synced_at: context.record_saves.last_backup_at.clone(),
            }
        }
        UploadOutcome::Conflict { remote, .. } => SaveSyncState::Conflict {
            local_at: Some(crate::ipc::now_iso8601()),
            remote,
        },
    };

    emit_state(app, game_id, &next);
    Ok(next)
}

/// Fetches a version and restores it over the local saves.
#[tauri::command]
pub async fn restore_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    save_id: Option<String>,
) -> CommandResult<SaveSyncState> {
    do_restore(&app, &state, game_id, save_id)
        .await
        .inspect_err(|e| tracing::error!(game_id, error = %e, "restoring saves failed"))
}

async fn do_restore(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
    save_id: Option<String>,
) -> CommandResult<SaveSyncState> {
    let settings = state.settings().await;
    let context = context(state, game_id).await?;
    let title = match resolve_saves(app, state, &context).await? {
        Resolution::Title(title) => title,
        Resolution::Candidates(candidates) => return Ok(SaveSyncState::Unmatched { candidates }),
        Resolution::Unknown => {
            return Ok(SaveSyncState::Unmatched {
                candidates: Vec::new(),
            })
        }
    };

    let sync = sync_for(state, &context, &settings).await?;

    let version = match save_id {
        Some(id) => sync
            .versions(game_id)
            .await?
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| CommandError::Message("That save version is gone.".into()))?,
        None => sync
            .newest_remote(game_id)
            .await?
            .ok_or_else(|| CommandError::Message("There is no save on the server yet.".into()))?,
    };

    sync.fetch(game_id, &version.id).await?;
    restore(app, state, &context, &title).await?;

    let hash = save_sync::staged_hash(&context.saves_root, game_id)
        .await
        .unwrap_or(None);
    record_sync(
        state,
        game_id,
        Some(version.id.clone()),
        hash,
        context.platform(),
    )
    .await;

    let next = SaveSyncState::InSync {
        last_synced_at: version.created_at.clone(),
    };
    emit_state(app, game_id, &next);
    Ok(next)
}

/// Applies the user's answer to a conflict.
#[tauri::command]
pub async fn resolve_save_conflict(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    choice: ConflictChoice,
) -> CommandResult<SaveSyncState> {
    match choice {
        // Both upload with force. Keeping the remote version in the history is what the
        // server does anyway, so "keep both" differs only in what the user was promised.
        ConflictChoice::KeepLocal | ConflictChoice::KeepBoth => {
            do_backup(&app, &state, game_id, true).await
        }
        ConflictChoice::KeepRemote => do_restore(&app, &state, game_id, None).await,
    }
}

/// Searches the save manifest for a title the user typed.
///
/// The automatic search gives up rather than guess; this is how the user takes over when
/// their game is listed under a name nobody would predict.
#[tauri::command]
pub async fn search_save_titles(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    query: String,
) -> CommandResult<Vec<String>> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let context = context(&state, game_id).await?;
    let ludusavi = ludusavi_for(&app, &state, &context).await?;

    let found = ludusavi
        .find(&gameyfin_saves::GameQuery::Title {
            title: query,
            normalized: false,
            fuzzy: true,
        })
        .await
        .map_err(|e| CommandError::Message(format!("could not search for that game: {e}")))?;

    // Best score first, so the likeliest answer is the one under the cursor.
    let mut matches: Vec<(String, f64)> = found
        .games
        .into_iter()
        .map(|(title, game)| (title, game.score.unwrap_or(0.0)))
        .collect();
    matches.sort_by(|a, b| b.1.total_cmp(&a.1));

    Ok(matches.into_iter().map(|(title, _)| title).collect())
}

/// Records the title a user picked for a game Ludusavi could not identify.
#[tauri::command]
pub async fn set_save_title(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    title: Option<String>,
) -> CommandResult<SaveSyncState> {
    state
        .library()
        .update_record(game_id, move |record| {
            let clearing = title.is_none();
            record.saves.ludusavi_title = title;
            // Clearing the title asks for another look, so the remembered answer goes too;
            // setting one makes the remembered near misses irrelevant.
            record.saves.match_attempted = !clearing;
            record.saves.match_candidates = Vec::new();
        })
        .await;

    state_of(&app, &state, game_id).await
}

/// Records how a game's saves should be mapped onto this machine.
#[tauri::command]
pub async fn set_save_mapping(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    cross_os: bool,
    redirects: Vec<(String, String)>,
    custom_paths: Vec<String>,
) -> CommandResult<SaveSyncState> {
    use crate::library_state::SaveRestoreStrategy;

    let paths: Vec<String> = custom_paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let named = !paths.is_empty();

    state
        .library()
        .update_record(game_id, move |record| {
            record.save_restore_strategy = if cross_os {
                SaveRestoreStrategy::CrossOs
            } else {
                SaveRestoreStrategy::Portable
            };
            record.save_redirects = redirects;
            record.saves.custom_paths = paths;
        })
        .await;

    // Naming a folder answers the identification question, so a game that was written off
    // as unrecognised gets another go rather than staying stuck on the cached verdict.
    if named {
        state
            .library()
            .update_record(game_id, |record| {
                record.saves.match_attempted = false;
                record.saves.match_candidates.clear();
            })
            .await;
    }

    state_of(&app, &state, game_id).await
}

/// Turn Ludusavi's Windows/Linux translation on or off for one game.
///
/// Its own command because the toggle used to go through `set_save_mapping`, which writes
/// the whole set: turning translation on with the empty lists that call passed erased any
/// paths the user had entered.
#[tauri::command]
pub async fn set_save_cross_os(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    cross_os: bool,
) -> CommandResult<SaveSyncState> {
    use crate::library_state::SaveRestoreStrategy;

    state
        .library()
        .update_record(game_id, move |record| {
            record.save_restore_strategy = if cross_os {
                SaveRestoreStrategy::CrossOs
            } else {
                SaveRestoreStrategy::Portable
            };
        })
        .await;

    state_of(&app, &state, game_id).await
}

/// This game's hand-set paths, so the dialog opens on what is actually configured.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavePathSettings {
    custom_paths: Vec<String>,
    redirects: Vec<(String, String)>,
    cross_os: bool,
}

#[tauri::command]
pub async fn save_paths(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<SavePathSettings> {
    use crate::library_state::SaveRestoreStrategy;

    let record = state.library().record(game_id).await;
    Ok(SavePathSettings {
        custom_paths: record.saves.custom_paths.clone(),
        redirects: record.save_redirects.clone(),
        cross_os: record.save_restore_strategy == SaveRestoreStrategy::CrossOs,
    })
}

/// Everything the Saves settings screen owns, sent as one payload because the screen is
/// saved as a whole.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSyncSettings {
    pub enabled: bool,
    pub on_launch: bool,
    pub on_exit: bool,
    pub backend: SaveBackend,
    pub folder: Option<String>,
    pub webdav_url: Option<String>,
    pub webdav_username: Option<String>,
    pub webdav_password: Option<String>,
    pub max_versions: u32,
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[tauri::command]
pub async fn set_save_sync_settings(
    state: State<'_, AppState>,
    settings: SaveSyncSettings,
) -> CommandResult<()> {
    let backend = settings.backend;

    crate::ipc::mutate_settings(&state, move |current| {
        current.save_sync_enabled = settings.enabled;
        current.sync_saves_on_launch = settings.on_launch;
        current.sync_saves_on_exit = settings.on_exit;
        current.save_backend = backend;
        current.save_folder = blank_to_none(settings.folder);
        current.webdav_url = blank_to_none(settings.webdav_url);
        current.webdav_username = blank_to_none(settings.webdav_username);
        current.webdav_password = blank_to_none(settings.webdav_password);
        current.save_max_versions = settings.max_versions.max(1);
    })
    .await
}

/// Checks the configured save location answers, so a wrong path or a bad password is caught
/// at setup rather than after a play session.
#[tauri::command]
pub async fn test_save_store(state: State<'_, AppState>) -> CommandResult<String> {
    let settings = state.settings().await;
    let store = store_for(&state, &settings).await?;
    let description = store.describe();

    match store.games().await {
        Ok(games) if settings.save_backend != SaveBackend::Server => Ok(format!(
            "{description} is reachable, holding saves for {} game(s).",
            games.len()
        )),
        // A server has no "which games" route, so reaching it at all is the check.
        Ok(_) => Ok(format!("{description} is reachable.")),
        Err(gameyfin_api::ApiError::SaveSyncUnsupported) => Err(CommandError::Message(
            "That server does not support save sync. Use a folder or WebDAV instead.".into(),
        )),
        Err(gameyfin_api::ApiError::SaveSyncDisabled) => Err(CommandError::Message(
            "Save sync is turned off on that server. An administrator can enable it.".into(),
        )),
        Err(e) if e.is_auth() => Err(CommandError::Message(
            "Those credentials were refused.".into(),
        )),
        Err(e) => Err(CommandError::Message(format!("Could not reach it: {e}"))),
    }
}

#[tauri::command]
pub async fn delete_save_version(
    state: State<'_, AppState>,
    game_id: i64,
    save_id: String,
) -> CommandResult<()> {
    let settings = state.settings().await;
    let store = store_for(&state, &settings).await?;
    store.delete(game_id, &save_id).await?;
    Ok(())
}

/// Marks a version exempt from retention pruning, or lifts that.
#[tauri::command]
pub async fn set_save_locked(
    state: State<'_, AppState>,
    game_id: i64,
    save_id: String,
    locked: bool,
) -> CommandResult<()> {
    let settings = state.settings().await;
    let store = store_for(&state, &settings).await?;
    store.set_locked(game_id, &save_id, locked).await?;
    Ok(())
}

// --- Moving between backends ---------------------------------------------------------

/// How far a migration has got, so a long copy is not a frozen screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MigrationProgress {
    done: usize,
    total: usize,
}

/// Copy saves from another backend into the active one.
///
/// The source is never touched, so this can be rerun, and running it after a failure
/// resumes rather than duplicating: anything already at the destination is skipped.
#[tauri::command]
pub async fn migrate_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    from: SaveBackend,
    to: SaveBackend,
    all_versions: bool,
) -> CommandResult<gameyfin_core::save_migration::MigrationSummary> {
    let settings = state.settings().await;
    if from == to {
        return Err(CommandError::Message(
            "Pick two different places, one to copy from and one to copy to.".into(),
        ));
    }

    let source = store_of(&state, &settings, from).await?;
    let destination = store_of(&state, &settings, to).await?;
    let scratch = state.config_dir().await.join("migration");

    tracing::info!(
        from = source.describe(),
        to = destination.describe(),
        all_versions,
        "migrating saves"
    );

    // Progress crosses threads, so it is pumped through a channel rather than emitted from
    // inside the callback.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let pump = {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((done, total)) = rx.recv().await {
                let _ = app.emit("save-migration-progress", MigrationProgress { done, total });
            }
        })
    };

    let summary = gameyfin_core::save_migration::migrate(
        source.as_ref(),
        destination.as_ref(),
        &scratch,
        all_versions,
        |done, total| {
            let _ = tx.send((done, total));
        },
    )
    .await;

    drop(tx);
    let _ = pump.await;

    let summary = summary.map_err(|e| CommandError::Message(format!("Migration failed: {e}")))?;
    tracing::info!(
        games = summary.games,
        copied = summary.copied,
        skipped = summary.skipped,
        failed = summary.failed,
        "migration finished"
    );
    Ok(summary)
}

// --- The backup helper itself --------------------------------------------------------

/// Download progress, in the shape the settings screen expects.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolProgress {
    received_bytes: u64,
    total_bytes: u64,
    bytes_per_second: f64,
}

/// The version bundled with this build, read from the sidecar rather than assumed.
async fn bundled_version(app: &AppHandle) -> Option<String> {
    let binary = ludusavi_binary(app).ok()?;
    let mut command = tokio::process::Command::new(&binary);
    command.arg("--version");

    // Without this the settings screen flashes a console window every time it opens.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command.output().await.ok()?;

    // "ludusavi 0.31.0"; the releases are tagged with a leading v.
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.split_whitespace().nth(1)?.trim().to_string();
    Some(if version.starts_with('v') {
        version
    } else {
        format!("v{version}")
    })
}

#[tauri::command]
pub async fn save_tool_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<gameyfin_core::save_tool::SaveToolStatus> {
    let config_dir = state.config_dir().await;
    let installed = gameyfin_core::save_tool::installed(&config_dir);
    let bundled = bundled_version(&app).await;

    // An unreachable feed must not stop the screen showing what is already installed.
    let releases =
        match gameyfin_core::save_tool::releases(&state.http().await, crate::ipc::RELEASE_CHOICES)
            .await
        {
            Ok(releases) => releases,
            Err(e) => {
                tracing::warn!(error = %e, "could not check for a save helper update");
                Vec::new()
            }
        };

    Ok(gameyfin_core::save_tool::SaveToolStatus {
        manifest: gameyfin_core::save_tool::manifest_info(&ludusavi_config_dir(&config_dir)),
        installed,
        bundled,
        available: releases.iter().map(|r| r.version.clone()).collect(),
        latest: releases.into_iter().next(),
    })
}

/// Refresh the game database that says where each game keeps its saves.
///
/// Worth its own button: the database changes far more often than the helper, and a game
/// the helper does not recognise is usually waiting for exactly this rather than for a new
/// release.
#[tauri::command]
pub async fn update_save_manifest(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<gameyfin_core::save_tool::ManifestInfo> {
    let app_config = state.config_dir().await;
    let config_dir = ludusavi_config_dir(&app_config);
    tokio::fs::create_dir_all(&config_dir)
        .await
        .map_err(|e| CommandError::Message(format!("could not prepare the helper: {e}")))?;

    // Serialised with every other run: two helpers on one config directory deadlock.
    let _guard = state.ludusavi_lock().lock_owned().await;
    let ludusavi = Ludusavi::new(ludusavi_binary_for(&app, &app_config)?, &config_dir);

    // Forced, because a user pressing the button is asking about right now, and Ludusavi
    // otherwise skips any check made in the last 24 hours.
    ludusavi.update_manifest(true).await.map_err(|e| {
        tracing::error!(error = %e, "could not update the save database");
        CommandError::Message(format!("Could not update the game database: {e}"))
    })?;

    let info = gameyfin_core::save_tool::manifest_info(&config_dir).ok_or_else(|| {
        CommandError::Message("The game database is still missing after the update.".into())
    })?;
    tracing::info!(bytes = info.bytes, "save database updated");
    let _ = app.emit("save-tool-changed", ());
    Ok(info)
}

/// Installs a version of the backup helper, or the newest when none is named.
#[tauri::command]
pub async fn install_save_tool(
    app: AppHandle,
    state: State<'_, AppState>,
    version: Option<String>,
) -> CommandResult<gameyfin_core::save_tool::InstalledSaveTool> {
    let config_dir = state.config_dir().await;
    let releases =
        gameyfin_core::save_tool::releases(&state.http().await, crate::ipc::RELEASE_CHOICES)
            .await
            .map_err(|e| {
                CommandError::Message(format!("could not look up the save helper: {e}"))
            })?;

    let release = match &version {
        Some(wanted) => releases
            .into_iter()
            .find(|r| &r.version == wanted)
            .ok_or_else(|| {
                CommandError::Message(format!("no such save helper version: {wanted}"))
            })?,
        None => releases
            .into_iter()
            .next()
            .ok_or_else(|| CommandError::Message("no save helper release was found".into()))?,
    };

    tracing::info!(
        version = %release.version,
        size_bytes = release.size_bytes,
        verified = release.sha256.is_some(),
        "installing the save helper"
    );

    // The transfer pool, not the general one: that carries a whole-request timeout a
    // multi-megabyte download would trip.
    let downloader = gameyfin_core::Downloader::new(state.transfer_http().await);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let pump = {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            while let Some((received, total, rate)) = rx.recv().await {
                if last.elapsed() >= std::time::Duration::from_millis(200) {
                    last = std::time::Instant::now();
                    let _ = app.emit(
                        "save-tool-progress",
                        ToolProgress {
                            received_bytes: received,
                            total_bytes: total,
                            bytes_per_second: rate,
                        },
                    );
                }
            }
        })
    };

    let installed = gameyfin_core::save_tool::install(&config_dir, &release, &downloader, |p| {
        let _ = tx.send((
            p.received_bytes,
            p.total_bytes.unwrap_or(0),
            p.bytes_per_second,
        ));
    })
    .await;

    drop(tx);
    let _ = pump.await;

    let installed =
        installed.map_err(|e| CommandError::Message(format!("could not install it: {e}")))?;
    let _ = app.emit("save-tool-changed", ());
    Ok(installed)
}

/// Drops a user-installed helper, falling back to the bundled one.
#[tauri::command]
pub async fn remove_save_tool(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    gameyfin_core::save_tool::remove(&state.config_dir().await)
        .await
        .map_err(|e| CommandError::Message(format!("could not remove it: {e}")))?;
    let _ = app.emit("save-tool-changed", ());
    Ok(())
}

// --- Launch hooks --------------------------------------------------------------------

/// Restores a newer save before the game starts, if there is one and it is safe.
///
/// Never fails a launch: a save that could not be restored is worth a log line and a
/// state the UI can show, not a game that refuses to start.
pub async fn before_launch(app: &AppHandle, state: &State<'_, AppState>, game_id: i64) {
    let settings = state.settings().await;
    if !settings.save_sync_enabled || !settings.sync_saves_on_launch {
        return;
    }

    match state_of(app, state, game_id).await {
        Ok(SaveSyncState::RemoteNewer { .. }) => {
            if let Err(e) = do_restore(app, state, game_id, None).await {
                tracing::warn!(game_id, error = %e, "could not restore the save before launch");
            }
        }
        Ok(other) => {
            if save_sync::needs_attention(&other) {
                tracing::info!(game_id, "save needs a decision before it can be restored");
                emit_state(app, game_id, &other);
            }
        }
        Err(e) => tracing::warn!(game_id, error = %e, "could not check saves before launch"),
    }
}

/// Backs up and uploads after the game exits.
pub async fn after_exit(app: &AppHandle, state: &State<'_, AppState>, game_id: i64) {
    let settings = state.settings().await;
    if !settings.save_sync_enabled || !settings.sync_saves_on_exit {
        return;
    }

    match do_backup(app, state, game_id, false).await {
        Ok(state) => {
            if save_sync::needs_attention(&state) {
                tracing::info!(game_id, "save upload needs a decision");
            }
        }
        Err(e) => tracing::warn!(game_id, error = %e, "could not back up the save after playing"),
    }
}

/// Generates this installation's id on first use, so the server can name the device.
pub async fn ensure_installation_id(state: &State<'_, AppState>) -> CommandResult<()> {
    if state.settings().await.installation_id.is_some() {
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    crate::ipc::mutate_settings(state, move |settings| settings.installation_id = Some(id)).await
}
