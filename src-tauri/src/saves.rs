//! Save synchronisation: Ludusavi locally, a store (server, folder or WebDAV) remotely. The
//! decisions live in `gameyfin_core::save_sync`; this resolves paths and runs the helper.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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

/// Whether a game runs as a Windows program here, deciding where saves live and how they are
/// tagged. `None` rather than a guess: a wrong tag makes another machine refuse the save.
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
    let strategy = effective_strategy(record.save_restore_strategy.into(), runs_as_windows);

    Ok(GameContext {
        game_id,
        steam_app_id: game.steam_app_id(),
        staging: layout.saves_dir(game_id),
        saves_root: layout.saves_root(),
        install_dir,
        prefix_dir,
        record_saves: record.saves.clone(),
        strategy,
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

/// Never cross-OS for a game in a prefix: it drops the portable mapping, so a save stored under
/// `/gameyfin/home` restores to nowhere.
fn effective_strategy(stored: RestoreStrategy, runs_as_windows: Option<bool>) -> RestoreStrategy {
    if !cfg!(windows) && runs_as_windows == Some(true) {
        RestoreStrategy::Portable
    } else {
        stored
    }
}

/// The path as the filesystem resolves it: Ludusavi reports real paths, so a redirect through a
/// symlink (`/home` on ostree) never matches.
fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
        .manual_redirects(
            context
                .redirects
                .iter()
                .map(|redirect| Redirect {
                    source: real(Path::new(&redirect.source))
                        .to_string_lossy()
                        .into_owned(),
                    ..redirect.clone()
                })
                .collect::<Vec<_>>(),
        )
        .portable_install_dir(&real(&context.install_dir))
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
        builder = builder.wine_prefix(&real(&context.prefix_dir));
    }

    // A Proton game's travelling home is the prefix's, so a save crosses to Windows. The machine's
    // own home is registered after it, only as itself.
    let prefix_home = in_prefix
        .then(|| gameyfin_core::prefix::prefix_home(&context.prefix_dir))
        .flatten();
    match (prefix_home, home()) {
        (Some(prefix_home), host_home) => {
            builder = builder.portable_home(&real(&prefix_home));
            if let Some(host_home) = host_home {
                builder = builder.portable_host_home(&real(&host_home));
            }
        }
        (None, Some(host_home)) => builder = builder.portable_home(&real(&host_home)),
        (None, None) => {}
    }
    // Cross-OS translation needs one preferred prefix, which only a custom entry can carry.
    if context.strategy == RestoreStrategy::CrossOs {
        builder = builder.preferred_wine_prefix(&context.title, &real(&context.prefix_dir));
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

/// A game's standing as far as it can be told without knowing what this PC changed.
enum Standing {
    Settled(SaveSyncState),
    Open {
        context: Box<GameContext>,
        title: String,
        remote: Option<SaveVersion>,
    },
}

async fn standing(app: &AppHandle, state: &AppState, game_id: i64) -> CommandResult<Standing> {
    let settings = state.settings();
    if !settings.save_sync_enabled {
        return Ok(Standing::Settled(SaveSyncState::Off));
    }
    let context = context(state, game_id).await?;
    let title = match resolve_saves(app, state, &context).await?.require_title() {
        Ok(title) => title,
        Err(unmatched) => return Ok(Standing::Settled(unmatched)),
    };

    let sync = sync_for(state, &context, &settings).await?;
    let remote = match sync.newest_remote(game_id).await {
        Ok(remote) => remote,
        // Both mean the server cannot store saves; the UI advises differently for each.
        Err(gameyfin_api::ApiError::SaveSyncDisabled) => {
            return Ok(Standing::Settled(SaveSyncState::Disabled))
        }
        Err(gameyfin_api::ApiError::SaveSyncUnsupported { .. }) => {
            return Ok(Standing::Settled(SaveSyncState::Unsupported))
        }
        Err(e) => return Err(e.into()),
    };
    Ok(Standing::Open {
        context: Box::new(context),
        title,
        remote,
    })
}

/// Local change by the record of the last backup: cheap, and wrong once the record is.
async fn recorded_change(context: &GameContext) -> bool {
    let staged = staged_hash(context).await;
    match (&staged, &context.record_saves.last_backup_hash) {
        (Some(current), Some(synced)) => current != synced,
        (Some(_), None) => true,
        _ => false,
    }
}

pub async fn state_of(
    app: &AppHandle,
    state: &AppState,
    game_id: i64,
) -> CommandResult<SaveSyncState> {
    let (context, remote) = match standing(app, state, game_id).await? {
        Standing::Settled(settled) => return Ok(settled),
        Standing::Open {
            context, remote, ..
        } => (context, remote),
    };
    let local_changed = recorded_change(&context).await;
    Ok(save_sync::decide(
        &context.record_saves,
        remote.as_ref(),
        local_changed,
        context.platform(),
        state.settings().installation_id.as_deref(),
    ))
}

/// This PC's own save files for a game, from a scan rather than from a record.
#[derive(Debug, Clone, Default)]
struct LocalFiles {
    present: bool,
    /// Whether they differ from what the last sync left in staging.
    changed: bool,
    newest_at: Option<String>,
}

async fn scan_local(
    app: &AppHandle,
    state: &AppState,
    context: &GameContext,
    title: &str,
) -> CommandResult<LocalFiles> {
    let ludusavi = ludusavi_for(app, state, context).await?;
    let scan = ludusavi
        .preview(title, &context.staging)
        .await
        .context("could not look at this PC's saves")?;
    drop(ludusavi);

    let newest_at = scan
        .games
        .values()
        .flat_map(|game| game.files.iter())
        .filter(|(_, file)| !file.failed && !file.ignored)
        .filter_map(|(path, _)| std::fs::metadata(path).and_then(|m| m.modified()).ok())
        .max()
        .and_then(|written| {
            time::OffsetDateTime::from(written)
                .format(&time::format_description::well_known::Rfc3339)
                .ok()
        });
    Ok(LocalFiles {
        present: scan.games.values().any(|game| game.produced_data()),
        changed: scan.games.values().any(|game| game.changed()),
        newest_at,
    })
}

/// What a launch goes on: the state judged by the save files actually on disk, what those
/// files are, the newest stored version, and what a first start should do about it.
struct LaunchView {
    state: SaveSyncState,
    local: Option<LocalFiles>,
    newest: Option<SaveVersion>,
    first_start: Option<save_sync::FirstStart>,
}

async fn state_at_launch(
    app: &AppHandle,
    state: &AppState,
    game_id: i64,
) -> CommandResult<LaunchView> {
    let (context, title, remote) = match standing(app, state, game_id).await? {
        Standing::Settled(settled) => {
            return Ok(LaunchView {
                state: settled,
                local: None,
                newest: None,
                first_start: None,
            })
        }
        Standing::Open {
            context,
            title,
            remote,
        } => (context, title, remote),
    };

    // The files, not the record of the last backup, which can be wrong.
    let local = match scan_local(app, state, &context, &title).await {
        Ok(local) => Some(local),
        Err(e) => {
            tracing::warn!(game_id, error = %e, "could not scan this PC's saves; using the record");
            None
        }
    };
    let local_changed = match &local {
        Some(local) => local.changed,
        None => recorded_change(&context).await,
    };
    let judged = save_sync::decide(
        &context.record_saves,
        remote.as_ref(),
        local_changed,
        context.platform(),
        state.settings().installation_id.as_deref(),
    );
    let judged = match &local {
        Some(local) => save_sync::missing_here(judged, remote.as_ref(), local.present),
        None => judged,
    };

    // A PC that never synced has nothing to compare against, so a first start asks which
    // stored save to use. Which of them restore here is worked out when asking.
    let first_start = match &remote {
        Some(newest) if save_sync::never_synced(&context.record_saves) => {
            Some(save_sync::first_start(&context.record_saves, &newest.id))
        }
        _ => None,
    };
    Ok(LaunchView {
        state: judged,
        local,
        newest: remote,
        first_start,
    })
}

/// Which side of a play session a sync belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum SyncMoment {
    Launch,
    Exit,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SaveSyncPhase {
    /// Asking the store what it holds, which is the moment skipping is free.
    Checking,
    Downloading,
    /// Files are being written into the game's folders; stopping here would leave half a save.
    Restoring,
    Scanning,
    Uploading,
    Done {
        state: SaveSyncState,
    },
    Skipped,
    /// The user chose this PC's save over the newest stored version, so nothing was restored.
    KeptLocal,
    /// The first-start prompt took over, so this window has nothing more to say.
    Asking,
    /// The stored save was put back. Where it went is said, so the user can find it.
    Restored {
        files: u32,
        folders: Vec<String>,
        saved_at: Option<String>,
        device: Option<String>,
    },
    Failed {
        message: String,
    },
}

impl SaveSyncPhase {
    /// Whether this is the end of it, so the window knows to close itself.
    fn is_final(&self) -> bool {
        matches!(
            self,
            SaveSyncPhase::Done { .. }
                | SaveSyncPhase::Skipped
                | SaveSyncPhase::KeptLocal
                | SaveSyncPhase::Asking
                | SaveSyncPhase::Restored { .. }
                | SaveSyncPhase::Failed { .. }
        )
    }
}

/// What the window around a launch or an exit is told.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveSyncProgress {
    pub game_id: i64,
    pub title: String,
    pub moment: SyncMoment,
    pub phase: SaveSyncPhase,
    /// False once stopping would leave the save half written, so the button can say so.
    pub skippable: bool,
    /// A failure that keeps the game from starting until the user chooses what to do.
    pub blocking: bool,
}

/// Reports an automatic sync's progress and carries a skip, which is read between steps so it
/// never interrupts anything.
pub struct SyncWatch {
    app: AppHandle,
    game_id: i64,
    title: String,
    moment: SyncMoment,
    skips: Arc<std::sync::Mutex<HashSet<i64>>>,
}

impl SyncWatch {
    pub fn new(
        app: &AppHandle,
        state: &AppState,
        game_id: i64,
        title: String,
        moment: SyncMoment,
    ) -> Self {
        Self {
            app: app.clone(),
            game_id,
            title,
            moment,
            skips: state.save_skips(),
        }
    }

    fn say(&self, phase: SaveSyncPhase, skippable: bool, blocking: bool) {
        if phase.is_final() {
            // Cleared here rather than at the next launch: a skip left behind would stop a
            // sync the user never asked to stop.
            crate::state::lock(&self.skips).remove(&self.game_id);
        }
        let _ = self.app.emit(
            "save-sync-progress",
            SaveSyncProgress {
                game_id: self.game_id,
                title: self.title.clone(),
                moment: self.moment,
                phase,
                skippable,
                blocking,
            },
        );
    }

    /// A step that can still be abandoned without consequence.
    fn step(&self, phase: SaveSyncPhase) {
        self.say(phase, true, false);
    }

    /// A step that has to finish now it has started.
    fn committed(&self, phase: SaveSyncPhase) {
        self.say(phase, false, false);
    }

    fn finish(&self, phase: SaveSyncPhase) {
        self.say(phase, false, false);
    }

    /// Stops the launch on a failure the user has to answer before the game starts.
    fn hold(&self, message: String) {
        self.say(SaveSyncPhase::Failed { message }, false, true);
    }

    fn skipped(&self) -> bool {
        crate::state::lock(&self.skips).contains(&self.game_id)
    }
}

/// Stops the sync a game is waiting on, if it has not reached the point of no return.
#[tauri::command]
pub async fn skip_save_sync(state: State<'_, AppState>, game_id: i64) -> CommandResult<()> {
    crate::state::lock(&state.save_skips()).insert(game_id);
    Ok(())
}

/// Which games the Saves tab is asking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum SaveScope {
    /// Games installed on this PC, which is the only place a backup can be made.
    Installed,
    /// Installed games, and uninstalled ones with stored saves, so a save left behind shows.
    WithSaves,
    All,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveOverviewRow {
    pub game_id: i64,
    pub title: String,
    pub installed: bool,
    pub state: SaveSyncState,
    pub versions: u32,
    pub newest_at: Option<String>,
    pub newest_device: Option<String>,
    /// The newest version's size, which is what the user recognises a save by.
    pub size_bytes: u64,
    pub remote_platform: Option<SavePlatform>,
    /// Whether anything has ever identified this game, so a row can offer to look.
    pub identified: bool,
}

/// How many games are asked about at once. The store is a server or a share, so this is
/// about not opening a hundred connections rather than about local work.
const OVERVIEW_CONCURRENCY: usize = 8;

/// Every game's save status in one pass. Never runs the helper, so a large library costs only
/// cheap store listings, asked in parallel.
#[tauri::command]
pub async fn save_overview(
    state: State<'_, AppState>,
    scope: SaveScope,
) -> CommandResult<Vec<SaveOverviewRow>> {
    let settings = state.settings();
    let library = state.library();

    let mut rows = Vec::new();
    let mut wanted = Vec::new();
    for game in state.games().await? {
        let installed = library.record(game.id).is_installed();
        if scope == SaveScope::Installed && !installed {
            continue;
        }
        wanted.push((game.id, game.title.clone(), installed));
    }

    if !settings.save_sync_enabled {
        return Ok(wanted
            .into_iter()
            // Nothing is listed with sync off, so no uninstalled game is known to have saves.
            .filter(|(_, _, installed)| scope != SaveScope::WithSaves || *installed)
            .map(|(game_id, title, installed)| SaveOverviewRow {
                game_id,
                title,
                installed,
                state: SaveSyncState::Off,
                versions: 0,
                newest_at: None,
                newest_device: None,
                size_bytes: 0,
                remote_platform: None,
                identified: false,
            })
            .collect());
    }

    let store: Arc<dyn SaveStore> = Arc::from(store_for(&state, &settings).await?);
    // Every store can say which games it holds anything for in one listing, so most of the
    // library needs no request at all. An empty or failed answer means "ask about all of them".
    let held: Option<HashSet<i64>> = match store.games().await {
        Ok(games) if !games.is_empty() => Some(games.into_iter().collect()),
        _ => None,
    };

    let mut listings: HashMap<i64, Vec<SaveVersion>> = HashMap::new();
    let mut unavailable: Option<SaveSyncState> = None;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(OVERVIEW_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();
    for (game_id, ..) in &wanted {
        let game_id = *game_id;
        if held.as_ref().is_some_and(|held| !held.contains(&game_id)) {
            continue;
        }
        let (store, semaphore) = (store.clone(), semaphore.clone());
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            (game_id, store.list(game_id).await)
        });
    }
    while let Some(finished) = tasks.join_next().await {
        let Ok((game_id, listed)) = finished else {
            continue;
        };
        match listed {
            Ok(versions) => {
                listings.insert(game_id, versions);
            }
            // Both mean the whole store cannot answer, so the rest of the sweep is pointless.
            Err(gameyfin_api::ApiError::SaveSyncDisabled) => {
                unavailable = Some(SaveSyncState::Disabled);
                tasks.abort_all();
            }
            Err(gameyfin_api::ApiError::SaveSyncUnsupported { .. }) => {
                unavailable = Some(SaveSyncState::Unsupported);
                tasks.abort_all();
            }
            Err(e) => {
                tracing::debug!(game_id, error = %e, "could not list saves");
                listings.remove(&game_id);
            }
        }
    }

    for (game_id, title, installed) in wanted {
        let listed = listings.remove(&game_id).unwrap_or_default();
        if scope == SaveScope::WithSaves && !installed && listed.is_empty() {
            continue;
        }
        if let Some(state) = unavailable.clone() {
            rows.push(SaveOverviewRow {
                game_id,
                title,
                installed,
                state,
                versions: 0,
                newest_at: None,
                newest_device: None,
                size_bytes: 0,
                remote_platform: None,
                identified: false,
            });
            continue;
        }
        let versions = listed;
        let newest = versions.first();
        let context = context(&state, game_id).await?;
        let staged = staged_hash(&context).await;
        let local_changed = match (&staged, &context.record_saves.last_backup_hash) {
            (Some(current), Some(synced)) => current != synced,
            (Some(_), None) => true,
            _ => false,
        };
        rows.push(SaveOverviewRow {
            state: save_sync::decide(
                &context.record_saves,
                newest,
                local_changed,
                context.platform(),
                settings.installation_id.as_deref(),
            ),
            versions: versions.len() as u32,
            newest_at: newest.and_then(|v| v.created_at.clone()),
            newest_device: newest.and_then(|v| v.device_name.clone()),
            size_bytes: newest.map_or(0, |v| v.size_bytes),
            remote_platform: newest.map(|v| {
                v.platform
                    .parse::<SavePlatform>()
                    .unwrap_or(SavePlatform::Unknown)
            }),
            identified: context.record_saves.ludusavi_title.is_some()
                || !context.record_saves.custom_paths.is_empty(),
            game_id,
            title,
            installed,
        });
    }
    Ok(rows)
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

/// Deletes stored versions for good, as the server's own saves page does.
#[tauri::command]
pub async fn delete_save_versions(
    state: State<'_, AppState>,
    game_id: i64,
    save_ids: Vec<String>,
) -> CommandResult<()> {
    let settings = state.settings();
    let store = store_for(&state, &settings).await?;
    for id in &save_ids {
        store.delete(game_id, id).await?;
    }
    let newest_left = store.list(game_id).await?.into_iter().next().map(|v| v.id);
    tracing::info!(game_id, deleted = ?save_ids, "deleted save versions");
    state
        .library()
        .update_record(game_id, move |record| {
            save_sync::forget_versions(&mut record.saves, &save_ids, newest_left)
        })
        .await;
    Ok(())
}

/// Keeps a version safe from the store pruning old versions, or lets it be pruned again.
#[tauri::command]
pub async fn set_save_locked(
    state: State<'_, AppState>,
    game_id: i64,
    save_id: String,
    locked: bool,
) -> CommandResult<()> {
    let settings = state.settings();
    store_for(&state, &settings)
        .await?
        .set_locked(game_id, &save_id, locked)
        .await?;
    Ok(())
}

/// Deletes every stored save, for every game, as the server's saves page can.
#[tauri::command]
pub async fn delete_all_saves(state: State<'_, AppState>) -> CommandResult<u32> {
    let settings = state.settings();
    let deleted = store_for(&state, &settings).await?.delete_all().await?;
    let mut by_game: HashMap<i64, Vec<String>> = HashMap::new();
    for version in &deleted {
        by_game
            .entry(version.game_id)
            .or_default()
            .push(version.id.clone());
    }
    tracing::info!(
        games = by_game.len(),
        versions = deleted.len(),
        "deleted every stored save"
    );
    for (game_id, ids) in by_game {
        state
            .library()
            .update_record(game_id, move |record| {
                save_sync::forget_versions(&mut record.saves, &ids, None)
            })
            .await;
    }
    Ok(u32::try_from(deleted.len()).unwrap_or(u32::MAX))
}

/// Backs up and uploads. The result includes a conflict, which the UI then resolves.
#[tauri::command]
pub async fn backup_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    force: bool,
) -> CommandResult<SaveSyncState> {
    do_backup(&app, &state, game_id, force, None)
        .await
        .inspect_err(|e| tracing::error!(game_id, error = %e, "backing up saves failed"))
}

