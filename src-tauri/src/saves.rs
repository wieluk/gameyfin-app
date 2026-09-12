//! Save synchronisation: Ludusavi locally, a store (server, folder or WebDAV) remotely. The
//! decisions live in `gameyfin_core::save_sync`; this resolves paths and runs the helper.

use std::path::{Path, PathBuf};

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

use crate::error::{CommandError, CommandResult, Context};
use crate::library_state::{GameRecord, SaveRestoreStrategy};
use crate::settings::{SaveBackend, Settings};
use crate::state::AppState;

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

/// The home a portable save is recorded against.
fn home() -> Option<PathBuf> {
    crate::integrations::home().ok()
}

/// Everything one game's sync needs, resolved once.
struct GameContext {
    game_id: i64,
    title: String,
    steam_app_id: Option<u32>,
    /// Where Ludusavi reads and writes this game's backup.
    staging: PathBuf,
    /// Holds every game's staging directory and the packed archives beside them.
    saves_root: PathBuf,
    install_dir: PathBuf,
    prefix_dir: PathBuf,
    record_saves: LocalSaveState,
    strategy: RestoreStrategy,
    redirects: Vec<Redirect>,
    /// Whether this game runs as a Windows program here. `None` when nothing on disk says.
    runs_as_windows: Option<bool>,
}

impl GameContext {
    fn platform(&self) -> SavePlatform {
        match self.runs_as_windows {
            Some(windows) => SavePlatform::for_game(windows),
            // Interchangeable with everything, so a game with no files here never reports a
            // mismatch it has no way to be sure about.
            None => SavePlatform::Unknown,
        }
    }

    /// Whether this game's saves live inside a Wine prefix rather than in the Linux home.
    fn saves_in_prefix(&self) -> bool {
        !cfg!(windows) && self.runs_as_windows == Some(true)
    }
}

/// Whether a game runs as a Windows program on this machine, which is what decides where
/// its saves live and how they are tagged.
///
/// Evidence in order of how much it is worth: the file that would be launched, then the
/// name it was recorded under, then a prefix that has booted, then what a previous sync
/// concluded. `None` rather than a guess: tagging a save wrongly is what makes another
/// machine refuse it.
fn runs_as_windows(record: &GameRecord, install_dir: &Path, prefix_dir: &Path) -> Option<bool> {
    // Everything runs as a Windows program on Windows, and `needs_proton` says so too.
    if cfg!(windows) {
        return Some(true);
    }
    if let Some(executable) = &record.executable {
        let absolute = install_dir.join(executable);
        if absolute.is_file() {
            return Some(
                gameyfin_core::needs_proton(&absolute)
                    || gameyfin_core::looks_like_windows_program(&absolute),
            );
        }
        // Gone from disk: the name it was recorded under still says what it was.
        return Some(gameyfin_core::needs_proton(Path::new(executable)));
    }
    // A prefix that has been booted means this game has already run as Windows here.
    if gameyfin_core::prefix::wine_root(prefix_dir)
        .join("drive_c")
        .is_dir()
    {
        return Some(true);
    }
    match record.saves.platform {
        Some(SavePlatform::Windows | SavePlatform::Proton) => Some(true),
        Some(SavePlatform::Linux | SavePlatform::MacOS) => Some(false),
        _ => None,
    }
}

async fn context(state: &AppState, game_id: i64) -> CommandResult<GameContext> {
    let game = state.game(game_id).await?;
    let layout = InstallLayout::new(crate::ipc::root_for_game(state, game_id)?);
    let record = state.library().record(game_id);
    let install_dir = record
        .install_dir
        .clone()
        .unwrap_or_else(|| layout.install_dir(game_id, &game.title));
    let prefix_dir = layout.prefix_dir(game_id);
    let runs_as_windows = runs_as_windows(&record, &install_dir, &prefix_dir);

    Ok(GameContext {
        game_id,
        steam_app_id: game.steam_app_id(),
        staging: layout.saves_dir(game_id),
        saves_root: layout.saves_root(),
        install_dir,
        prefix_dir,
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
        runs_as_windows,
        title: game.title,
    })
}

/// A helper run, holding the shared-config lock. One config directory serves every game,
/// since each copy carries the 17 MB manifest, so its file is rewritten per game.
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

