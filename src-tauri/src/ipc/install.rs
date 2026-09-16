//! Installing and uninstalling: moving unpacked files into place, running setup programs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gameyfin_core::setups::SetupRole;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::library::{discard_download_if_asked, extract_download};
use super::{contained, file_label, notify, notify_state};
use crate::error::{blocking, CommandError, CommandResult, Context};
use crate::library_state::{
    relative_to, scan_download_setups, scan_setups, Activity, GameRecord, Stage,
};
use crate::settings::InstallerMemoryLimit;
use crate::state::AppState;

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallOption {
    pub key: String,
    pub label: String,
    pub description: String,
    /// Hands control to a setup wizard, which asks where to install.
    pub interactive: bool,
    /// Why the option cannot be used yet.
    pub blocked_by: Option<String>,
}

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallPlan {
    pub payload: String,
    pub options: Vec<InstallOption>,
    pub default_install_dir: String,
    /// The path to paste into a setup wizard; `None` on Windows or when nothing will ask.
    pub windows_install_path: Option<String>,
    pub needs_install_path: bool,
    /// Setup programs in the unpacked files, in the order to run them.
    pub setups: Vec<SetupProgram>,
    /// Where a file picker should start: the unpacked files, else the download folder.
    pub browse_dir: Option<String>,
}

/// A setup program that came with a game, as the chooser offers it.
#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupProgram {
    /// Relative to the unpacked download, or to the installed game's folder.
    pub path: String,
    pub role: SetupRole,
    /// A patch the game installer already includes.
    pub superseded: bool,
    pub installed: bool,
    /// Has silent switches, so it installs without asking anything. Not checked for unmatched ones.
    pub silent: bool,
    /// What an automatic install would run now.
    pub recommended: bool,
    /// Recognised by name as a setup, patch or DLC. The rest is every other program in the download.
    pub matched: bool,
}

#[tauri::command]
pub async fn install_options(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<InstallPlan> {
    use gameyfin_core::InstallMethod;

    let record = state.library().record(game_id);
    let install_dir = super::install_dir_for(&state, game_id).await?;
    let (runtime, can_run_windows) = windows_runtime(&state, game_id).await;
    let blocked = |needs_windows: bool| {
        (needs_windows && !can_run_windows).then(gameyfin_core::windows_runtime_hint)
    };
    let option = |method: InstallMethod, key: String, label: String, blocked_by| InstallOption {
        key,
        label,
        description: method_description(method).to_string(),
        interactive: method.is_interactive(),
        blocked_by,
    };

    let (payload, options, setups) = if record.existing_staging().is_some() {
        let options = vec![option(
            InstallMethod::CopyExecutable,
            "move".into(),
            "Move into your games folder".into(),
            None,
        )];
        let setups = setup_programs(&state, game_id).await;
        (
            "unpacked files".to_string(),
            options,
            setups.into_iter().map(|(setup, _)| setup).collect(),
        )
    } else {
        let archive = record
            .existing_archive()
            .ok_or_else(|| CommandError::msg("This game has not been downloaded yet."))?;
        let payload = blocking("could not inspect the download", move || {
            gameyfin_core::classify(&archive)
        })
        .await?;
        let methods = if payload.is_archive() {
            vec![InstallMethod::Extract]
        } else {
            gameyfin_core::methods_for(payload, cfg!(windows))
        };
        let options = methods
            .into_iter()
            .map(|method| {
                let needs_runtime = method == InstallMethod::RunWindowsInstallerViaProton;
                option(
                    method,
                    method.key().to_string(),
                    method_label(method, runtime.as_ref()).to_string(),
                    blocked(needs_runtime),
                )
            })
            .collect();
        (payload.label().to_string(), options, Vec::new())
    };

    let needs_install_path =
        !setups.is_empty() || options.iter().any(|o: &InstallOption| o.interactive);
    let title = state.title(game_id).await;
    Ok(InstallPlan {
        payload,
        windows_install_path: needs_install_path
            .then(|| windows_install_path(&title))
            .flatten(),
        needs_install_path,
        options,
        default_install_dir: install_dir.to_string_lossy().into_owned(),
        browse_dir: browse_dir_for(&record),
        setups,
    })
}

/// Setups from the unpacked download, then the installed folder, planned as one set.
async fn setup_programs(state: &AppState, game_id: i64) -> Vec<(SetupProgram, PathBuf)> {
    let record = state.library().record(game_id);
    let title = state.title(game_id).await;
    let sources = [
        record
            .existing_staging()
            .map(|dir| (dir, record.staging_setups.clone())),
        record
            .install_dir
            .clone()
            .filter(|dir| dir.is_dir())
            .map(|dir| (dir, record.setup_candidates.clone())),
    ];
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for (base, relatives) in sources.into_iter().flatten() {
        for relative in relatives {
            if found.iter().any(|(known, _)| *known == relative) {
                continue;
            }
            if let Some(path) = contained(&base, &relative).ok().filter(|p| p.is_file()) {
                found.push((relative, path));
            }
        }
    }

    let relatives: Vec<String> = found.iter().map(|(relative, _)| relative.clone()).collect();
    let plan = gameyfin_core::setups::plan(&relatives, &title);
    let planned: Vec<_> = plan
        .setups
        .iter()
        .filter_map(|setup| {
            let (_, path) = found.iter().find(|(relative, _)| *relative == setup.path)?;
            Some((setup.clone(), path.clone()))
        })
        .collect();
    let programs: Vec<PathBuf> = planned.iter().map(|(_, path)| path.clone()).collect();
    let silent: Vec<bool> = tokio::task::spawn_blocking(move || {
        programs
            .iter()
            .map(|p| gameyfin_core::identify(p).is_ok_and(|kind| kind.runs_unattended()))
            .collect()
    })
    .await
    .unwrap_or_default();

    let game_installed = record.is_installed();
    let games = plan
        .setups
        .iter()
        .filter(|s| s.role == SetupRole::Game)
        .count();
    let mut listed: Vec<(SetupProgram, PathBuf)> = planned
        .into_iter()
        .enumerate()
        .map(|(i, (setup, path))| {
            // A game installed before setups were tracked still has its game setup behind it.
            let installed = record.installed_setups.contains(&setup.path)
                || (setup.role == SetupRole::Game && game_installed && games == 1);
            let recommended = !installed
                && !setup.superseded
                && setup.role != SetupRole::Other
                && if game_installed {
                    setup.role != SetupRole::Game
                } else {
                    plan.confident
                };
            let program = SetupProgram {
                silent: silent.get(i).copied().unwrap_or(false),
                path: setup.path,
                role: setup.role,
                superseded: setup.superseded,
                installed,
                recommended,
                matched: true,
            };
            (program, path)
        })
        .collect();
    listed.extend(other_programs(&record, &found).await);
    listed
}

/// Every other program in the unpacked download, unticked, for when names gave nothing away.
async fn other_programs(
    record: &GameRecord,
    known: &[(String, PathBuf)],
) -> Vec<(SetupProgram, PathBuf)> {
    let Some(dir) = record.existing_staging() else {
        return Vec::new();
    };
    let scan = dir.clone();
    let found = tokio::task::spawn_blocking(move || {
        gameyfin_core::executable::find_download_programs(&scan).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    found
        .into_iter()
        .filter(|path| !known.iter().any(|(_, matched)| matched == path))
        .filter_map(|path| {
            let relative = relative_to(&path, &dir)?;
            let program = SetupProgram {
                installed: record.installed_setups.contains(&relative),
                path: relative,
                role: SetupRole::Other,
                superseded: false,
                silent: false,
                recommended: false,
                matched: false,
            };
            Some((program, path))
        })
        .collect()
}

/// Setups in the unpacked download an automatic install would still run.
pub fn pending_setups(record: &GameRecord, title: &str) -> usize {
    if record.existing_staging().is_none() {
        return 0;
    }
    gameyfin_core::setups::plan(&record.staging_setups, title)
        .runnable()
        .filter(|setup| !record.installed_setups.contains(&setup.path))
        .count()
}

/// The detected runtime, and whether any can run a Windows program.
async fn windows_runtime(
    state: &AppState,
    game_id: i64,
) -> (Option<gameyfin_core::WindowsRuntime>, bool) {
    if cfg!(windows) {
        return (None, true);
    }
    let ctx = crate::proton::runtime_context(state, Some(game_id), None).await;
    // umu downloads a Proton build on first run, so it counts before one exists.
    let umu_ready = ctx.umu_launcher.is_some();
    let found = tokio::task::spawn_blocking(move || gameyfin_core::detect_windows_runtime(&ctx))
        .await
        .ok()
        .flatten();
    let available = umu_ready || found.is_some();
    (found, available)
}

fn method_label(
    method: gameyfin_core::InstallMethod,
    runtime: Option<&gameyfin_core::WindowsRuntime>,
) -> &'static str {
    use gameyfin_core::InstallMethod::*;
    match method {
        Extract => "Extract",
        RunWindowsInstaller => "Run the installer",
        // No runtime yet reads as Proton: umu downloads one on first run.
        RunWindowsInstallerViaProton => match runtime {
            Some(r) if r.is_wine() => "Run the installer with Wine",
            _ => "Run the installer with Proton",
        },
        CopyExecutable => "Move into your games folder",
    }
}

fn method_description(method: gameyfin_core::InstallMethod) -> &'static str {
    use gameyfin_core::InstallMethod::*;
    match method {
        Extract => "Unpack the archive so its contents can be inspected and installed.",
        RunWindowsInstaller => "Start the game's own setup program. Paste the install path when it asks.",
        RunWindowsInstallerViaProton => {
            "Start the game's own setup program in a Windows compatibility layer. Paste the install path when it asks."
        }
        CopyExecutable => "For a game that runs straight from its files, with no setup step.",
    }
}

fn browse_dir_for(record: &GameRecord) -> Option<String> {
    record
        .existing_staging()
        .or_else(|| {
            record
                .archive_path
                .as_ref()?
                .parent()
                .map(Path::to_path_buf)
        })
        .map(|d| d.to_string_lossy().into_owned())
}

/// The game's folder as a program in its prefix sees it; `None` on Windows.
fn windows_install_path(title: &str) -> Option<String> {
    (!cfg!(windows)).then(|| gameyfin_core::prefix::game_windows_path(title))
}

/// `method` is `extract`, `move`, or a payload method key. Setups go through `install_setups`.
#[tauri::command]
pub async fn install_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    method: Option<String>,
    delete_archive: Option<bool>,
    delete_download: Option<bool>,
) -> CommandResult<()> {
    let method = match method {
        Some(method) => method,
        None => default_method(&state, game_id).await?,
    };
    if method == "extract" {
        return extract_download(&app, game_id, delete_archive).await;
    }

    let record = state.library().record(game_id);
    let install_dir = super::install_dir_for(&state, game_id).await?;
    let staging = || {
        record
            .existing_staging()
            .ok_or_else(|| CommandError::msg("Nothing has been unpacked yet."))
    };

    if method == "move" {
        return move_into_place(&app, game_id, &staging()?, &install_dir, delete_download).await;
    }

    let archive = record
        .existing_archive()
        .ok_or_else(|| CommandError::msg("This game has not been downloaded yet."))?;
    match gameyfin_core::InstallMethod::from_key(&method) {
        Some(gameyfin_core::InstallMethod::CopyExecutable) => {
            copy_executable(&app, game_id, &archive, &install_dir, delete_download).await
        }
        Some(
            gameyfin_core::InstallMethod::RunWindowsInstaller
            | gameyfin_core::InstallMethod::RunWindowsInstallerViaProton,
        ) => run_setups(&app, game_id, SetupRun::single(archive, delete_download)).await,
        _ => Err(CommandError::msg(format!(
            "{method} is not a way to install this."
        ))),
    }
}