async fn do_backup(
    app: &AppHandle,
    state: &AppState,
    game_id: i64,
    force: bool,
    watch: Option<&SyncWatch>,
) -> CommandResult<SaveSyncState> {
    ensure_installation_id(state).await?;
    let settings = state.settings();
    let context = context(state, game_id).await?;
    let title = match resolve_saves(app, state, &context).await?.require_title() {
        Ok(title) => title,
        Err(unmatched) => return Ok(unmatched),
    };

    if let Some(watch) = watch {
        watch.step(SaveSyncPhase::Scanning);
    }
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

    if watch.is_some_and(SyncWatch::skipped) {
        tracing::info!(
            game_id,
            "the upload was skipped; the backup stays on this PC"
        );
        return Ok(SaveSyncState::LocalNewer {
            local_at: context.record_saves.last_backup_at.clone(),
        });
    }
    if let Some(watch) = watch {
        watch.committed(SaveSyncPhase::Uploading);
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

/// What a restore did: "in sync" alone does not say the save was just put back, or where.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreReport {
    pub state: SaveSyncState,
    pub files: u32,
    /// Where the files went, leaving out folders nested in another.
    pub folders: Vec<String>,
    /// When the restored version was saved, and on which device.
    pub saved_at: Option<String>,
    pub device: Option<String>,
}

impl RestoreReport {
    /// A restore that wrote nothing, with the state saying why.
    fn untouched(state: SaveSyncState) -> Self {
        Self {
            state,
            files: 0,
            folders: Vec::new(),
            saved_at: None,
            device: None,
        }
    }
}

#[tauri::command]
pub async fn restore_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    save_id: Option<String>,
) -> CommandResult<RestoreReport> {
    do_restore(&app, &state, game_id, save_id, None)
        .await
        .inspect_err(|e| tracing::error!(game_id, error = %e, "restoring saves failed"))
}