fn ludusavi_config_dir(app_config: &Path) -> PathBuf {
    app_config.join("ludusavi")
}

/// A user-installed helper wins: the bundled one is read-only in a Flatpak or system install.
fn ludusavi_binary(app: &AppHandle, app_config: &Path) -> CommandResult<PathBuf> {
    if let Some(installed) = gameyfin_core::save_tool::installed(app_config) {
        return Ok(installed.binary);
    }
    crate::sidecar::bundled(app, "ludusavi")
        .ok_or_else(|| CommandError::msg("The save backup helper is missing from this build."))
}

async fn ludusavi_for(
    app: &AppHandle,
    state: &AppState,
    context: &GameContext,
) -> CommandResult<LudusaviSession> {
    let guard = state.ludusavi_lock().lock_owned().await;
    let app_config = state.config_dir();
    let binary = ludusavi_binary(app, &app_config)?;
    let settings = state.settings();

    let mut builder = ConfigBuilder::new(&context.staging)
        .strategy(context.strategy)
        .manual_redirects(context.redirects.clone())
        .portable_install_dir(&context.install_dir)
        // Under the game's own title, which is also what a hand-set path registers it as.
        .custom_save_paths(
            context.title.clone(),
            context.record_saves.custom_paths.clone(),
        );

    // Only where the game actually keeps saves: on Windows, and for a native game, the
    // prefix is either absent or irrelevant, and an empty root is one more place to scan.
    let in_prefix = context.saves_in_prefix()
        && gameyfin_core::prefix::wine_root(&context.prefix_dir)
            .join("drive_c")
            .is_dir();
    if in_prefix {
        builder = builder.wine_prefix(&context.prefix_dir);
    }

    // For a Windows game on Proton the home that travels is the one inside the prefix, so
    // pointing both at one synthetic target is what lets a save cross between the two. The
    // machine's own home is registered after it, and only as itself.
    let prefix_home = in_prefix
        .then(|| gameyfin_core::prefix::prefix_home(&context.prefix_dir))
        .flatten();
    match (prefix_home, home()) {
        (Some(prefix_home), host_home) => {
            builder = builder.portable_home(&prefix_home);
            if let Some(host_home) = host_home {
                builder = builder.portable_host_home(&host_home);
            }
        }
        (None, Some(host_home)) => builder = builder.portable_home(&host_home),
        (None, None) => {}
    }
    // Cross-OS translation needs one preferred prefix, which only a custom entry can carry.
    if context.strategy == RestoreStrategy::CrossOs {
        builder = builder.preferred_wine_prefix(&context.title, &context.prefix_dir);
    }

    let config_dir = ludusavi_config_dir(&app_config);
    builder
        .write(&config_dir)
        .await
        .context("could not configure save backup")?;
    Ok(LudusaviSession {
        _guard: guard,
        tool: Ludusavi::new(binary, config_dir)
            .auto_update_manifest(settings.save_manifest_auto_update),
    })
}

/// What identifying a game against the manifest produced.
enum Resolution {
    Title(String),
    /// Near misses only, so the user has to choose.
    Candidates(Vec<String>),
    Unknown,
}

impl Resolution {
    /// The title to back up under, or the state the UI should show instead.
    fn require_title(self) -> Result<String, SaveSyncState> {
        match self {
            Resolution::Title(title) => Ok(title),
            Resolution::Candidates(candidates) => Err(SaveSyncState::Unmatched { candidates }),
            Resolution::Unknown => Err(SaveSyncState::Unmatched {
                candidates: Vec::new(),
            }),
        }
    }
}