/// With no method named, whatever the chooser would put first for what is on disk.
async fn default_method(state: &AppState, game_id: i64) -> CommandResult<String> {
    let record = state.library().record(game_id);
    if record.existing_staging().is_some() {
        return Ok("move".to_string());
    }
    let archive = record
        .existing_archive()
        .ok_or_else(|| CommandError::msg("This game has not been downloaded yet."))?;
    let payload = blocking("could not inspect the download", move || {
        gameyfin_core::classify(&archive)
    })
    .await?;
    if payload.is_archive() {
        return Ok("extract".to_string());
    }
    gameyfin_core::methods_for(payload, cfg!(windows))
        .first()
        .map(|method| method.key().to_string())
        .ok_or_else(|| {
            CommandError::msg(format!(
                "Gameyfin does not know how to install a {}.",
                payload.label()
            ))
        })
}

fn claim(
    state: &AppState,
    game_id: i64,
    activity: Activity,
) -> CommandResult<crate::library_state::Claim> {
    state
        .library()
        .claim(game_id, activity)
        .ok_or_else(|| CommandError::msg("Wait for what this game is doing to finish."))
}

/// Moves unpacked files into the games folder. The old install is replaced only once the
/// new files are completely in place beside it.
async fn move_into_place(
    app: &AppHandle,
    game_id: i64,
    source: &Path,
    install_dir: &Path,
    delete_download: Option<bool>,
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let _claim = claim(&state, game_id, Activity::installing())?;
    notify_state(app, game_id);
    tracing::info!(
        game_id,
        ?source,
        ?install_dir,
        "moving unpacked files into place"
    );

    let parent = install_dir
        .parent()
        .ok_or_else(|| CommandError::msg("The games folder has no parent."))?;
    let name = file_label(install_dir);
    let incoming = parent.join(format!(".incoming-{name}"));
    let outgoing = parent.join(format!(".outgoing-{name}"));
    tokio::fs::create_dir_all(parent)
        .await
        .context(format!("could not create {}", parent.display()))?;
    let _ = tokio::fs::remove_dir_all(&incoming).await;

    // A rename is instant on one filesystem; Downloads may be on another, which needs a copy.
    if tokio::fs::rename(source, &incoming).await.is_err() {
        let measured = source.to_path_buf();
        let total = tokio::task::spawn_blocking(move || {
            gameyfin_core::extract::directory_size(&measured).unwrap_or(0)
        })
        .await
        .unwrap_or(0);
        let watcher = watch_written(app, game_id, vec![incoming.clone()], 0, total);
        let (from, to) = (source.to_path_buf(), incoming.clone());
        let copied = blocking("could not copy the files", move || copy_tree(&from, &to)).await;
        watcher.abort();
        if let Err(e) = copied {
            let _ = tokio::fs::remove_dir_all(&incoming).await;
            return Err(e);
        }
        let _ = tokio::fs::remove_dir_all(source).await;
    }
    let replacing = install_dir.exists();
    if replacing {
        tokio::fs::rename(install_dir, &outgoing)
            .await
            .context("could not set the old install aside")?;
    }
    tokio::fs::rename(&incoming, install_dir)
        .await
        .context("could not put the files in place")?;
    if replacing {
        let _ = tokio::fs::remove_dir_all(&outgoing).await;
    }

    if let Some(download_dir) = source.parent() {
        if super::dir_is_empty(download_dir).await {
            let _ = tokio::fs::remove_dir(download_dir).await;
        }
    }
    state
        .library()
        .update_record(game_id, |r| {
            r.extracted_dir = None;
            r.staging_setups.clear();
        })
        .await;
    finish_install(&state, game_id, install_dir).await;
    discard_download_if_asked(app, game_id, install_dir, delete_download).await;
    notify_state(app, game_id);
    Ok(())
}

