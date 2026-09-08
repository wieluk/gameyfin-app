//! Save synchronisation: drives Ludusavi locally and the server's `/saves` routes remotely.
//!
//! The decisions live in `gameyfin_core::save_sync`; this module resolves paths, runs the
//! sidecar, and reports progress to the UI.

use std::path::PathBuf;

use gameyfin_api::saves::{SaveVersion, UploadOutcome};
use gameyfin_core::save_sync::{self, LocalSaveState, SaveSync, SaveSyncState};
use gameyfin_core::{ConflictChoice, InstallLayout};
use gameyfin_saves::config::{Redirect, RedirectKind};
use gameyfin_saves::{
    BackupFormat, ConfigBuilder, GameIdentity, Ludusavi, RestoreStrategy, SavePlatform, TitleMatch,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::{CommandError, CommandResult};
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

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join(name);
            if beside.exists() {
                return Ok(beside);
            }
            let triple = dir.join(format!("{}-{}", name, current_triple()));
            if triple.exists() {
                return Ok(triple);
            }
        }
    }

    // The development layout, where `fetch-ludusavi.mjs` puts it.
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!("ludusavi-{}", current_triple()));
    if checkout.exists() {
        return Ok(checkout);
    }

    Err(CommandError::Message(
        "The save backup helper is missing from this build.".into(),
    ))
}

fn current_triple() -> &'static str {
    // Only the platforms the app ships for. Anything else has no bundled sidecar anyway.
    if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc.exe"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
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
    prefixes_root: PathBuf,
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
        prefixes_root: layout.prefixes_root(),
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
async fn ludusavi_for(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
) -> CommandResult<Ludusavi> {
    let binary = ludusavi_binary(app)?;
    let config_dir = state
        .config_dir()
        .await
        .join("ludusavi")
        .join(context.game_id.to_string());

    let mut builder = ConfigBuilder::new(&context.staging)
        .strategy(context.strategy)
        .wine_prefix_collection(&context.prefixes_root)
        .manual_redirects(context.redirects.clone())
        .portable_install_dir(&context.install_dir);

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

    Ok(Ludusavi::new(binary, config_dir))
}

/// Resolves the title Ludusavi knows this game by, caching it on the record.
async fn resolved_title(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
) -> CommandResult<Option<String>> {
    if let Some(title) = &context.record_saves.ludusavi_title {
        return Ok(Some(title.clone()));
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

    // A fuzzy match is never accepted without the user confirming it, so only a certain
    // one is cached here.
    let title = match resolved {
        TitleMatch::Certain(title) => Some(title),
        TitleMatch::Ambiguous(_) | TitleMatch::None => None,
    };

    if let Some(title) = &title {
        let title = title.clone();
        state
            .library()
            .update_record(context.game_id, move |record| {
                record.saves.ludusavi_title = Some(title);
            })
            .await;
    }

    Ok(title)
}

/// Candidate titles for a game Ludusavi could not identify on its own.
async fn candidates(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
) -> Vec<String> {
    let Ok(ludusavi) = ludusavi_for(app, state, context).await else {
        return Vec::new();
    };
    let identity = GameIdentity {
        title: context.title.clone(),
        steam_app_id: context.steam_app_id,
        gog_id: None,
    };

    match gameyfin_saves::resolve(&ludusavi, &identity).await {
        Ok(TitleMatch::Ambiguous(found)) => found.into_iter().map(|c| c.title).collect(),
        _ => Vec::new(),
    }
}

fn sync_for<'a>(
    client: &'a gameyfin_api::GameyfinClient,
    context: &GameContext,
    settings: &crate::settings::Settings,
) -> SaveSync<'a> {
    SaveSync::new(client, context.staging.clone())
        .identified_as(settings.installation_id.clone(), hostname())
}

/// A human-readable name for this machine, shown in the save history.
fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.is_empty())
}