/// Identifies a game, running the search at most once and remembering the answer: it costs
/// four subprocesses, and an unrecognised game would repeat that on every refresh.
async fn resolve_saves(
    app: &AppHandle,
    state: &AppState,
    context: &GameContext,
) -> CommandResult<Resolution> {
    if let Some(title) = &context.record_saves.ludusavi_title {
        return Ok(Resolution::Title(title.clone()));
    }
    // Hand-set paths are registered under this exact title, so nothing is left to identify.
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
        .context("could not identify this game")?;

    // A fuzzy match is never accepted without the user, so only a certain one becomes the title.
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
        ?title,
        candidates = candidates.len(),
        "identified game"
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

/// The store the user chose. Explicit rather than inferred, so it is clear which is in use.
async fn store_for(state: &AppState, settings: &Settings) -> CommandResult<Box<dyn SaveStore>> {
    store_of(state, settings, settings.save_backend).await
}

/// Any backend, not only the active one: a migration has to reach the previous one.
async fn store_of(
    state: &AppState,
    settings: &Settings,
    backend: SaveBackend,
) -> CommandResult<Box<dyn SaveStore>> {
    let versions = settings.save_max_versions.max(1) as usize;
    match backend {
        SaveBackend::Server => Ok(Box::new(ServerStore::new(state.require_client()?))),
        SaveBackend::Folder => {
            let folder = settings.save_folder.as_deref().unwrap_or_default().trim();
            if folder.is_empty() {
                return Err(CommandError::msg(
                    "No save folder is set. Choose one under Settings, Saves.",
                ));
            }
            Ok(Box::new(FolderStore::new(folder, versions)))
        }
        SaveBackend::WebDav => {
            let url = settings.webdav_url.as_deref().unwrap_or_default().trim();
            if url.is_empty() {
                return Err(CommandError::msg(
                    "No WebDAV address is set. Add one under Settings, Saves.",
                ));
            }
            Ok(Box::new(WebDavStore::new(
                url,
                settings.webdav_username.clone(),
                settings.webdav_password.clone(),
                state.transfer_http(),
                versions,
            )))
        }
    }
}

async fn sync_for(
    state: &AppState,
    context: &GameContext,
    settings: &Settings,
) -> CommandResult<SaveSync> {
    let store = store_for(state, settings).await?;
    // The saves root, not this game's directory: `SaveSync` appends the game id itself.
    Ok(SaveSync::new(store, context.saves_root.clone())
        .identified_as(settings.installation_id.clone(), device_name(settings)))
}

/// A host name says little about which machine a save came from, so the user's own name wins.
fn device_name(settings: &Settings) -> Option<String> {
    settings
        .device_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(hostname)
}

/// `HOSTNAME` is a shell variable, not something a desktop entry passes on, so Linux reads
/// `/etc/hostname` instead.
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
        return Some(text.trim().to_string()).filter(|name| !name.is_empty());
    }
    None
}

/// What a backup scan captured. "Found nothing" and "never tried" are different answers.
#[derive(Debug, Clone, Copy)]
struct ScanSummary {
    /// Whether the requested title appeared in the reply at all.
    matched: bool,
    files: usize,
}

/// Runs Ludusavi into the staging directory. Packing happens at upload time, so exactly one
/// place decides what bytes the store sees.
async fn back_up(
    app: &AppHandle,
    state: &AppState,
    context: &GameContext,
    title: &str,
) -> CommandResult<ScanSummary> {
    let ludusavi = ludusavi_for(app, state, context).await?;
    tokio::fs::create_dir_all(&context.staging)
        .await
        .context("could not create the save folder")?;
    let output = ludusavi
        .backup(title, &context.staging, BackupFormat::Zip)
        .await
        .context("save backup failed")?;

    let scanned = output.games.get(title);
    let summary = ScanSummary {
        matched: scanned.is_some(),
        files: scanned.map_or(0, |game| {
            game.files
                .values()
                .filter(|f| !f.failed && !f.ignored)
                .count()
        }),
    };
    tracing::info!(
        game_id = context.game_id,
        title,
        ?summary,
        "save backup scan finished"
    );
    Ok(summary)
}

async fn record_sync(
    state: &AppState,
    game_id: i64,
    save_id: Option<String>,
    hash: Option<String>,
    platform: SavePlatform,
) {
    let now = crate::ipc::now_iso8601();
    state
        .library()
        .update_record(game_id, move |record| {
            if save_id.is_some() {
                record.saves.last_synced_save_id = save_id;
            }
            if hash.is_some() {
                record.saves.last_backup_hash = hash;
                record.saves.last_backup_at = Some(now);
            }
            record.saves.platform = Some(platform);
        })
        .await;
}

async fn staged_hash(context: &GameContext) -> Option<String> {
    save_sync::staged_hash(&context.saves_root, context.game_id)
        .await
        .unwrap_or(None)
}