async fn do_restore(
    app: &AppHandle,
    state: &AppState,
    game_id: i64,
    save_id: Option<String>,
    watch: Option<&SyncWatch>,
) -> CommandResult<RestoreReport> {
    let settings = state.settings();
    let context = context(state, game_id).await?;
    let title = match resolve_saves(app, state, &context).await?.require_title() {
        Ok(title) => title,
        Err(unmatched) => return Ok(RestoreReport::untouched(unmatched)),
    };
    let sync = sync_for(state, &context, &settings).await?;

    let versions = sync.versions(game_id).await?;
    // An older version restored by hand counts as based on the newest, or the next launch
    // would restore the newest right over it.
    let newest_id = versions.first().map(|v| v.id.clone());
    let version = match save_id {
        Some(id) => versions
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| CommandError::msg("That save version is gone."))?,
        None => versions
            .into_iter()
            .next()
            .ok_or_else(|| CommandError::msg("There is no save on the server yet."))?,
    };
    if let Some(watch) = watch {
        watch.step(SaveSyncPhase::Downloading);
    }
    sync.fetch(game_id, &version.id).await?;

    // The last moment stopping costs nothing: the archive is downloaded but no file of the
    // game's has been touched.
    if watch.is_some_and(SyncWatch::skipped) {
        tracing::info!(game_id, "the restore was skipped before it started");
        return Ok(RestoreReport::untouched(SaveSyncState::RemoteNewer {
            remote_at: version.created_at,
            device: version.device_name,
        }));
    }
    if let Some(watch) = watch {
        watch.committed(SaveSyncPhase::Restoring);
    }
    let ludusavi = ludusavi_for(app, state, &context).await?;
    let restored = ludusavi
        .restore(&title, &context.staging)
        .await
        .context("restoring the save failed")?;
    drop(ludusavi);

    // Ludusavi reports these as handled. Outside a sandbox the write fails; inside a Flatpak
    // it lands in scratch space that vanishes, so the game never sees the save either way.
    let misplaced: Vec<&str> = restored
        .games
        .values()
        .flat_map(|game| game.misplaced(gameyfin_saves::config::SYNTHETIC_ROOT))
        .map(|(path, _)| path)
        .collect();
    if !misplaced.is_empty() {
        tracing::warn!(
            game_id,
            ?misplaced,
            "the restore could not place every file"
        );
        return Err(CommandError::msg(format!(
            "The save could not be put back: {} of its files have nowhere to go on this PC. \
             Set its folders under Saves, then restore it again.",
            misplaced.len()
        )));
    }

    let placed: Vec<&str> = restored
        .games
        .values()
        .flat_map(|game| game.placed())
        .collect();
    let report = RestoreReport {
        state: SaveSyncState::InSync {
            last_synced_at: version.created_at.clone(),
        },
        files: u32::try_from(placed.len()).unwrap_or(u32::MAX),
        folders: gameyfin_saves::api::folders_of(placed),
        saved_at: version.created_at,
        device: version.device_name,
    };
    tracing::info!(
        game_id,
        files = report.files,
        folders = ?report.folders,
        "save restored"
    );

    let hash = staged_hash(&context).await;
    let base = newest_id.or(Some(version.id));
    record_sync(state, game_id, base, hash, context.platform()).await;
    emit_state(app, game_id, &report.state);
    Ok(report)
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
            do_backup(&app, &state, game_id, true, None).await
        }
        ConflictChoice::KeepRemote => do_restore(&app, &state, game_id, None, None)
            .await
            .map(|report| report.state),
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