/// Moves everything in `from` into `to`, replacing what is already there, then removes `from`.
fn merge_into(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() && target.is_dir() {
            merge_into(&entry.path(), &target)?;
            continue;
        }
        if target.is_dir() {
            std::fs::remove_dir_all(&target)?;
        }
        // A rename cannot cross drives, where the prefix may sit apart from the game.
        if std::fs::rename(entry.path(), &target).is_err() {
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &target)?;
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::copy(entry.path(), &target)?;
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    std::fs::remove_dir(from)
}

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

/// Puts a native program in place and marks it runnable.
async fn copy_executable(
    app: &AppHandle,
    game_id: i64,
    source: &Path,
    install_dir: &Path,
    delete_download: Option<bool>,
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let _claim = claim(&state, game_id, Activity::installing())?;
    let name = keep_program(source, install_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(
            install_dir.join(&name),
            std::fs::Permissions::from_mode(0o755),
        )
        .await;
    }
    record_install(&state, game_id, install_dir, Some(name)).await;
    state.library().clear_activity(game_id);
    discard_download_if_asked(app, game_id, install_dir, delete_download).await;
    notify_state(app, game_id);
    Ok(())
}

/// Copies a program into the install folder, keeping the download where Downloads expects it.
async fn keep_program(program: &Path, install_dir: &Path) -> CommandResult<String> {
    tokio::fs::create_dir_all(install_dir)
        .await
        .context(format!("could not create {}", install_dir.display()))?;
    let name = file_label(program);
    tokio::fs::copy(program, install_dir.join(&name))
        .await
        .context("could not copy the program")?;
    Ok(name)
}

async fn record_install(
    state: &AppState,
    game_id: i64,
    install_dir: &Path,
    executable: Option<String>,
) {
    state
        .library()
        .update_record(game_id, |r| {
            r.install_dir = Some(install_dir.to_path_buf());
            r.executable = executable;
            r.installed_at = Some(super::now_iso8601());
        })
        .await;
}

/// Records a completed install and ends the game's busy state.
pub async fn finish_install(state: &AppState, game_id: i64, install_dir: &Path) {
    adopt_install(state, game_id, install_dir).await;
    state.library().clear_activity(game_id);
}

/// Records what is in the folder, leaving the busy state to a run that has more to do.
/// A chosen executable that still exists is kept.
async fn adopt_install(state: &AppState, game_id: i64, install_dir: &Path) {
    let dir = install_dir.to_path_buf();
    // The title picks `Binaries/Win64/Game.exe` over a launcher helper beside it.
    let title = state.title(game_id).await;
    let (detected, setups) = tokio::task::spawn_blocking(move || {
        let detected = match gameyfin_core::executable::detect(&dir, &title) {
            Ok(gameyfin_core::Detection::Confident(path)) => relative_to(&path, &dir),
            _ => None,
        };
        (detected, scan_setups(&dir))
    })
    .await
    .unwrap_or_default();
    let staging = state.library().record(game_id).existing_staging();
    let staging_setups = match staging {
        Some(dir) => tokio::task::spawn_blocking(move || scan_download_setups(&dir))
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let previous = state.library().record(game_id);
    let kept = previous.executable.filter(|exe| {
        previous.install_dir.as_deref() == Some(install_dir) && install_dir.join(exe).is_file()
    });
    let executable = kept.or(detected);
    tracing::info!(game_id, ?install_dir, ?executable, "install recorded");
    state
        .library()
        .update_record(game_id, |r| {
            r.setup_candidates = setups;
            r.staging_setups = staging_setups;
        })
        .await;
    record_install(state, game_id, install_dir, executable).await;
}

const STUCK_AFTER: Duration = Duration::from_secs(180);
const MEASURE_EVERY: Duration = Duration::from_secs(2);

async fn size_of(dirs: Vec<PathBuf>) -> u64 {
    tokio::task::spawn_blocking(move || {
        dirs.iter()
            .filter_map(|dir| gameyfin_core::extract::directory_size(dir).ok())
            .sum()
    })
    .await
    .unwrap_or(0)
}

/// Shows bytes written to `dirs` past `baseline` until aborted. Zero `total_bytes` is unknown.
fn watch_written(
    app: &AppHandle,
    game_id: i64,
    dirs: Vec<PathBuf>,
    baseline: u64,
    total_bytes: u64,
) -> tauri::async_runtime::JoinHandle<()> {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (mut last, mut at) = (0, std::time::Instant::now());
        loop {
            tokio::time::sleep(MEASURE_EVERY).await;
            let written = size_of(dirs.clone()).await.saturating_sub(baseline);
            let elapsed = at.elapsed().as_secs_f64().max(f64::EPSILON);
            let progress = crate::progress::TransferProgress {
                received_bytes: written,
                total_bytes,
                bytes_per_second: written.saturating_sub(last) as f64 / elapsed,
            };
            (last, at) = (written, std::time::Instant::now());
            let library = app.state::<AppState>().library().clone();
            if library.show_progress(game_id, progress) {
                notify_state(&app, game_id);
            }
        }
    })
}

/// One setup program in a run.
pub struct SetupStep {
    pub program: PathBuf,
    /// A failed game setup ends the run; a failed patch or DLC does not.
    pub role: SetupRole,
}

pub struct SetupRun {
    /// In the order they run.
    pub steps: Vec<SetupStep>,
    /// Reruns as administrator, through the shell.
    pub elevated: bool,
    /// Passes the toolkits' silent switches and notifies when done.
    pub silent: bool,
    /// Overrides the setting.
    pub delete_download: Option<bool>,
    /// Setups an unattended run left for the user, mentioned once the rest are in.
    pub left_for_user: usize,
}

impl SetupRun {
    fn single(program: PathBuf, delete_download: Option<bool>) -> Self {
        Self {
            steps: vec![SetupStep {
                program,
                role: SetupRole::Game,
            }],
            elevated: false,
            silent: false,
            delete_download,
            left_for_user: 0,
        }
    }
}