/// Works out where one game stands without changing anything.
pub async fn state_of(
    app: &AppHandle,
    state: &AppState,
    game_id: i64,
) -> CommandResult<SaveSyncState> {
    let settings = state.settings();
    if !settings.save_sync_enabled {
        return Ok(SaveSyncState::Off);
    }
    let context = context(state, game_id).await?;
    if let Err(unmatched) = resolve_saves(app, state, &context).await?.require_title() {
        return Ok(unmatched);
    }

    let sync = sync_for(state, &context, &settings).await?;
    let remote = match sync.newest_remote(game_id).await {
        Ok(remote) => remote,
        // Both mean the server cannot store saves; the UI advises differently for each.
        Err(gameyfin_api::ApiError::SaveSyncDisabled) => return Ok(SaveSyncState::Disabled),
        Err(gameyfin_api::ApiError::SaveSyncUnsupported { .. }) => {
            return Ok(SaveSyncState::Unsupported)
        }
        Err(e) => return Err(e.into()),
    };

    // A staged archive whose hash differs means this machine played since the last sync.
    let staged = staged_hash(&context).await;
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
        settings.installation_id.as_deref(),
    ))
}

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
    let settings = state.settings();
    Ok(store_for(&state, &settings).await?.list(game_id).await?)
}

/// Backs up and uploads. The result includes a conflict, which the UI then resolves.
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
    state: &AppState,
    game_id: i64,
    force: bool,
) -> CommandResult<SaveSyncState> {
    ensure_installation_id(state).await?;
    let settings = state.settings();
    let context = context(state, game_id).await?;
    let title = match resolve_saves(app, state, &context).await?.require_title() {
        Ok(title) => title,
        Err(unmatched) => return Ok(unmatched),
    };

    let scan = back_up(app, state, &context, &title).await?;
    if scan.files == 0 {
        // Not an error, but the user pressed a button and deserves to hear it found nothing.
        tracing::warn!(
            game_id,
            title,
            matched = scan.matched,
            "no save files were captured"
        );
        let next = SaveSyncState::NothingToBackUp {
            title,
            known: scan.matched,
        };
        emit_state(app, game_id, &next);
        return Ok(next);
    }

    let sync = sync_for(state, &context, &settings).await?;
    let outcome = sync
        .upload(
            game_id,
            context.record_saves.last_synced_save_id.clone(),
            context.platform(),
            Some(title),
            force,
        )
        .await?;
    let hash = staged_hash(&context).await;

    let next = match outcome {
        UploadOutcome::Stored(version) => {
            record_sync(state, game_id, Some(version.id), hash, context.platform()).await;
            SaveSyncState::InSync {
                last_synced_at: version.created_at,
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
    state: &AppState,
    game_id: i64,
    save_id: Option<String>,
) -> CommandResult<SaveSyncState> {
    let settings = state.settings();
    let context = context(state, game_id).await?;
    let title = match resolve_saves(app, state, &context).await?.require_title() {
        Ok(title) => title,
        Err(unmatched) => return Ok(unmatched),
    };
    let sync = sync_for(state, &context, &settings).await?;

    let version = match save_id {
        Some(id) => sync
            .versions(game_id)
            .await?
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| CommandError::msg("That save version is gone."))?,
        None => sync
            .newest_remote(game_id)
            .await?
            .ok_or_else(|| CommandError::msg("There is no save on the server yet."))?,
    };
    sync.fetch(game_id, &version.id).await?;

    let ludusavi = ludusavi_for(app, state, &context).await?;
    ludusavi
        .restore(&title, &context.staging)
        .await
        .context("restoring the save failed")?;
    drop(ludusavi);

    let hash = staged_hash(&context).await;
    record_sync(state, game_id, Some(version.id), hash, context.platform()).await;
    let next = SaveSyncState::InSync {
        last_synced_at: version.created_at,
    };
    emit_state(app, game_id, &next);
    Ok(next)
}

#[tauri::command]
pub async fn resolve_save_conflict(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    choice: ConflictChoice,
) -> CommandResult<SaveSyncState> {
    match choice {
        // Keeping both is what the store does anyway: the remote version stays in the history.
        ConflictChoice::KeepLocal | ConflictChoice::KeepBoth => {
            do_backup(&app, &state, game_id, true).await
        }
        ConflictChoice::KeepRemote => do_restore(&app, &state, game_id, None).await,
    }
}

/// Searches the manifest, for a game listed under a name nobody would predict.
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
        .context("could not search for that game")?;

    // Best score first, so the likeliest answer is under the cursor.
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
            // Clearing the title asks for another look; setting one makes near misses moot.
            record.saves.match_attempted = title.is_some();
            record.saves.ludusavi_title = title;
            record.saves.match_candidates = Vec::new();
        })
        .await;
    state_of(&app, &state, game_id).await
}