/// Its own command, so changing the mapping cannot erase hand-entered paths.
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

/// Folders worth offering to open or to browse for one game's saves.
#[derive(Debug, Default, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveLocations {
    /// Holds every game's staging directory and the packed archives beside them.
    pub saves_root: Option<String>,
    /// What the helper backs this game up into.
    pub staging: Option<String>,
    /// The Windows-side home inside the prefix, where a Proton game keeps its saves.
    pub prefix_home: Option<String>,
    /// The prefix's drive C, for a game that keeps saves beside itself instead.
    pub prefix_drive_c: Option<String>,
    pub install_dir: Option<String>,
    pub home: Option<String>,
    /// Whether this game writes saves inside its prefix, so browsing should start there.
    pub saves_in_prefix: bool,
    /// Where the helper actually found files. Only filled when asked for: it runs a scan.
    pub detected: Vec<String>,
    /// Folders named like the game in the usual save places. Filled with `detected`.
    pub suggested: Vec<String>,
}

/// Where games keep saves by convention, with how deep the game's own folder can sit.
fn save_roots(context: &GameContext) -> Vec<(PathBuf, usize)> {
    let windows_home = if context.saves_in_prefix() {
        gameyfin_core::prefix::prefix_home(&context.prefix_dir)
    } else if cfg!(windows) {
        home()
    } else {
        None
    };
    if let Some(profile) = windows_home {
        // `Documents/My Games/Studio/Game` is the deepest common layout.
        return [
            ("Documents", 3),
            ("Saved Games", 2),
            ("AppData/Roaming", 2),
            ("AppData/Local", 2),
            ("AppData/LocalLow", 2),
        ]
        .into_iter()
        .map(|(sub, depth)| (profile.join(sub), depth))
        .collect();
    }
    home()
        .map(|home| {
            vec![
                (home.join(".local/share"), 2),
                (home.join(".config"), 2),
                (home, 1),
            ]
        })
        .unwrap_or_default()
}