/// Runs Ludusavi into the staging directory. False means the game had no saves to take.
///
/// Packing and hashing happen at upload time, so there is exactly one place that decides
/// what bytes the server sees.
async fn back_up(
    app: &AppHandle,
    state: &State<'_, AppState>,
    context: &GameContext,
    title: &str,
) -> CommandResult<bool> {
    let ludusavi = ludusavi_for(app, state, context).await?;

    tokio::fs::create_dir_all(&context.staging)
        .await
        .map_err(|e| CommandError::Message(format!("could not create the save folder: {e}")))?;

    let output = ludusavi
        .backup(title, &context.staging, BackupFormat::Zip)
        .await
        .map_err(|e| CommandError::Message(format!("save backup failed: {e}")))?;

    Ok(output.games.values().any(|game| game.produced_data()))
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
    save_id: Option<i64>,
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
    if resolved_title(app, state, &context).await?.is_none() {
        return Ok(SaveSyncState::Unmatched {
            candidates: candidates(app, state, &context).await,
        });
    }

    let client = state.client().await.ok_or(CommandError::NotConnected)?;
    let remote = match sync_for(&client, &context, &settings)
        .newest_remote(game_id)
        .await
    {
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
    let client = state.client().await.ok_or(CommandError::NotConnected)?;
    Ok(client.list_saves(game_id).await?)
}

/// Backs up and uploads. Returns the resulting state, including a conflict.
#[tauri::command]
pub async fn backup_saves(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    force: bool,
) -> CommandResult<SaveSyncState> {
    do_backup(&app, &state, game_id, force).await
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
    let Some(title) = resolved_title(app, state, &context).await? else {
        return Ok(SaveSyncState::Unmatched {
            candidates: candidates(app, state, &context).await,
        });
    };

    if !back_up(app, state, &context, &title).await? {
        // Nothing to back up is not an error: plenty of games have no saves yet.
        return Ok(SaveSyncState::NeverSynced);
    }

    let client = state.client().await.ok_or(CommandError::NotConnected)?;
    let sync = sync_for(&client, &context, &settings);
    let base = context.record_saves.last_synced_save_id;

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
            record_sync(state, game_id, Some(version.id), hash, context.platform()).await;
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
    save_id: Option<i64>,
) -> CommandResult<SaveSyncState> {
    do_restore(&app, &state, game_id, save_id).await
}

async fn do_restore(
    app: &AppHandle,
    state: &State<'_, AppState>,
    game_id: i64,
    save_id: Option<i64>,
) -> CommandResult<SaveSyncState> {
    let settings = state.settings().await;
    let context = context(state, game_id).await?;
    let Some(title) = resolved_title(app, state, &context).await? else {
        return Ok(SaveSyncState::Unmatched {
            candidates: candidates(app, state, &context).await,
        });
    };

    let client = state.client().await.ok_or(CommandError::NotConnected)?;
    let sync = sync_for(&client, &context, &settings);

    let version = match save_id {
        Some(id) => client
            .list_saves(game_id)
            .await?
            .into_iter()
            .find(|v| v.id == id)
            .ok_or_else(|| CommandError::Message("That save version is gone.".into()))?,
        None => sync
            .newest_remote(game_id)
            .await?
            .ok_or_else(|| CommandError::Message("There is no save on the server yet.".into()))?,
    };

    sync.fetch(game_id, version.id).await?;
    restore(app, state, &context, &title).await?;

    let hash = save_sync::staged_hash(&context.saves_root, game_id)
        .await
        .unwrap_or(None);
    record_sync(state, game_id, Some(version.id), hash, context.platform()).await;

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
            record.saves.ludusavi_title = title;
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
            record.save_redirects = redirects;
        })
        .await;

    state_of(&app, &state, game_id).await
}

#[tauri::command]
pub async fn set_save_sync_settings(
    state: State<'_, AppState>,
    enabled: bool,
    on_launch: bool,
    on_exit: bool,
) -> CommandResult<()> {
    crate::ipc::mutate_settings(&state, |settings| {
        settings.save_sync_enabled = enabled;
        settings.sync_saves_on_launch = on_launch;
        settings.sync_saves_on_exit = on_exit;
    })
    .await
}

#[tauri::command]
pub async fn delete_save_version(
    state: State<'_, AppState>,
    game_id: i64,
    save_id: i64,
) -> CommandResult<()> {
    let client = state.client().await.ok_or(CommandError::NotConnected)?;
    client.delete_save(game_id, save_id).await?;
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