/// Records how a game's saves map onto this machine.
#[tauri::command]
pub async fn set_save_mapping(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    cross_os: bool,
    redirects: Vec<(String, String)>,
    custom_paths: Vec<String>,
) -> CommandResult<SaveSyncState> {
    let paths: Vec<String> = custom_paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let named = !paths.is_empty();
    state
        .library()
        .update_record(game_id, move |record| {
            record.save_restore_strategy = cross_os.into();
            record.save_redirects = redirects;
            record.saves.custom_paths = paths;
            // Naming a folder answers the identification question, so try again.
            if named {
                record.saves.match_attempted = false;
                record.saves.match_candidates.clear();
            }
        })
        .await;
    state_of(&app, &state, game_id).await
}

/// Its own command because going through `set_save_mapping` erased hand-entered paths.
#[tauri::command]
pub async fn set_save_cross_os(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    cross_os: bool,
) -> CommandResult<SaveSyncState> {
    state
        .library()
        .update_record(game_id, move |record| {
            record.save_restore_strategy = cross_os.into();
        })
        .await;
    state_of(&app, &state, game_id).await
}

/// This game's hand-set paths, so the dialog opens on what is configured.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
    let record = state.library().record(game_id);
    Ok(SavePathSettings {
        custom_paths: record.saves.custom_paths,
        redirects: record.save_redirects,
        cross_os: record.save_restore_strategy == SaveRestoreStrategy::CrossOs,
    })
}

/// The name this machine would use, for the field's placeholder.
#[tauri::command]
pub async fn detected_device_name() -> CommandResult<Option<String>> {
    Ok(hostname())
}

/// Checks the configured store answers, so a wrong path is caught at setup rather than
/// after a play session.
#[tauri::command]
pub async fn test_save_store(state: State<'_, AppState>) -> CommandResult<String> {
    let settings = state.settings();
    let store = store_for(&state, &settings).await?;
    let description = store.describe();
    if settings.save_backend == SaveBackend::Server {
        return test_server_saves(&state, store.as_ref(), &description).await;
    }
    match store.games().await {
        Ok(games) => Ok(format!(
            "{description} is reachable, holding saves for {} game(s).",
            games.len()
        )),
        Err(e) if e.is_auth() => Err(CommandError::msg("Those credentials were refused.")),
        Err(e) => Err(CommandError::msg(format!("Could not reach it: {e}"))),
    }
}

