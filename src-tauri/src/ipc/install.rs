//! Installing and uninstalling: moving unpacked files into place, running setup programs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::library::{discard_download_if_asked, extract_download};
use super::{contained, file_label, notify, notify_state};
use crate::error::{blocking, CommandError, CommandResult, Context};
use crate::library_state::{relative_to, scan_setups, Activity, GameRecord, Stage};
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
    pub setup_candidates: Vec<String>,
    /// Where a file picker should start: the unpacked files, else the download folder.
    pub browse_dir: Option<String>,
}

#[tauri::command]
pub async fn install_options(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<InstallPlan> {
    use gameyfin_core::InstallMethod;

    let record = state.library().record(game_id);
    let install_dir = super::install_dir_for(&state, game_id).await?;
    let (runtime, can_run_windows) = if cfg!(windows) {
        (None, true)
    } else {
        let ctx = crate::proton::runtime_context(&state, Some(game_id), None).await;
        // umu downloads a Proton build on first run, so it counts before one exists.
        let umu_ready = ctx.umu_launcher.is_some();
        let found =
            tokio::task::spawn_blocking(move || gameyfin_core::detect_windows_runtime(&ctx))
                .await
                .ok()
                .flatten();
        let available = umu_ready || found.is_some();
        (found, available)
    };
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

    let (payload, options, setup_candidates) = if record.existing_staging().is_some() {
        let mut options: Vec<InstallOption> = record
            .setup_candidates
            .iter()
            .map(|setup| {
                option(
                    InstallMethod::RunWindowsInstaller,
                    format!("setup:{setup}"),
                    format!("Run {setup}"),
                    blocked(true),
                )
            })
            .collect();
        options.push(option(
            InstallMethod::CopyExecutable,
            "move".into(),
            "Move into your games folder".into(),
            None,
        ));
        (
            "unpacked files".to_string(),
            options,
            record.setup_candidates.clone(),
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

    let needs_install_path = options.iter().any(|o: &InstallOption| o.interactive);
    Ok(InstallPlan {
        payload,
        windows_install_path: needs_install_path
            .then(|| windows_install_path(&install_dir))
            .flatten(),
        needs_install_path,
        options,
        default_install_dir: install_dir.to_string_lossy().into_owned(),
        browse_dir: browse_dir_for(&record),
        setup_candidates,
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
        // No runtime yet reads as Proton: umu downloads one on first run.
        RunWindowsInstallerViaProton => match runtime {
            Some(r) if r.is_wine_family() => "Run the installer with Wine",
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

/// The Windows path of a game's folder on the mapped games drive; `None` on Windows.
fn windows_install_path(install_dir: &Path) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let folder = install_dir.file_name()?.to_string_lossy().into_owned();
    Some(gameyfin_core::games_drive_path(&folder))
}

/// `method` is `extract`, `move`, `setup:<relative path>`, or a payload method key.
#[tauri::command]
pub async fn install_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    method: Option<String>,
    delete_archive: Option<bool>,
) -> CommandResult<()> {
    let method = method.unwrap_or_else(|| "extract".to_string());
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
        return move_into_place(&app, game_id, &staging()?, &install_dir).await;
    }
    if let Some(relative) = method.strip_prefix("setup:") {
        let program = contained(&staging()?, relative)?;
        return run_installer(&app, game_id, &program, &install_dir, false).await;
    }

    let archive = record
        .existing_archive()
        .ok_or_else(|| CommandError::msg("This game has not been downloaded yet."))?;
    match gameyfin_core::InstallMethod::from_key(&method) {
        Some(gameyfin_core::InstallMethod::CopyExecutable) => {
            copy_executable(&app, game_id, &archive, &install_dir).await
        }
        Some(
            gameyfin_core::InstallMethod::RunWindowsInstaller
            | gameyfin_core::InstallMethod::RunWindowsInstallerViaProton,
        ) => run_installer(&app, game_id, &archive, &install_dir, false).await,
        _ => Err(CommandError::msg(format!(
            "{method} is not a way to install this."
        ))),
    }
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
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let _claim = claim(&state, game_id, Activity::Installing)?;
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
        let (from, to) = (source.to_path_buf(), incoming.clone());
        if let Err(e) = blocking("could not copy the files", move || copy_tree(&from, &to)).await {
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
    discard_download_if_asked(app, game_id, install_dir).await;
    notify_state(app, game_id);
    Ok(())
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
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    let _claim = claim(&state, game_id, Activity::Installing)?;
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
    discard_download_if_asked(app, game_id, install_dir).await;
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
    state.library().clear_activity(game_id);
}

/// Records a completed install. A chosen executable that still exists is kept.
pub async fn finish_install(state: &AppState, game_id: i64, install_dir: &Path) {
    let dir = install_dir.to_path_buf();
    let (detected, setups) = tokio::task::spawn_blocking(move || {
        let detected = match gameyfin_core::executable::detect(&dir, "") {
            Ok(gameyfin_core::Detection::Confident(path)) => relative_to(&path, &dir),
            _ => None,
        };
        (detected, scan_setups(&dir))
    })
    .await
    .unwrap_or_default();
    let staging = state.library().record(game_id).existing_staging();
    let staging_setups = match staging {
        Some(dir) => tokio::task::spawn_blocking(move || scan_setups(&dir))
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

/// Runs a setup program and adopts whatever it installs. `elevated` reruns as administrator.
async fn run_installer(
    app: &AppHandle,
    game_id: i64,
    program: &Path,
    install_dir: &Path,
    elevated: bool,
) -> CommandResult<()> {
    let state = app.state::<AppState>();
    if !program.is_file() {
        return Err(CommandError::msg(format!(
            "{} is no longer there.",
            program.display()
        )));
    }
    // Checked before a prefix is built: Wine would only say `Bad format` minutes later.
    super::ensure_windows_program(program, "download")?;
    let claim = claim(
        &state,
        game_id,
        Activity::preparing("Preparing the installer"),
    )?;

    let mut unarc_prefix = None;
    let mut config = if gameyfin_core::needs_proton(program) {
        // The prefix must exist before any drive is mapped, or wineboot skips building it.
        let (runtime, prefix, _) =
            super::launch::ready_windows_prefix(app, game_id, Some(program)).await?;
        unarc_prefix = Some(prefix.clone());

        // The installations root, so the wizard offers a subfolder rather than a bare `G:\`.
        let installations_root = install_dir.parent().unwrap_or(install_dir);
        if let Err(e) = gameyfin_core::map_drive(&prefix, installations_root) {
            tracing::warn!(game_id, error = %e, "could not map the games drive");
        }
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
        config
    } else {
        gameyfin_core::LaunchConfig::native(program)
    };

    // Pass the destination where the toolkit accepts one, to a folder name `cmd` can handle.
    let kind = gameyfin_core::identify(program).unwrap_or(gameyfin_core::InstallerKind::Unknown);
    let safe_folder = gameyfin_core::windows_safe_name(&file_label(install_dir));
    let destination = if cfg!(windows) {
        install_dir.to_string_lossy().into_owned()
    } else {
        gameyfin_core::games_drive_path(&safe_folder)
    };
    config.arguments.extend(kind.destination_args(&destination));
    // The user's own options last, so their flags win over the toolkit defaults.
    config.arguments.extend(gameyfin_core::arguments::split(
        &state.library().record(game_id).installer_arguments,
    ));

    let mut command =
        gameyfin_core::resolve_command(&config).context("could not build the installer command")?;
    tracing::debug!(game_id, program = ?command.program, args = ?command.args, kind = kind.label(), "installer command");
    tokio::fs::create_dir_all(install_dir)
        .await
        .context(format!("could not create {}", install_dir.display()))?;

    state
        .library()
        .update_record(game_id, |r| r.elevation_program = None)
        .await;
    state.library().set_activity(game_id, Activity::Installing);
    notify_state(app, game_id);

    // Caps the whole process tree, which the installer's own RAM option does not reach.
    let limit = state.settings().installer_memory_limit;
    let cap_mib = if cfg!(windows) {
        None
    } else {
        limit.resolve(gameyfin_core::process::total_memory_bytes())
    };
    let address_space = cap_mib.and_then(|mib| {
        let cap = command.cap_address_space(mib * 1024 * 1024);
        tracing::info!(
            game_id,
            mib,
            via = cap.label(),
            "capping the installer's address space"
        );
        cap.before_exec()
    });

    let stopper = state.processes.register(game_id);
    let (app, install_dir, program) = (
        app.clone(),
        install_dir.to_path_buf(),
        program.to_path_buf(),
    );
    tauri::async_runtime::spawn(async move {
        let _claim = claim;
        let state = app.state::<AppState>();
        let unarc_guard = unarc_prefix
            .map(|prefix| tauri::async_runtime::spawn(gameyfin_core::unarc::guard(prefix)));
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
        let elapsed = started.elapsed();
        state.processes.finish(game_id);

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
                let advice = gameyfin_core::diagnose::explain(&tail)
                    .map(str::to_string)
                    .or_else(|| memory_advice(limit, cap_mib, &tail, elapsed, stopped));
                if let Some(advice) = advice {
                    message.push_str("\n\n");
                    message.push_str(&advice);
                }
                Some(message)
            }
            // Not a broken download: remembering the program lets the UI offer the elevated retry.
            Err(gameyfin_core::CoreError::ElevationRequired { .. }) => {
                state
                    .library()
                    .update_record(game_id, |r| r.elevation_program = Some(program.clone()))
                    .await;
                Some(format!(
                    "{} needs to run as administrator.",
                    file_label(&program)
                ))
            }
            Err(e) => Some(e.to_string()),
        };
        if let Some(message) = failure {
            tracing::error!(game_id, ?elapsed, "installer failed: {message}");
            state.library().fail(game_id, Stage::Install, message);
            return notify_state(&app, game_id);
        }

        // The wizard was pointed at a shell-safe name; move the result to the expected one.
        if let Some(root) = install_dir.parent() {
            let written_to = root.join(&safe_folder);
            if written_to != install_dir
                && written_to.is_dir()
                && !super::dir_is_empty(&written_to).await
            {
                let _ = tokio::fs::remove_dir(&install_dir).await;
                if let Err(e) = tokio::fs::rename(&written_to, &install_dir).await {
                    tracing::warn!(game_id, error = %e, "could not rename the installed folder");
                }
            }
        }

        let populated = crate::library_state::has_content(&install_dir);
        if populated {
            finish_install(&state, game_id, &install_dir).await;
            discard_download_if_asked(&app, game_id, &install_dir).await;
        } else if kind == gameyfin_core::InstallerKind::Unknown && elapsed >= Duration::from_secs(3)
        {
            // No known toolkit, a clean exit, and a real run: this was the game itself.
            tracing::info!(
                game_id,
                ?program,
                "treating the download as the game itself"
            );
            match keep_program(&program, &install_dir).await {
                Ok(name) => {
                    record_install(&state, game_id, &install_dir, Some(name)).await;
                    discard_download_if_asked(&app, game_id, &install_dir).await;
                }
                Err(e) => state.library().fail(game_id, Stage::Install, e.to_string()),
            }
        } else {
            state.library().fail(
                game_id,
                Stage::Install,
                "The installer finished without putting anything in the suggested folder. \
                 If you chose a different location, use \"I installed it myself\".",
            );
        }
        notify_state(&app, game_id);
    });
    Ok(())
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
        move_into_place(app, game_id, &source, &install_dir).await
    }
    .await;
    match result {
        Ok(()) => crate::notify::install_finished(app, &title).await,
        Err(e) => {
            tracing::error!(game_id, "automatic install failed: {e}");
            crate::notify::failed(app, "Install", &title, &e.to_string()).await;
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
    let _claim = claim(&state, game_id, Activity::Installing)?;
    tracing::info!(game_id, %path, "adopting a user-chosen install folder");
    finish_install(&state, game_id, &dir).await;
    notify(&app);
    Ok(())
}

/// Runs a setup program from the installed files or the unpacked download, such as a DLC.
#[tauri::command]
pub async fn run_setup(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    relative: String,
) -> CommandResult<()> {
    let record = state.library().record(game_id);
    let program = [record.install_dir, record.extracted_dir]
        .into_iter()
        .flatten()
        .find_map(|base| contained(&base, &relative).ok().filter(|p| p.is_file()))
        .ok_or_else(|| {
            CommandError::msg(format!("{relative} could not be found for this game."))
        })?;
    let install_dir = super::install_dir_for(&state, game_id).await?;
    run_installer(&app, game_id, &program, &install_dir, false).await
}

/// A setup program the user picked, for an installer detection missed.
#[tauri::command]
pub async fn run_setup_path(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    path: String,
) -> CommandResult<()> {
    let install_dir = super::install_dir_for(&state, game_id).await?;
    tracing::info!(game_id, %path, "running a user-chosen setup program");
    run_installer(&app, game_id, Path::new(&path), &install_dir, false).await
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
    let install_dir = super::install_dir_for(&state, game_id).await?;
    run_installer(&app, game_id, &program, &install_dir, true).await
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