/// Runs setup programs one after another and adopts what they install. Returns once the first
/// is started, so a prefix that cannot be built is reported to the caller.
async fn run_setups(app: &AppHandle, game_id: i64, run: SetupRun) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let Some(first) = run.steps.first() else {
        return Err(CommandError::msg("Choose a setup program to run."));
    };
    for step in &run.steps {
        if !step.program.is_file() {
            return Err(CommandError::msg(format!(
                "{} is no longer there.",
                step.program.display()
            )));
        }
        // Checked before a prefix is built: Wine would only say `Bad format` minutes later.
        super::ensure_windows_program(&step.program, "download")?;
    }
    let install_dir = super::install_dir_for(&state, game_id).await?;
    let claim = claim(
        &state,
        game_id,
        Activity::preparing("Preparing the installer"),
    )?;
    let prepared = prepare_installer(
        app,
        game_id,
        &first.program,
        &install_dir,
        run.silent,
        false,
    )
    .await?;
    let record = state.library().record(game_id);
    let bases: Vec<PathBuf> = [record.extracted_dir, Some(install_dir.clone())]
        .into_iter()
        .flatten()
        .collect();

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let total = run.steps.len();
        let mut prepared = Some(prepared);
        let mut failures = Vec::new();
        for (index, step) in run.steps.iter().enumerate() {
            let name = file_label(&step.program);
            let label = (total > 1).then(|| format!("{name} ({} of {total})", index + 1));
            let first = prepared.take();
            if first.is_none() {
                let message = format!("Preparing {}", label.as_deref().unwrap_or(&name));
                state
                    .library()
                    .set_activity(game_id, Activity::preparing(message));
                notify_state(&app, game_id);
            }
            let outcome = run_step(&app, game_id, step, &install_dir, &run, first, label).await;
            match outcome {
                Ok(_) => {
                    let relative = bases
                        .iter()
                        .find_map(|base| relative_to(&step.program, base))
                        .unwrap_or(name);
                    state
                        .library()
                        .update_record(game_id, |r| {
                            if !r.installed_setups.contains(&relative) {
                                r.installed_setups.push(relative);
                            }
                        })
                        .await;
                }
                Err(failure) => {
                    let ends_run = failure.ends_run || step.role == SetupRole::Game;
                    failures.push(if total > 1 {
                        format!("{name}: {}", failure.message)
                    } else {
                        failure.message
                    });
                    if ends_run {
                        break;
                    }
                }
            }
        }

        let outcome = if failures.is_empty() {
            // Only a clean run may take the download a retry would need.
            discard_download_if_asked(&app, game_id, &install_dir, run.delete_download).await;
            state.library().clear_activity(game_id);
            Ok(())
        } else {
            let message = failures.join("\n\n");
            state
                .library()
                .fail(game_id, Stage::Install, message.clone());
            Err(message)
        };
        notify_state(&app, game_id);
        drop(claim);

        if !run.silent {
            return;
        }
        let title = state.title(game_id).await;
        match outcome {
            Ok(()) => crate::notify::install_finished(&app, game_id, &title).await,
            Err(message) => crate::notify::failed(&app, game_id, "Install", &title, &message).await,
        }
        if run.left_for_user > 0 {
            crate::notify::setup_needed(
                &app,
                game_id,
                "/installed",
                &format!("{title} has setup programs that cannot install on their own. Run them from Installed."),
            )
            .await;
        }
    });
    Ok(())
}

/// How one attempt at a setup program runs.
struct Attempt {
    silent: bool,
    elevated: bool,
    /// Asked for once a 32-bit program found no 32-bit graphics driver without it.
    wow64: bool,
    /// Shown on the row while it runs.
    label: Option<String>,
}

/// Runs one setup program, and again under WOW64 when it had no 32-bit graphics driver. A patch or
/// DLC that exits cleanly but changes nothing in the game's folder is reported as not applied.
async fn run_step(
    app: &AppHandle,
    game_id: i64,
    step: &SetupStep,
    install_dir: &Path,
    run: &SetupRun,
    mut prepared: Option<PreparedInstaller>,
    label: Option<String>,
) -> Result<String, StepFailure> {
    // A clean exit does not say a patch or DLC did anything, so the game's folder is compared.
    let judged = matches!(step.role, SetupRole::Patch | SetupRole::Dlc);
    let before = if judged {
        folder_state(install_dir).await
    } else {
        None
    };
    let wow64_already = app
        .state::<AppState>()
        .library()
        .record(game_id)
        .launch_toggles
        .wow64;
    let mut attempt = Attempt {
        silent: run.silent,
        elevated: run.elevated,
        wow64: false,
        label,
    };
    loop {
        let mut lacks_gl = false;
        let ready = match prepared.take() {
            Some(ready) => Ok(ready),
            None => prepare_installer(
                app,
                game_id,
                &step.program,
                install_dir,
                attempt.silent,
                attempt.wow64,
            )
            .await
            .map_err(|e| StepFailure {
                message: e.to_string(),
                ends_run: false,
            }),
        };
        let result = match ready {
            Ok(ready) => {
                execute_installer(
                    app,
                    game_id,
                    &step.program,
                    install_dir,
                    ready,
                    &attempt,
                    &mut lacks_gl,
                )
                .await
            }
            Err(failure) => Err(failure),
        };
        if result.as_ref().is_err_and(|failure| failure.ends_run) {
            return result;
        }
        let unchanged =
            result.is_ok() && before.is_some() && folder_state(install_dir).await == before;
        if result.is_ok() && !unchanged {
            return result;
        }

        if lacks_gl && !attempt.wow64 && !wow64_already {
            tracing::info!(game_id, program = ?step.program, "no 32-bit graphics driver; running it again with WOW64");
            attempt.wow64 = true;
            continue;
        }
        let output = match result {
            Ok(output) => output,
            Err(mut failure) => {
                let advice = gameyfin_core::diagnose::MISSING_32BIT_GL;
                if lacks_gl && !failure.message.contains(advice) {
                    failure.message.push_str("\n\n");
                    failure.message.push_str(advice);
                }
                return Err(failure);
            }
        };
        tracing::warn!(game_id, program = ?step.program, "setup finished without changing the game's folder");
        return Err(StepFailure {
            message: nothing_changed(&output, lacks_gl),
            ends_run: false,
        });
    }
}

struct StepFailure {
    message: String,
    /// Stopped by the user, which stops the rest of the run too.
    ends_run: bool,
}

/// An installer command ready to start, with what watching it needs.
struct PreparedInstaller {
    command: gameyfin_core::ResolvedCommand,
    kind: gameyfin_core::InstallerKind,
    unarc_prefix: Option<PathBuf>,
    /// `C:\Games\<title>` in the prefix, linked to the game's folder.
    game_link: Option<PathBuf>,
    watched: Vec<PathBuf>,
    limit: InstallerMemoryLimit,
    cap_mib: Option<u64>,
}