/// There is no capability route, so listing a real game's saves is the probe: the 404 that
/// means "no save sync here" is only distinguishable when the game exists.
async fn test_server_saves(
    state: &AppState,
    store: &dyn SaveStore,
    description: &str,
) -> CommandResult<String> {
    let games = state.games().await?;
    let Some(game) = games.first() else {
        return Err(CommandError::msg(format!(
            "{description} is signed in, but checking needs a game to ask about. Add one and test again."
        )));
    };
    match store.list(game.id).await {
        Ok(_) => Ok(format!("{description} supports save sync.")),
        Err(gameyfin_api::ApiError::SaveSyncUnsupported { endpoint, status }) => {
            Err(CommandError::msg(format!(
                "{description} does not support save sync: {endpoint} returned HTTP {status}. Either the \
                 server predates the feature, or something in front of it answered instead of Gameyfin."
            )))
        }
        Err(gameyfin_api::ApiError::SaveSyncDisabled) => Err(CommandError::msg(format!(
            "{description} has save sync turned off. An administrator can enable it."
        ))),
        Err(e) if e.is_auth() => Err(CommandError::msg(format!(
            "{description} refused the session. Sign in again from Settings."
        ))),
        Err(e) => Err(CommandError::msg(format!("Could not reach it: {e}"))),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MigrationProgress {
    done: usize,
    total: usize,
}

/// Copies saves from another backend into the active one. The source is never touched, and
/// anything already at the destination is skipped, so a failed run can be repeated.
#[tauri::command]
pub async fn migrate_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    from: SaveBackend,
    to: SaveBackend,
    all_versions: bool,
) -> CommandResult<gameyfin_core::save_migration::MigrationSummary> {
    let settings = state.settings();
    if from == to {
        return Err(CommandError::msg(
            "Pick two different places, one to copy from and one to copy to.",
        ));
    }
    let source = store_of(&state, &settings, from).await?;
    let destination = store_of(&state, &settings, to).await?;
    let scratch = state.config_dir().join("migration");
    tracing::info!(
        from = source.describe(),
        to = destination.describe(),
        all_versions,
        "migrating saves"
    );

    let mut throttle = crate::progress::Throttle::new(200);
    let summary = gameyfin_core::save_migration::migrate(
        source.as_ref(),
        destination.as_ref(),
        &scratch,
        all_versions,
        |done, total| {
            if throttle.ready() || done == total {
                let _ = app.emit("save-migration-progress", MigrationProgress { done, total });
            }
        },
    )
    .await
    .context("Migration failed")?;
    tracing::info!(?summary, "migration finished");
    Ok(summary)
}

/// The version bundled with this build, read from the sidecar rather than assumed.
async fn bundled_version(app: &AppHandle) -> Option<String> {
    let binary = crate::sidecar::bundled(app, "ludusavi")?;
    let mut command = tokio::process::Command::new(&binary);
    command.arg("--version");
    // Without this the settings screen flashes a console window every time it opens.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().await.ok()?;
    // "ludusavi 0.31.0", against releases tagged with a leading v.
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
    let config_dir = state.config_dir();
    // An unreachable feed must not stop the screen showing what is installed.
    let releases = gameyfin_core::save_tool::releases(&state.http(), crate::wine::RELEASE_CHOICES)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not check for a save helper update");
            Vec::new()
        });
    Ok(gameyfin_core::save_tool::SaveToolStatus {
        manifest: gameyfin_core::save_tool::manifest_info(&ludusavi_config_dir(&config_dir)),
        installed: gameyfin_core::save_tool::installed(&config_dir),
        bundled: bundled_version(&app).await,
        available: releases.iter().map(|r| r.version.clone()).collect(),
        latest: releases.into_iter().next(),
    })
}

/// Fetches the game database if there is none yet, so the first backup does not wait on 17 MB.
pub async fn ensure_manifest(app: AppHandle) {
    let state = app.state::<AppState>();
    let app_config = state.config_dir();
    let config_dir = ludusavi_config_dir(&app_config);
    if gameyfin_core::save_tool::manifest_info(&config_dir).is_some() {
        return;
    }
    let Ok(binary) = ludusavi_binary(&app, &app_config) else {
        return;
    };
    if tokio::fs::create_dir_all(&config_dir).await.is_err() {
        return;
    }
    let _guard = state.ludusavi_lock().lock_owned().await;
    tracing::info!("downloading the save database for the first time");
    // Unforced: this is the first copy, so there is nothing too recent to replace.
    match Ludusavi::new(binary, &config_dir)
        .update_manifest(false)
        .await
    {
        Ok(()) => {
            let _ = app.emit("save-tool-changed", ());
        }
        // Not fatal: Ludusavi fetches it on first use anyway, just less conveniently.
        Err(e) => tracing::warn!(error = %e, "could not download the save database"),
    }
}