/// Containers whose names say nothing about the game, but may hide it one level down.
const GENERIC_FOLDERS: &[&str] = &[
    "mygames",
    "microsoft",
    "temp",
    "packages",
    "programs",
    "cache",
    "crashdumps",
    "unity",
    "google",
    "nvidia",
];

/// Whether a folder name is close enough to the title to offer, e.g. "Witcher 3" for
/// "The Witcher 3: Wild Hunt". Both sides already [`comparable`].
fn named_like(title: &str, folder: &str) -> bool {
    const SHORTEST: usize = 5;
    if GENERIC_FOLDERS.contains(&folder) {
        return false;
    }
    folder == title
        || (folder.len() >= SHORTEST && title.contains(folder))
        || (title.len() >= SHORTEST && folder.contains(title))
}

/// Folders under `roots` named like `title`, shallowest first. Bounded, since AppData can hold
/// thousands of folders.
fn folders_named_like(title: &str, roots: &[(PathBuf, usize)]) -> Vec<String> {
    const MOST_VISITED: usize = 5000;
    const MOST_FOUND: usize = 5;
    let wanted = comparable(title);
    if wanted.len() < 3 {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut visited = 0;
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = roots.iter().cloned().collect();
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MOST_VISITED || found.len() >= MOST_FOUND {
                return found;
            }
            // Not following links: a prefix links its folders back into the host home.
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let path = entry.path();
            if named_like(&wanted, &comparable(&entry.file_name().to_string_lossy())) {
                found.push(path.to_string_lossy().into_owned());
            } else if depth > 1 {
                queue.push_back((path, depth - 1));
            }
        }
    }
    found
}

/// Only folders that exist: opening one that does not is refused, and offering to browse
/// into nothing is worse than not offering.
fn existing(path: PathBuf) -> Option<String> {
    path.is_dir().then(|| path.to_string_lossy().into_owned())
}

/// Where a game's saves are. `probe` runs a scan, which takes the helper's lock, so only the
/// dialog asks for it.
#[tauri::command]
pub async fn save_locations(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    probe: bool,
) -> CommandResult<SaveLocations> {
    let context = context(&state, game_id).await?;
    let wine_root = gameyfin_core::prefix::wine_root(&context.prefix_dir);

    let mut locations = SaveLocations {
        saves_root: existing(context.saves_root.clone()),
        staging: existing(context.staging.clone()),
        prefix_home: gameyfin_core::prefix::prefix_home(&context.prefix_dir).and_then(existing),
        prefix_drive_c: existing(wine_root.join("drive_c")),
        install_dir: existing(context.install_dir.clone()),
        home: home().and_then(existing),
        saves_in_prefix: context.saves_in_prefix(),
        detected: Vec::new(),
        suggested: Vec::new(),
    };
    if !probe {
        return Ok(locations);
    }

    // Before identifying: an unknown game is the one that most needs a suggestion.
    let (title, roots) = (context.title.clone(), save_roots(&context));
    locations.suggested = tokio::task::spawn_blocking(move || folders_named_like(&title, &roots))
        .await
        .unwrap_or_default();

    let Ok(title) = resolve_saves(&app, &state, &context).await?.require_title() else {
        return Ok(locations);
    };
    let ludusavi = ludusavi_for(&app, &state, &context).await?;
    match ludusavi.preview(&title, &context.staging).await {
        Ok(scan) => {
            let mut folders: Vec<String> = Vec::new();
            for file in scan.games.values().flat_map(|game| game.files.keys()) {
                // The folder, not the file: it is what a file manager can be pointed at.
                let Some(folder) = Path::new(file).parent().map(Path::to_path_buf) else {
                    continue;
                };
                if let Some(folder) = existing(folder) {
                    if !folders.contains(&folder) {
                        folders.push(folder);
                    }
                }
            }
            locations.detected = folders;
        }
        Err(e) => tracing::debug!(game_id, error = %e, "could not scan for save folders"),
    }
    Ok(locations)
}