/// Builds the prefix and the command. `silent` adds the toolkit's unattended switches.
async fn prepare_installer(
    app: &AppHandle,
    game_id: i64,
    program: &Path,
    install_dir: &Path,
    silent: bool,
    wow64: bool,
) -> CommandResult<PreparedInstaller> {
    let state = app.state::<AppState>();
    let title = state.title(game_id).await;
    let mut unarc_prefix = None;
    let mut game_link = None;
    let mut config = if gameyfin_core::needs_proton(program) {
        // The prefix must exist before any drive is mapped, or wineboot skips building it.
        let (runtime, prefix, _) =
            super::launch::ready_windows_prefix(app, game_id, Some(program)).await?;
        unarc_prefix = Some(prefix.clone());

        // Where installers are sent, and where repacks and their patches look for the game.
        let link = gameyfin_core::prefix::game_link(&prefix, &title);
        if crate::library_state::has_content(install_dir) {
            settle_game_link(&link, install_dir).await;
        } else {
            clear_game_link(&link).await;
        }
        game_link = Some(link);
        // Run through a drive letter: installer batch scripts choke on our `(id)` folder names.
        let mut target = program.to_path_buf();
        if let (Some(source_dir), Some(name)) = (program.parent(), program.file_name()) {
            let drive = gameyfin_core::prefix::SOURCE_DRIVE;
            match gameyfin_core::map_drive_letter(&prefix, drive, source_dir, false) {
                Ok(_) => target = format!("{drive}:\\{}", name.to_string_lossy()).into(),
                Err(e) => tracing::warn!(game_id, error = %e, "could not map the installer drive"),
            }
        }
        tracing::info!(
            game_id,
            runtime = runtime.kind(),
            ?prefix,
            ?program,
            "running installer"
        );
        let mut config =
            gameyfin_core::LaunchConfig::for_windows_program_unattended(target, prefix, &runtime);
        config.working_dir = program.parent().map(Path::to_path_buf);
        super::launch::apply_game_runtime(&state, game_id, &runtime, &mut config).await;
        super::launch::apply_game_options(&state.library().record(game_id), &mut config);
        if wow64 {
            config
                .environment
                .insert("PROTON_USE_WOW64".to_string(), "1".to_string());
        }
        config
    } else {
        gameyfin_core::LaunchConfig::native(program)
    };

    // Pass the destination where the toolkit accepts one, to a folder name `cmd` can handle.
    let kind = gameyfin_core::identify(program).unwrap_or(gameyfin_core::InstallerKind::Unknown);
    let destination = match &game_link {
        Some(_) => gameyfin_core::prefix::game_windows_path(&title),
        None => install_dir.to_string_lossy().into_owned(),
    };
    if silent {
        config
            .arguments
            .extend(state.settings().unattended_args(kind));
    }
    config.arguments.extend(kind.destination_args(&destination));
    // The user's own options last, so their flags win over the toolkit defaults.
    config.arguments.extend(gameyfin_core::arguments::split(
        &state.library().record(game_id).installer_arguments,
    ));

    let command =
        gameyfin_core::resolve_command(&config).context("could not build the installer command")?;
    tracing::debug!(game_id, program = ?command.program, args = ?command.args, kind = kind.label(), "installer command");
    tokio::fs::create_dir_all(install_dir)
        .await
        .context(format!("could not create {}", install_dir.display()))?;

    state
        .library()
        .update_record(game_id, |r| r.elevation_program = None)
        .await;
    // A first install writes inside the prefix until it is moved into place.
    let mut watched = vec![install_dir.to_path_buf()];
    if let Some(link) = game_link.as_ref().filter(|link| !link.is_symlink()) {
        watched.push(link.clone());
    }

    // Caps the whole process tree, which the installer's own RAM option does not reach.
    let limit = state.settings().installer_memory_limit;
    let cap_mib = if cfg!(windows) {
        None
    } else {
        limit.resolve(gameyfin_core::process::total_memory_bytes())
    };
    Ok(PreparedInstaller {
        command,
        kind,
        unarc_prefix,
        game_link,
        watched,
        limit,
        cap_mib,
    })
}

/// Clears `C:\Games\<title>` for a first install. A folder that exists makes Inno Setup ask
/// whether to install into it anyway, so the installer makes its own, moved into place after.
/// One with files in it stays, for the move to pick up.
async fn clear_game_link(link: &Path) {
    let link = link.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir(&link),
        Ok(_) => std::fs::remove_file(&link),
        Err(e) => Err(e),
    })
    .await;
}

/// Links `C:\Games\<title>` to the game's folder. Files an installer wrote to a real folder
/// there, having replaced the link, are moved into the game's folder first.
async fn settle_game_link(link: &Path, install_dir: &Path) {
    let (link, install_dir) = (link.to_path_buf(), install_dir.to_path_buf());
    let settled = tokio::task::spawn_blocking(move || {
        if std::fs::symlink_metadata(&link).is_ok_and(|m| m.is_dir()) {
            merge_into(&link, &install_dir)?;
        }
        gameyfin_core::prefix::link_folder(&link, &install_dir)
            .map_err(|e| std::io::Error::other(e.to_string()))
    })
    .await;
    if !matches!(settled, Ok(Ok(()))) {
        tracing::warn!(?settled, "could not link the game into its prefix");
    }
}