/// The database changes far more often than the helper, so it has its own button.
#[tauri::command]
pub async fn update_save_manifest(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<gameyfin_core::save_tool::ManifestInfo> {
    let app_config = state.config_dir();
    let config_dir = ludusavi_config_dir(&app_config);
    tokio::fs::create_dir_all(&config_dir)
        .await
        .context("could not prepare the helper")?;
    // Serialised with every other run: two helpers on one config directory deadlock.
    let _guard = state.ludusavi_lock().lock_owned().await;
    // Forced, because Ludusavi otherwise skips any check made in the last 24 hours.
    Ludusavi::new(ludusavi_binary(&app, &app_config)?, &config_dir)
        .update_manifest(true)
        .await
        .context("Could not update the game database")?;

    let info = gameyfin_core::save_tool::manifest_info(&config_dir)
        .ok_or_else(|| CommandError::msg("The game database is still missing after the update."))?;
    let _ = app.emit("save-tool-changed", ());
    Ok(info)
}

/// Installs a version of the helper, or the newest when none is named.
#[tauri::command]
pub async fn install_save_tool(
    app: AppHandle,
    state: State<'_, AppState>,
    version: Option<String>,
) -> CommandResult<gameyfin_core::save_tool::InstalledSaveTool> {
    let releases = gameyfin_core::save_tool::releases(&state.http(), crate::wine::RELEASE_CHOICES)
        .await
        .context("could not look up the save helper")?;
    let release = match &version {
        Some(wanted) => releases.into_iter().find(|r| &r.version == wanted),
        None => releases.into_iter().next(),
    }
    .ok_or_else(|| CommandError::msg("That save helper version was not found."))?;

    tracing::info!(version = %release.version, "installing the save helper");
    let downloader = gameyfin_core::Downloader::new(state.transfer_http());
    let installed = gameyfin_core::save_tool::install(
        &state.config_dir(),
        &release,
        &downloader,
        crate::progress::emitter(&app, "save-tool-progress"),
    )
    .await
    .context("could not install it")?;
    let _ = app.emit("save-tool-changed", ());
    Ok(installed)
}

/// Drops a user-installed helper, falling back to the bundled one.
#[tauri::command]
pub async fn remove_save_tool(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    gameyfin_core::save_tool::remove(&state.config_dir())
        .await
        .context("could not remove it")?;
    let _ = app.emit("save-tool-changed", ());
    Ok(())
}

/// The offer to download saves that already exist, made once before a first play.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavePullOffer {
    pub game_id: i64,
    pub title: String,
    pub remote_at: Option<String>,
    pub device: Option<String>,
    pub size_bytes: u64,
}

/// What [`before_launch`] decided the launch should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchGate {
    Proceed,
    /// The dialog starts the game once answered, so launching here would race the restore.
    AwaitingSaveDecision,
}