/// How a find was matched to a game in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum SaveFindMatch {
    /// This game was already backed up under that title.
    Recorded,
    /// The titles are the same, give or take capitals and punctuation.
    Title,
    /// Nothing in the library answers to it.
    None,
}

/// Saves the helper found on this PC for one game.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveFind {
    /// The title the save database knows it by, which is what a backup is filed under.
    pub ludusavi_title: String,
    pub files: u32,
    pub bytes: u64,
    /// Where the files are, so the user can look before deciding.
    pub folder: Option<String>,
    pub game_id: Option<i64>,
    pub game_title: Option<String>,
    pub matched: SaveFindMatch,
    /// Whether the store already holds something for the game this was matched to.
    pub already_backed_up: bool,
}

/// A scan of the whole machine can take minutes, where one game takes seconds.
const SCAN_TIMEOUT: Duration = Duration::from_secs(3600);

/// Compares titles the way a person would: case, spacing and punctuation are not the point.
fn comparable(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Everything on this PC the save database recognises. Its own config directory and lock keep a
/// whole-machine scan from delaying a launch.
#[tauri::command]
pub async fn scan_this_pc(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Vec<SaveFind>> {
    let app_config = state.config_dir();
    let binary = ludusavi_binary(&app, &app_config)?;
    let settings = state.settings();
    let _guard = state.save_scan_lock().lock_owned().await;

    let scan_config = app_config.join("ludusavi-scan");
    let staging = scan_config.join("preview");
    let mut builder = ConfigBuilder::new(&staging);
    // No redirects: a preview reports where files are, and a rewritten path would name a
    // folder that does not exist on this machine.
    if let Some(steam) = home().and_then(|home| gameyfin_core::steam::root(&home)) {
        builder = builder.root(
            gameyfin_saves::config::RootStore::Steam,
            steam.to_string_lossy(),
        );
    }
    // Every prefix Gameyfin has made, which is where a Windows game's saves are.
    for root in settings.library_roots() {
        let prefixes = InstallLayout::new(PathBuf::from(root)).prefixes_root();
        let Ok(entries) = std::fs::read_dir(&prefixes) else {
            continue;
        };
        for prefix in entries.flatten().map(|e| e.path()) {
            if gameyfin_core::prefix::wine_root(&prefix)
                .join("drive_c")
                .is_dir()
            {
                builder = builder.wine_prefix(&prefix);
            }
        }
    }
    // The manifest is 17 MB and already downloaded; copying beats fetching it twice.
    tokio::fs::create_dir_all(&scan_config)
        .await
        .context("could not prepare the scan")?;
    let manifest = ludusavi_config_dir(&app_config).join("manifest.yaml");
    if manifest.is_file() && !scan_config.join("manifest.yaml").is_file() {
        let _ = tokio::fs::copy(&manifest, scan_config.join("manifest.yaml")).await;
    }
    builder
        .write(&scan_config)
        .await
        .context("could not configure the scan")?;

    let ludusavi = Ludusavi::with_runner(
        binary,
        &scan_config,
        Box::new(gameyfin_saves::ProcessRunner::with_timeout(SCAN_TIMEOUT)),
    )
    .auto_update_manifest(settings.save_manifest_auto_update);

    tracing::info!("scanning this PC for saves");
    let scan = ludusavi
        .preview_all(&staging)
        .await
        .map_err(|e| CommandError::msg(format!("could not scan for saves: {e}")))?;

    // What the library can be matched against, without asking the helper again.
    let games = state.games().await.unwrap_or_default();
    let library = state.library();
    let mut by_recorded_title: HashMap<String, i64> = HashMap::new();
    let mut by_title: HashMap<String, i64> = HashMap::new();
    for game in &games {
        if let Some(title) = library.record(game.id).saves.ludusavi_title {
            by_recorded_title.insert(title, game.id);
        }
        by_title.entry(comparable(&game.title)).or_insert(game.id);
    }

    let mut finds: Vec<SaveFind> = Vec::new();
    for (title, found) in scan.games {
        if !found.produced_data() {
            continue;
        }
        let (game_id, matched) = match by_recorded_title.get(&title) {
            Some(id) => (Some(*id), SaveFindMatch::Recorded),
            None => match by_title.get(&comparable(&title)) {
                Some(id) => (Some(*id), SaveFindMatch::Title),
                None => (None, SaveFindMatch::None),
            },
        };
        let folder = found
            .files
            .keys()
            .next()
            .and_then(|file| Path::new(file).parent())
            .map(|folder| folder.to_string_lossy().into_owned());
        finds.push(SaveFind {
            game_title: game_id
                .and_then(|id| games.iter().find(|g| g.id == id))
                .map(|game| game.title.clone()),
            already_backed_up: game_id
                .is_some_and(|id| library.record(id).saves.last_backup_hash.is_some()),
            files: found.files.len() as u32,
            bytes: found.bytes(),
            folder,
            game_id,
            matched,
            ludusavi_title: title,
        });
    }
    // The ones that can be acted on first, then alphabetically.
    finds.sort_by(|a, b| {
        (a.game_id.is_none(), comparable(&a.ludusavi_title))
            .cmp(&(b.game_id.is_none(), comparable(&b.ludusavi_title)))
    });
    tracing::info!(found = finds.len(), "scan finished");
    Ok(finds)
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

/// Copies saves from another location into the one in use. The source is never touched, and
/// anything already at the destination is skipped, so a failed run can be repeated.
#[tauri::command]
pub async fn migrate_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    from: SaveBackend,
    all_versions: bool,
) -> CommandResult<gameyfin_core::save_migration::MigrationSummary> {
    let settings = state.settings();
    let to = settings.save_backend;
    if from == to {
        return Err(CommandError::msg(
            "Saves are kept there already. Pick another place to copy from.",
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

/// How many past versions of the save helper are offered.
const SAVE_TOOL_CHOICES: usize = 10;

#[tauri::command]
pub async fn save_tool_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<gameyfin_core::save_tool::SaveToolStatus> {
    let config_dir = state.config_dir();
    // An unreachable feed must not stop the screen showing what is installed.
    let releases = gameyfin_core::save_tool::releases(&state.http(), SAVE_TOOL_CHOICES)
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
    let releases = gameyfin_core::save_tool::releases(&state.http(), SAVE_TOOL_CHOICES)
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
        crate::progress::emitter(&app, "save-tool-progress", None),
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

/// A stored version offered on a first start, and whether it restores on this PC.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OfferedSave {
    pub version: SaveVersion,
    pub restorable: bool,
}

/// The stored saves offered on the first start of a game on this PC, newest first.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavePullOffer {
    pub game_id: i64,
    pub title: String,
    pub versions: Vec<OfferedSave>,
    /// Whether this PC already has save files for the game, which a restore replaces.
    pub local_saves: bool,
    /// When those files last changed, shown beside the stored saves' dates.
    pub local_at: Option<String>,
}

/// Every stored version to offer, or None when not one of them restores on this PC.
async fn pull_offer(
    state: &AppState,
    game_id: i64,
    title: &str,
    local: Option<&LocalFiles>,
) -> CommandResult<Option<SavePullOffer>> {
    let settings = state.settings();
    let context = context(state, game_id).await?;
    let versions: Vec<OfferedSave> = store_for(state, &settings)
        .await?
        .list(game_id)
        .await?
        .into_iter()
        .map(|version| OfferedSave {
            restorable: save_sync::restorable_here(
                &version,
                context.platform(),
                settings.installation_id.as_deref(),
            ),
            version,
        })
        .collect();
    if !versions.iter().any(|offered| offered.restorable) {
        return Ok(None);
    }
    Ok(Some(SavePullOffer {
        game_id,
        title: title.to_string(),
        versions,
        // An unreadable scan counts as saves present, so the prompt warns before replacing.
        local_saves: local.is_none_or(|local| local.present),
        local_at: local.and_then(|local| local.newest_at.clone()),
    }))
}

/// What [`before_launch`] decided the launch should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchGate {
    Proceed,
    /// The dialog starts the game once answered, so launching here would race the restore.
    AwaitingSaveDecision,
}

/// Restores a newer save before the game starts, judged by the save files on disk. Only a
/// restore that failed holds the game; every other outcome is shown and the game starts.
pub async fn before_launch(app: &AppHandle, state: &AppState, game_id: i64) -> LaunchGate {
    let settings = state.settings();
    if !settings.save_sync_enabled || !settings.sync_saves_on_launch {
        return LaunchGate::Proceed;
    }
    // Answered in the first-start prompt a moment ago, which settled the save already.
    let answered = crate::state::lock(&state.answered_offers()).remove(&game_id);
    if answered.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(120)) {
        return LaunchGate::Proceed;
    }
    let title = state.title(game_id).await;
    // On the row, so a check too quick for the window still shows the launch is under way.
    state.library().set_activity(
        game_id,
        crate::library_state::Activity::preparing("Checking saves"),
    );
    crate::ipc::notify_state(app, game_id);
    let watch = SyncWatch::new(app, state, game_id, title.clone(), SyncMoment::Launch);
    watch.step(SaveSyncPhase::Checking);

    let view = match state_at_launch(app, state, game_id).await {
        Ok(view) => view,
        Err(e) => {
            tracing::warn!(game_id, error = %e, "could not check saves before launch");
            watch.finish(SaveSyncPhase::Failed {
                message: e.to_string(),
            });
            return LaunchGate::Proceed;
        }
    };

    match (view.first_start, view.newest.as_ref()) {
        (Some(save_sync::FirstStart::KeepLocal), _) => {
            tracing::info!(
                game_id,
                "keeping this PC's save, as chosen for this version"
            );
            watch.finish(SaveSyncPhase::KeptLocal);
            return LaunchGate::Proceed;
        }
        (Some(save_sync::FirstStart::Ask), Some(_)) => {
            match pull_offer(state, game_id, &title, view.local.as_ref()).await {
                Ok(Some(offer)) => {
                    tracing::info!(
                        game_id,
                        versions = offer.versions.len(),
                        "first start here; asking which save to use"
                    );
                    // Closes this window before the prompt opens: two dialogs about one save at
                    // once is one too many.
                    watch.finish(SaveSyncPhase::Asking);
                    let _ = app.emit("save-pull-offer", offer);
                    return LaunchGate::AwaitingSaveDecision;
                }
                // None of them restores on this platform, and the state says why.
                Ok(None) => {
                    watch.finish(SaveSyncPhase::Done {
                        state: view.state.clone(),
                    });
                    return LaunchGate::Proceed;
                }
                Err(e) => {
                    tracing::warn!(game_id, error = %e, "could not list the saves to offer");
                    watch.finish(SaveSyncPhase::Failed {
                        message: e.to_string(),
                    });
                    return LaunchGate::Proceed;
                }
            }
        }
        _ => match &view.state {
            SaveSyncState::RemoteNewer { .. } => {}
            other => {
                if save_sync::needs_attention(other) {
                    tracing::info!(game_id, "save needs a decision before it can be restored");
                    emit_state(app, game_id, other);
                }
                watch.finish(SaveSyncPhase::Done {
                    state: other.clone(),
                });
                return LaunchGate::Proceed;
            }
        },
    }

    if watch.skipped() {
        crate::state::lock(&state.unsynced_sessions()).insert(game_id);
        watch.finish(SaveSyncPhase::Skipped);
        return LaunchGate::Proceed;
    }
    match do_restore(app, state, game_id, None, Some(&watch)).await {
        // Skipped after the download: the game starts without the newer save.
        Ok(RestoreReport {
            state: SaveSyncState::RemoteNewer { .. },
            ..
        }) => {
            crate::state::lock(&state.unsynced_sessions()).insert(game_id);
            watch.finish(SaveSyncPhase::Skipped);
        }
        Ok(RestoreReport {
            state: SaveSyncState::InSync { .. },
            files,
            folders,
            saved_at,
            device,
        }) => watch.finish(SaveSyncPhase::Restored {
            files,
            folders,
            saved_at,
            device,
        }),
        // Nothing was written, and the state says why.
        Ok(report) => watch.finish(SaveSyncPhase::Done {
            state: report.state,
        }),
        Err(e) => {
            tracing::warn!(game_id, error = %e, "could not restore the save before launch");
            // Playing on starts the game on an older save and uploads it over the newer one
            // at exit, so the user decides first.
            watch.hold(e.to_string());
            return LaunchGate::AwaitingSaveDecision;
        }
    }
    LaunchGate::Proceed
}

/// Records the answer to the first-start offer: the version to restore, or none to keep what
/// this PC has. Declining covers the newest version only, so a newer save asks again.
#[tauri::command]
pub async fn answer_save_pull_offer(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    save_id: Option<String>,
) -> CommandResult<()> {
    if let Some(save_id) = save_id {
        let report = do_restore(&app, &state, game_id, Some(save_id), None).await?;
        let SaveSyncState::InSync { .. } = report.state else {
            return Err(CommandError::msg(
                "Gameyfin does not know where this game keeps its saves yet. Choose the game \
                 under Saves, then restore it from there.",
            ));
        };
        state
            .library()
            .update_record(game_id, |r| r.saves.pull_offer_declined = None)
            .await;
        crate::state::lock(&state.answered_offers()).insert(game_id, std::time::Instant::now());
        // In the window a launch restore uses, so where the save went is said the same way.
        SyncWatch::new(
            &app,
            &state,
            game_id,
            state.title(game_id).await,
            SyncMoment::Launch,
        )
        .finish(SaveSyncPhase::Restored {
            files: report.files,
            folders: report.folders,
            saved_at: report.saved_at,
            device: report.device,
        });
        return Ok(());
    }
    let settings = state.settings();
    let newest = store_for(&state, &settings)
        .await?
        .list(game_id)
        .await?
        .into_iter()
        .next()
        .map(|version| version.id);
    state
        .library()
        .update_record(game_id, move |r| r.saves.pull_offer_declined = newest)
        .await;
    crate::state::lock(&state.answered_offers()).insert(game_id, std::time::Instant::now());
    Ok(())
}

pub async fn after_exit(app: &AppHandle, state: &AppState, game_id: i64) {
    let settings = state.settings();
    if !settings.save_sync_enabled || !settings.sync_saves_on_exit {
        return;
    }
    let watch = SyncWatch::new(
        app,
        state,
        game_id,
        state.title(game_id).await,
        SyncMoment::Exit,
    );
    watch.step(SaveSyncPhase::Checking);

    // Uploading now would bury the newer save under what was played without it.
    if crate::state::lock(&state.unsynced_sessions()).remove(&game_id) {
        tracing::info!(
            game_id,
            "not uploading a session that started without the newer save"
        );
        watch.finish(SaveSyncPhase::Skipped);
        return;
    }

    // Sync is on by default and many servers predate it, so one request here spares a
    // backup the upload would only discard.
    if settings.save_backend == SaveBackend::Server {
        if let Ok(store) = store_for(state, &settings).await {
            let settled = match store.list(game_id).await {
                Err(gameyfin_api::ApiError::SaveSyncUnsupported { .. }) => {
                    Some(SaveSyncState::Unsupported)
                }
                Err(gameyfin_api::ApiError::SaveSyncDisabled) => Some(SaveSyncState::Disabled),
                _ => None,
            };
            if let Some(settled) = settled {
                watch.finish(SaveSyncPhase::Done { state: settled });
                return;
            }
        }
    }
    match do_backup(app, state, game_id, false, Some(&watch)).await {
        Ok(next) => {
            if save_sync::needs_attention(&next) {
                tracing::info!(game_id, "save upload needs a decision");
            }
            watch.finish(SaveSyncPhase::Done { state: next });
        }
        Err(e) => {
            tracing::warn!(game_id, error = %e, "could not back up the save after playing");
            watch.finish(SaveSyncPhase::Failed {
                message: e.to_string(),
            });
        }
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

/// Not on Windows: every test here is about how a Linux machine tells a Proton game from a
/// native one, which on Windows is the question that never arises.
#[cfg(all(test, not(windows)))]
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
    fn folders_named_like_the_game_are_offered_within_their_depth() {
        let root = scratch("named-like");
        for dir in [
            "Documents/My Games/The Witcher 3",
            "Documents/A/B/C/The Witcher 3",
            "AppData/Local/Microsoft/Windows",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let roots = [(root.join("Documents"), 3), (root.join("AppData/Local"), 2)];

        let found = folders_named_like("The Witcher 3: Wild Hunt", &roots);
        let expected = root.join("Documents/My Games/The Witcher 3");
        assert_eq!(found, vec![expected.to_string_lossy().into_owned()]);
        assert!(
            folders_named_like("Microsoft Flight Simulator", &roots).is_empty(),
            "a generic container is not the game"
        );
    }

    #[test]
    fn a_windows_game_is_recognised_by_the_file_it_launches() {
        // The executable is stored relative to the install folder; opening that path as-is would tag
        // every Windows game on Linux as native.
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

    #[test]
    fn a_symlinked_path_is_written_as_the_path_it_resolves_to() {
        // On ostree `/home` is `/var/home`, so a redirect written through the link never matches.
        let dir = scratch("real");
        std::fs::create_dir_all(dir.join("var/home/u")).unwrap();
        std::os::unix::fs::symlink(dir.join("var/home"), dir.join("home")).unwrap();

        assert!(real(&dir.join("home/u")) == real(&dir.join("var/home/u")));
        // Not there yet: kept as written rather than dropped.
        assert!(real(&dir.join("nowhere")) == dir.join("nowhere"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_game_run_in_a_prefix_always_travels_by_the_portable_mapping() {
        // Cross-OS drops that mapping, so a save stored under /gameyfin/home would restore to nowhere.
        assert!(
            effective_strategy(RestoreStrategy::CrossOs, Some(true)) == RestoreStrategy::Portable
        );
        // A native build keeps the choice: bridging it to Windows is what cross-OS is for.
        assert!(
            effective_strategy(RestoreStrategy::CrossOs, Some(false)) == RestoreStrategy::CrossOs
        );
    }
}