/// Runs one prepared installer to the end and adopts whatever it installed.
async fn execute_installer(
    app: &AppHandle,
    game_id: i64,
    program: &Path,
    install_dir: &Path,
    prepared: PreparedInstaller,
    attempt: &Attempt,
    lacks_32bit_gl: &mut bool,
) -> Result<String, StepFailure> {
    let (elevated, step) = (attempt.elevated, attempt.label.clone());
    let state = app.state::<AppState>();
    let PreparedInstaller {
        command,
        kind,
        unarc_prefix,
        game_link,
        watched,
        limit,
        cap_mib,
    } = prepared;
    let failed = |message: String| StepFailure {
        message,
        ends_run: false,
    };

    let baseline = size_of(watched.clone()).await;
    state.library().set_activity(
        game_id,
        Activity::Installing {
            progress: None,
            step,
        },
    );
    notify_state(app, game_id);
    let address_space = cap_mib.map(|mib| {
        tracing::info!(game_id, mib, "capping the installer's address space");
        mib * 1024 * 1024
    });

    let stopper = state.processes.register(game_id);
    let watcher = watch_written(app, game_id, watched, baseline, 0);
    let unarc_guard =
        unarc_prefix.map(|prefix| tauri::async_runtime::spawn(gameyfin_core::unarc::guard(prefix)));
    let started = std::time::Instant::now();
    let run = if elevated {
        // Started by the shell, so nothing to capture and no limit to apply.
        gameyfin_core::run_elevated(&command).await
    } else {
        gameyfin_core::run_capturing_stoppable(&command, address_space, &stopper).await
    };
    if let Some(guard) = unarc_guard {
        guard.abort();
    }
    watcher.abort();
    let elapsed = started.elapsed();
    state.processes.finish(game_id);

    // Kept on success too: a patcher that found nothing to patch often says so and exits cleanly.
    let output = match &run {
        Ok(run) if run.success() => {
            tracing::debug!(game_id, stderr = %run.stderr, stdout = %run.stdout, "installer output");
            run.diagnostic_tail(12)
        }
        _ => String::new(),
    };
    *lacks_32bit_gl = run.as_ref().is_ok_and(|run| {
        gameyfin_core::diagnose::lacks_32bit_gl(&run.stderr)
            || gameyfin_core::diagnose::lacks_32bit_gl(&run.stdout)
    });
    let failure = match &run {
        Ok(run) if run.success() => None,
        Ok(run) => {
            tracing::debug!(game_id, stderr = %run.stderr, stdout = %run.stdout, "installer output");
            let tail = run.diagnostic_tail(12);
            let stopped = stopper.was_stopped();
            let mut message = match (stopped, run.status) {
                (true, _) => "Installation stopped.".to_string(),
                (false, None) => "The installer was ended before it finished.".to_string(),
                (false, Some(code)) if tail.trim().is_empty() => {
                    format!("The installer exited with code {code} and no output.")
                }
                (false, Some(code)) => {
                    format!("The installer exited with code {code}:\n{tail}")
                }
            };
            // A recognised cause beats the memory guess.
            // From the whole output: the driver lines come well before the tail.
            let advice = if *lacks_32bit_gl {
                Some(gameyfin_core::diagnose::MISSING_32BIT_GL.to_string())
            } else {
                gameyfin_core::diagnose::explain(&tail)
                    .map(str::to_string)
                    .or_else(|| memory_advice(limit, cap_mib, &tail, elapsed, stopped))
            };
            if let Some(advice) = advice {
                message.push_str("\n\n");
                message.push_str(&advice);
            }
            Some(StepFailure {
                message,
                ends_run: stopped,
            })
        }
        // Not a broken download: remembering the program lets the UI offer the elevated retry.
        Err(gameyfin_core::CoreError::ElevationRequired { .. }) => {
            let refused = program.to_path_buf();
            state
                .library()
                .update_record(game_id, |r| r.elevation_program = Some(refused))
                .await;
            Some(failed(format!(
                "{} needs to run as administrator.",
                file_label(program)
            )))
        }
        Err(e) => Some(failed(e.to_string())),
    };
    if let Some(failure) = failure {
        tracing::error!(game_id, ?elapsed, "installer failed: {}", failure.message);
        return Err(failure);
    }

    // An installer that replaced the link with a folder of its own wrote there instead.
    if let Some(link) = &game_link {
        settle_game_link(link, install_dir).await;
    }

    if crate::library_state::has_content(install_dir) {
        adopt_install(&state, game_id, install_dir).await;
        return Ok(output);
    }
    if kind == gameyfin_core::InstallerKind::Unknown && elapsed >= Duration::from_secs(3) {
        // No known toolkit, a clean exit, and a real run: this was the game itself.
        tracing::info!(
            game_id,
            ?program,
            "treating the download as the game itself"
        );
        let name = keep_program(program, install_dir)
            .await
            .map_err(|e| failed(e.to_string()))?;
        record_install(&state, game_id, install_dir, Some(name)).await;
        return Ok(output);
    }
    Err(failed(
        "The installer finished without putting anything in the suggested folder. \
         If you chose a different location, use \"I installed it myself\"."
            .to_string(),
    ))
}

/// Files, bytes and the newest change under a folder, to tell whether a program touched it.
/// `None` when it cannot be read, which never compares equal.
async fn folder_state(dir: &Path) -> Option<(u64, u64, std::time::SystemTime)> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || scan_folder_state(&dir))
        .await
        .ok()
        .flatten()
}

fn scan_folder_state(dir: &Path) -> Option<(u64, u64, std::time::SystemTime)> {
    let (mut files, mut bytes, mut newest) = (0, 0, std::time::SystemTime::UNIX_EPOCH);
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).ok()?.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                pending.push(entry.path());
                continue;
            }
            files += 1;
            bytes += meta.len();
            if let Ok(modified) = meta.modified() {
                newest = newest.max(modified);
            }
        }
    }
    Some((files, bytes, newest))
}

fn nothing_changed(output: &str, lacks_32bit_gl: bool) -> String {
    let mut message =
        "It finished without changing anything in the game's folder, so it was not applied. \
         It may be for a different version of the game, or need to be run by hand."
            .to_string();
    if !output.trim().is_empty() {
        message.push_str("\n\nIt printed:\n");
        message.push_str(output.trim());
    }
    if lacks_32bit_gl {
        message.push_str("\n\n");
        message.push_str(gameyfin_core::diagnose::MISSING_32BIT_GL);
    }
    message
}