/// Restores a newer save before the game starts. Never fails a launch: a save that could not
/// be restored is worth a log line, not a game that refuses to start.
pub async fn before_launch(app: &AppHandle, state: &AppState, game_id: i64) -> LaunchGate {
    let settings = state.settings();
    if !settings.save_sync_enabled || !settings.sync_saves_on_launch {
        return LaunchGate::Proceed;
    }
    match state_of(app, state, game_id).await {
        Ok(SaveSyncState::RemoteNewer { remote_at, device }) => {
            let local = state.library().record(game_id).saves;
            // Saves may predate sync on this machine, and restoring over them is silent data
            // loss, so the first time is asked about and a refusal is honoured until the user
            // syncs by hand.
            if save_sync::never_synced(&local) {
                if local.pull_offer_answered {
                    tracing::info!(game_id, "keeping the local saves the user chose to keep");
                    return LaunchGate::Proceed;
                }
                if let Some(offer) = pull_offer(state, game_id, remote_at, device).await {
                    tracing::info!(game_id, "offering to download saves before the first play");
                    let _ = app.emit("save-pull-offer", offer);
                    return LaunchGate::AwaitingSaveDecision;
                }
            }
            if let Err(e) = do_restore(app, state, game_id, None).await {
                tracing::warn!(game_id, error = %e, "could not restore the save before launch");
            }
        }
        Ok(other) if save_sync::needs_attention(&other) => {
            tracing::info!(game_id, "save needs a decision before it can be restored");
            emit_state(app, game_id, &other);
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(game_id, error = %e, "could not check saves before launch"),
    }
    LaunchGate::Proceed
}

async fn pull_offer(
    state: &AppState,
    game_id: i64,
    remote_at: Option<String>,
    device: Option<String>,
) -> Option<SavePullOffer> {
    let settings = state.settings();
    // Best effort: the offer is worth making even when the size is unknown.
    let size_bytes = match store_for(state, &settings).await {
        Ok(store) => store
            .list(game_id)
            .await
            .ok()
            .and_then(|versions| versions.first().map(|v| v.size_bytes))
            .unwrap_or(0),
        Err(_) => 0,
    };
    Some(SavePullOffer {
        game_id,
        title: state.title(game_id).await,
        remote_at,
        device,
        size_bytes,
    })
}

/// Records the answer to the first-play offer. Declining is remembered so it is asked once;
/// a failed download is not, so the question survives to be answered again.
#[tauri::command]
pub async fn answer_save_pull_offer(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    download: bool,
) -> CommandResult<()> {
    if download {
        let restored = do_restore(&app, &state, game_id, None).await?;
        emit_state(&app, game_id, &restored);
    }
    state
        .library()
        .update_record(game_id, |r| r.saves.pull_offer_answered = true)
        .await;
    Ok(())
}

/// Backs up and uploads after the game exits.
pub async fn after_exit(app: &AppHandle, state: &AppState, game_id: i64) {
    let settings = state.settings();
    if !settings.save_sync_enabled || !settings.sync_saves_on_exit {
        return;
    }
    // Sync is on by default and many servers predate it, so one request here spares a
    // backup the upload would only discard.
    if settings.save_backend == SaveBackend::Server {
        if let Ok(store) = store_for(state, &settings).await {
            if matches!(
                store.list(game_id).await,
                Err(gameyfin_api::ApiError::SaveSyncUnsupported { .. }
                    | gameyfin_api::ApiError::SaveSyncDisabled)
            ) {
                return;
            }
        }
    }
    match do_backup(app, state, game_id, false).await {
        Ok(state) if save_sync::needs_attention(&state) => {
            tracing::info!(game_id, "save upload needs a decision");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(game_id, error = %e, "could not back up the save after playing"),
    }
}

/// Generates this installation's id on first use, so the store can name the device.
pub async fn ensure_installation_id(state: &AppState) -> CommandResult<()> {
    if state.settings().installation_id.is_some() {
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    state.set_settings(|s| s.installation_id = Some(id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-saves-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    #[cfg(not(windows))]
    fn a_windows_game_is_recognised_by_the_file_it_launches() {
        // The bug this covers: the executable is stored relative to the install folder, and
        // testing that path directly always failed to open, so every Windows game on Linux
        // was tagged as a native one and its save refused a Windows PC's.
        let dir = scratch("exe");
        let install = dir.join("install");
        std::fs::create_dir_all(install.join("bin")).unwrap();
        std::fs::write(install.join("bin/game.exe"), b"MZ").unwrap();

        let record = GameRecord {
            executable: Some("bin/game.exe".into()),
            ..Default::default()
        };
        assert_eq!(
            Some(true),
            runs_as_windows(&record, &install, &dir.join("prefix"))
        );

        let native = GameRecord {
            executable: Some("bin/game".into()),
            ..Default::default()
        };
        std::fs::write(install.join("bin/game"), b"\x7fELF").unwrap();
        assert_eq!(
            Some(false),
            runs_as_windows(&native, &install, &dir.join("prefix"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(not(windows))]
    fn a_missing_executable_falls_back_to_its_name_then_to_the_prefix() {
        let dir = scratch("fallback");
        let install = dir.join("install");
        let prefix = dir.join("prefix");

        let deleted = GameRecord {
            executable: Some("Game.exe".into()),
            ..Default::default()
        };
        assert_eq!(Some(true), runs_as_windows(&deleted, &install, &prefix));

        // Nothing recorded at all, but the prefix has booted, which only happens for a
        // Windows game.
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        assert_eq!(
            Some(true),
            runs_as_windows(&GameRecord::default(), &install, &prefix)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(not(windows))]
    fn a_game_with_nothing_on_disk_is_not_guessed_at() {
        let dir = scratch("unknown");
        let unknown = runs_as_windows(&GameRecord::default(), &dir.join("install"), &dir.join("p"));
        assert_eq!(None, unknown);

        // And an unknown platform is interchangeable with everything, so such a game never
        // reports a mismatch it cannot be sure about.
        let context_platform = match unknown {
            Some(windows) => SavePlatform::for_game(windows),
            None => SavePlatform::Unknown,
        };
        assert!(context_platform.interchangeable_with(SavePlatform::Windows));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(not(windows))]
    fn a_previous_sync_settles_it_when_the_files_are_gone() {
        let dir = scratch("remembered");
        let mut record = GameRecord::default();
        record.saves.platform = Some(SavePlatform::Proton);
        assert_eq!(
            Some(true),
            runs_as_windows(&record, &dir.join("install"), &dir.join("p"))
        );

        record.saves.platform = Some(SavePlatform::Linux);
        assert_eq!(
            Some(false),
            runs_as_windows(&record, &dir.join("install"), &dir.join("p"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