fn looks_like_memory_trouble(output: &str) -> bool {
    let lower = output.to_lowercase();
    // `unarc`/`ISDone` are what repack unpackers print when an allocation of any size fails.
    [
        "not enough memory",
        "out of memory",
        "cannot allocate",
        "unarc",
        "isdone",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Advice naming the memory setting to move, or `None` on Windows and for a quick cancel.
fn memory_advice(
    limit: InstallerMemoryLimit,
    cap_mib: Option<u64>,
    output: &str,
    elapsed: Duration,
    stopped: bool,
) -> Option<String> {
    use InstallerMemoryLimit::Auto;

    if cfg!(windows) {
        return None;
    }
    let certain = looks_like_memory_trouble(output);
    // No exit code tells a wedged installer from a change of mind, so time decides.
    if stopped && !certain && elapsed < STUCK_AFTER {
        return None;
    }
    let stuck = stopped && !certain;

    // 3 GB lets a hung FreeArc memory search finish.
    let fix = match (cap_mib, limit, certain, stuck) {
        (None, _, true, _) => None,
        (None, ..) => Some("set a memory limit, such as 3 GB".to_string()),
        (Some(_), Auto, true, _) => Some("pick a higher fixed memory limit or none".into()),
        (Some(_), Auto, ..) => Some("try a fixed memory limit, such as 3 GB".into()),
        (Some(mib), _, _, true) => Some(format!("try a memory limit other than {mib} MB")),
        (Some(mib), ..) => Some(format!("raise or remove the {mib} MB memory limit")),
    };
    let under = match (cap_mib, limit) {
        (None, _) => String::new(),
        (Some(mib), Auto) => format!(" under the automatic {mib} MB limit"),
        (Some(mib), _) => format!(" under the {mib} MB limit"),
    };
    Some(match fix {
        None => "This installer ran out of memory with nothing limiting it. Close other programs and try again.".into(),
        Some(fix) if stuck => format!("An installer that gets stuck is usually a memory problem. In Settings, Compatibility, {fix}."),
        Some(fix) if certain => format!("This installer ran out of memory{under}. In Settings, Compatibility, {fix}."),
        Some(fix) => format!("If the installer ran out of memory, in Settings, Compatibility, {fix}."),
    })
}

/// What a finished download does with automatic install on: an archive is unpacked, a program
/// that needs no answers is installed, and a setup program runs silently where it can.
pub async fn auto_install_download(
    app: &AppHandle,
    game_id: i64,
    title: &str,
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let archive = state
        .library()
        .record(game_id)
        .existing_archive()
        .ok_or_else(|| CommandError::msg("This game has not been downloaded yet."))?;

    let inspect = archive.clone();
    let payload = blocking("could not inspect the download", move || {
        gameyfin_core::classify(&inspect)
    })
    .await?;
    if payload.is_archive() {
        return extract_download(app, game_id, None).await;
    }

    let method = gameyfin_core::methods_for(payload, cfg!(windows))
        .into_iter()
        .next();
    tracing::info!(game_id, ?payload, ?method, "the download is not an archive");
    match method {
        Some(method) if !method.is_interactive() => {
            let install_dir = super::install_dir_for(&state, game_id).await?;
            copy_executable(app, game_id, &archive, &install_dir, None).await
        }
        Some(_) => {
            let step = SetupStep {
                program: archive,
                role: SetupRole::Game,
            };
            auto_run_setups(app, game_id, title, vec![step]).await
        }
        None => {
            let note = crate::notify::Note {
                category: crate::notify::Category::Action,
                title: "Downloaded".into(),
                body: format!(
                    "{title} is a {}. Choose how to install it in Downloads.",
                    payload.label()
                ),
                route: Some("/downloads"),
                game_id: Some(game_id),
            };
            crate::notify::send(app, note).await;
            Ok(())
        }
    }
}

/// The steps of a confident plan, resolved inside the folder that was scanned.
pub fn planned_steps(
    base: &Path,
    plan: &gameyfin_core::setups::SetupPlan,
) -> CommandResult<Vec<SetupStep>> {
    plan.runnable()
        .map(|setup| {
            Ok(SetupStep {
                program: contained(base, &setup.path)?,
                role: setup.role,
            })
        })
        .collect()
}

/// Runs setups silently. One without silent switches waits for the user, and so does the rest
/// when that one is the game.
pub async fn auto_run_setups(
    app: &AppHandle,
    game_id: i64,
    title: &str,
    steps: Vec<SetupStep>,
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let programs: Vec<PathBuf> = steps.iter().map(|s| s.program.clone()).collect();
    let kinds = blocking("could not inspect the setup programs", move || {
        programs
            .iter()
            .map(|p| gameyfin_core::identify(p))
            .collect::<gameyfin_core::CoreResult<Vec<_>>>()
    })
    .await?;
    let (silent, manual): (Vec<_>, Vec<_>) = steps
        .into_iter()
        .zip(kinds)
        .partition(|(_, kind)| kind.runs_unattended());
    let game_waits = manual.iter().any(|(s, _)| s.role == SetupRole::Game);
    let needs_runtime = silent
        .iter()
        .any(|(s, _)| gameyfin_core::needs_proton(&s.program));

    let waiting = if silent.is_empty() || game_waits {
        Some(format!(
            "{title} has a setup program that cannot install on its own. Run it from Downloads."
        ))
    } else if needs_runtime && !windows_runtime(&state, game_id).await.1 {
        Some(format!(
            "{title} has a setup program. {}",
            gameyfin_core::windows_runtime_hint()
        ))
    } else {
        None
    };
    tracing::info!(
        game_id,
        silent = silent.len(),
        manual = manual.len(),
        waiting = waiting.is_some(),
        "automatic setup"
    );
    if let Some(body) = waiting {
        crate::notify::setup_needed(app, game_id, "/downloads", &body).await;
        return Ok(());
    }
    let run = SetupRun {
        steps: silent.into_iter().map(|(step, _)| step).collect(),
        elevated: false,
        silent: true,
        delete_download: None,
        left_for_user: manual.len(),
    };
    run_setups(app, game_id, run).await
}

/// Moves the unpacked files into place after an automatic extraction.
pub async fn finish_auto_install(app: &AppHandle, game_id: i64) {
    let state = app.state::<AppState>();
    let title = state.title(game_id).await;
    let result = async {
        let install_dir = super::install_dir_for(&state, game_id).await?;
        let source = state
            .library()
            .record(game_id)
            .existing_staging()
            .ok_or_else(|| CommandError::msg("Nothing was unpacked."))?;
        move_into_place(app, game_id, &source, &install_dir, None).await
    }
    .await;
    match result {
        Ok(()) => crate::notify::install_finished(app, game_id, &title).await,
        Err(e) => {
            tracing::error!(game_id, "automatic install failed: {e}");
            crate::notify::failed(app, game_id, "Install", &title, &e.to_string()).await;
        }
    }
}

/// Adopts a folder the user installed into themselves.
#[tauri::command]
pub async fn locate_install(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    path: String,
) -> CommandResult<()> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(CommandError::msg(format!("{path} is not a folder.")));
    }
    super::safe_game_folder(&state, &dir)?;
    let _claim = claim(&state, game_id, Activity::installing())?;
    tracing::info!(game_id, %path, "adopting a user-chosen install folder");
    finish_install(&state, game_id, &dir).await;
    notify(&app);
    Ok(())
}

/// The setup programs that came with a game, in the order to run them.
#[tauri::command]
pub async fn list_setups(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<Vec<SetupProgram>> {
    Ok(setup_programs(&state, game_id)
        .await
        .into_iter()
        .map(|(setup, _)| setup)
        .collect())
}

/// Runs the chosen setups in the order given, which the user may have rearranged.
#[tauri::command]
pub async fn install_setups(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    paths: Vec<String>,
    silent: Option<bool>,
    delete_download: Option<bool>,
) -> CommandResult<()> {
    let mut known = setup_programs(&state, game_id).await;
    // Taken out as found, so a path sent twice does not run twice.
    let steps = paths
        .iter()
        .filter_map(|path| {
            let at = known.iter().position(|(setup, _)| setup.path == *path)?;
            let (setup, program) = known.swap_remove(at);
            Some(SetupStep {
                program,
                role: setup.role,
            })
        })
        .collect::<Vec<_>>();
    if steps.len() != paths.len() {
        return Err(CommandError::msg(
            "A chosen setup program is no longer there. Rescan and try again.",
        ));
    }
    tracing::info!(
        game_id,
        count = steps.len(),
        "running chosen setup programs"
    );
    let run = SetupRun {
        steps,
        elevated: false,
        silent: silent.unwrap_or(false),
        delete_download,
        left_for_user: 0,
    };
    run_setups(&app, game_id, run).await
}

/// A setup program the user picked, for an installer detection missed.
#[tauri::command]
pub async fn run_setup_path(
    app: AppHandle,
    game_id: i64,
    path: String,
    delete_download: Option<bool>,
) -> CommandResult<()> {
    tracing::info!(game_id, %path, "running a user-chosen setup program");
    run_setups(
        &app,
        game_id,
        SetupRun::single(path.into(), delete_download),
    )
    .await
}

/// Only reachable after Windows refused the program, so elevation is never asked for speculatively.
#[tauri::command]
pub async fn run_setup_elevated(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    if !cfg!(windows) {
        return Err(CommandError::msg(
            "Running as administrator only applies on Windows.",
        ));
    }
    let program = state
        .library()
        .record(game_id)
        .elevation_program
        .ok_or_else(|| CommandError::msg("Nothing is waiting to be run as administrator."))?;
    let run = SetupRun {
        elevated: true,
        ..SetupRun::single(program, None)
    };
    run_setups(&app, game_id, run).await
}

/// Found by naming convention, reported before anything is removed.
#[tauri::command]
pub async fn find_game_uninstaller(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<Option<String>> {
    let Some(dir) = state
        .library()
        .record(game_id)
        .install_dir
        .filter(|d| d.is_dir())
    else {
        return Ok(None);
    };
    let found = tokio::task::spawn_blocking(move || gameyfin_core::find_uninstaller(&dir))
        .await
        .unwrap_or(None);
    Ok(found.map(|p| p.to_string_lossy().into_owned()))
}

/// Removes an installed game's files, keeping the download. A picked `uninstaller` must be
/// inside the game's folder, since it is run.
#[tauri::command]
pub async fn uninstall_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    run_uninstaller: Option<bool>,
    uninstaller: Option<String>,
) -> CommandResult<()> {
    let Some(dir) = state.library().record(game_id).install_dir else {
        return Ok(());
    };
    let _claim = claim(&state, game_id, Activity::preparing("Uninstalling"))?;
    notify_state(&app, game_id);

    if dir.exists() {
        super::safe_game_folder(&state, &dir)?;
        let chosen = match uninstaller.filter(|p| !p.is_empty()) {
            Some(path) => {
                let relative = Path::new(&path)
                    .strip_prefix(&dir)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or(path);
                Some(contained(&dir, &relative)?)
            }
            None => None,
        };
        // A setup-installed game's own uninstaller also clears its registry entries and shortcuts.
        if run_uninstaller.unwrap_or(true) {
            if let Some(program) = chosen.or_else(|| gameyfin_core::find_uninstaller(&dir)) {
                tracing::info!(game_id, ?program, "running the game's uninstaller");
                if let Err(e) = run_uninstaller_program(&state, game_id, &program).await {
                    tracing::warn!(game_id, "{e}; removing the files directly instead");
                }
            }
        }
        tokio::fs::remove_dir_all(&dir)
            .await
            .context(format!("could not remove {}", dir.display()))?;
    }

    state
        .library()
        .update_record(game_id, |r| {
            r.install_dir = None;
            r.executable = None;
            r.installed_at = None;
            r.setup_candidates.clear();
            r.installed_setups.clear();
            r.elevation_program = None;
        })
        .await;
    state.library().clear_activity(game_id);
    notify(&app);
    Ok(())
}

async fn run_uninstaller_program(
    state: &AppState,
    game_id: i64,
    uninstaller: &Path,
) -> CommandResult<()> {
    let config = if gameyfin_core::needs_proton(uninstaller) {
        let runtime = crate::proton::runtime_for_game(state, game_id, Some(uninstaller)).await?;
        let prefix = super::layout_for(state, game_id)?.prefix_dir(game_id);
        let mut config = gameyfin_core::LaunchConfig::for_windows_program_unattended(
            uninstaller,
            prefix,
            &runtime,
        );
        super::launch::apply_game_runtime(state, game_id, &runtime, &mut config).await;
        config
    } else {
        gameyfin_core::LaunchConfig::native(uninstaller)
    };
    let command = gameyfin_core::resolve_command(&config)
        .context("could not build the uninstaller command")?;
    let run = match gameyfin_core::run_capturing(&command).await {
        // The user already asked to uninstall, and Windows still shows its own consent dialog.
        Err(gameyfin_core::CoreError::ElevationRequired { .. }) => {
            gameyfin_core::run_elevated(&command).await
        }
        other => other,
    }
    .context("could not run the uninstaller")?;
    if run.success() {
        Ok(())
    } else {
        Err(CommandError::msg(format!(
            "the uninstaller exited with code {}",
            run.status.unwrap_or(-1)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use InstallerMemoryLimit::{Auto, Mib, Off};

    fn failed(limit: InstallerMemoryLimit, cap_mib: Option<u64>, output: &str) -> Option<String> {
        memory_advice(limit, cap_mib, output, Duration::from_secs(600), false)
    }

    #[test]
    fn the_folder_state_notices_a_file_that_changed() {
        let dir = std::env::temp_dir().join(format!("gameyfin-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/level.pak"), b"old").unwrap();

        let before = scan_folder_state(&dir);
        assert!(before.is_some());
        assert_eq!(scan_folder_state(&dir), before);
        std::fs::write(dir.join("data/level.pak"), b"patched").unwrap();
        assert_ne!(scan_folder_state(&dir), before);
        assert_eq!(scan_folder_state(&dir.join("missing")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_installers_own_folder_merges_into_the_game() {
        let root = std::env::temp_dir().join(format!("gameyfin-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (game, dlc) = (root.join("(78) Wall World"), root.join("78 Wall World"));
        std::fs::create_dir_all(game.join("data")).unwrap();
        std::fs::write(game.join("WallWorld.exe"), b"game").unwrap();
        std::fs::write(game.join("data/base.pak"), b"base").unwrap();
        std::fs::create_dir_all(dlc.join("data")).unwrap();
        std::fs::write(dlc.join("data/dlc.pak"), b"dlc").unwrap();
        std::fs::write(dlc.join("WallWorld.exe"), b"patched").unwrap();

        merge_into(&dlc, &game).unwrap();
        assert!(!dlc.exists());
        assert_eq!(
            std::fs::read(game.join("WallWorld.exe")).unwrap(),
            b"patched"
        );
        assert!(game.join("data/base.pak").is_file() && game.join("data/dlc.pak").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn memory_trouble_is_recognised_in_the_wording_installers_print() {
        for output in [
            "ISDone.dll: An error occurred while unpacking: Not enough memory!",
            "unarc.dll returned an error code: -5",
            "err:virtual:allocate_virtual_memory cannot allocate",
            "OUT OF MEMORY",
        ] {
            assert!(looks_like_memory_trouble(output), "{output}");
        }
        for output in [
            "The setup files are corrupted.",
            "Library MSVCR120.dll not found",
            "",
        ] {
            assert!(!looks_like_memory_trouble(output), "{output}");
        }
    }

    #[test]
    fn the_advice_names_the_limit_that_was_in_force() {
        if cfg!(windows) {
            assert_eq!(failed(Auto, Some(8192), "Not enough memory!"), None);
            return;
        }
        let fixed = failed(Mib(2048), Some(2048), "Not enough memory!").unwrap();
        assert!(fixed.contains("under the 2048 MB limit") && fixed.contains("raise or remove"));
        let automatic = failed(Auto, Some(16384), "Not enough memory!").unwrap();
        assert!(automatic.contains("automatic 16384 MB") && automatic.contains("higher fixed"));
        let uncapped = failed(Off, None, "Not enough memory!").unwrap();
        assert!(!uncapped.contains("Settings"));
        for (limit, cap) in [(Off, None), (Auto, Some(8192)), (Mib(3072), Some(3072))] {
            assert!(failed(limit, cap, "Library MSVCR120.dll not found")
                .unwrap()
                .starts_with("If the installer"));
        }
    }

    #[test]
    fn a_quick_cancel_is_left_alone_but_a_wedged_installer_is_not() {
        if cfg!(windows) {
            return;
        }
        let quick = Duration::from_secs(20);
        assert_eq!(memory_advice(Auto, Some(8192), "", quick, true), None);
        assert!(memory_advice(Auto, Some(8192), "Not enough memory!", quick, true).is_some());

        let stuck =
            |limit, cap| memory_advice(limit, cap, "", Duration::from_secs(3600), true).unwrap();
        assert!(
            stuck(Off, None).starts_with("An installer that gets stuck")
                && stuck(Off, None).contains("3 GB")
        );
        assert!(stuck(Mib(3072), Some(3072)).contains("other than 3072 MB"));
        assert!(stuck(Auto, Some(8192)).contains("fixed memory limit, such as 3 GB"));
    }
}
