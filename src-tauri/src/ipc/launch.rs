//! Launching games and preparing the compatibility prefixes Windows programs run in.

use std::path::{Path, PathBuf};

use gameyfin_core::{PrefixState, WindowsRuntime};
use tauri::{AppHandle, Manager, State};

use super::notify;
use crate::error::{CommandError, CommandResult, Context};
use crate::library_state::{Activity, Stage};
use crate::saves::LaunchGate;
use crate::state::AppState;

/// Every failure is logged: a launch that fails silently leaves nothing to diagnose.
#[tauri::command]
pub async fn launch_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    tracing::info!(game_id, "launch requested");
    let Some(claim) = state
        .library()
        .claim(game_id, Activity::preparing("Starting"))
    else {
        tracing::info!(game_id, "already busy; ignoring the launch request");
        return Ok(());
    };
    notify(&app);
    let result = launch(&app, &state, game_id, claim).await;
    if let Err(e) = &result {
        tracing::error!(game_id, "could not launch: {e}");
    }
    notify(&app);
    result
}

async fn launch(
    app: &AppHandle,
    state: &AppState,
    game_id: i64,
    claim: crate::library_state::Claim,
) -> CommandResult<()> {
    let record = state.library().record(game_id);
    let install_dir = record
        .install_dir
        .clone()
        .filter(|d| d.exists())
        .ok_or_else(|| CommandError::msg("This game is not installed."))?;
    let executable = record
        .executable
        .as_deref()
        .and_then(|exe| super::contained(&install_dir, exe).ok())
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            CommandError::msg(
                "No launch executable has been chosen yet. Pick one in the game's options.",
            )
        })?;
    super::ensure_windows_program(&executable, "install")?;

    let mut launched_with = None;
    let mut config = if gameyfin_core::needs_proton(&executable) {
        let (runtime, prefix, prepared) =
            ready_windows_prefix(app, game_id, Some(&executable)).await?;
        tracing::info!(
            game_id,
            runtime = runtime.kind(),
            dxvk = prepared.dxvk.as_deref().unwrap_or("none"),
            vkd3d = prepared.vkd3d.as_deref().unwrap_or("none"),
            ?executable,
            "launching a Windows game"
        );
        launched_with = Some(runtime.description());
        // The prefix's own record, since it knows which components actually went in.
        let overrides = gameyfin_core::graphics::overrides_for_prefix(&prepared);
        let mut config = gameyfin_core::LaunchConfig::for_windows_program_with(
            &executable,
            prefix,
            &runtime,
            &overrides,
        );
        apply_game_runtime(state, game_id, &runtime, &mut config).await;
        config
    } else {
        gameyfin_core::LaunchConfig::native(&executable)
    };

    // After the prefix exists (Proton saves live inside it) and before the game reads them.
    if crate::saves::before_launch(app, state, game_id).await == LaunchGate::AwaitingSaveDecision {
        tracing::info!(
            game_id,
            "holding the launch until the save question is answered"
        );
        return Ok(());
    }

    config
        .arguments
        .extend(gameyfin_core::arguments::split(&record.launch_arguments));
    // DLL overrides merge, so `dxgi=builtin` does not also bring back the Mono and Gecko prompts.
    for (key, value) in gameyfin_core::environment::parse(&record.launch_environment) {
        let value = if key == "WINEDLLOVERRIDES" {
            let current = config.environment.get(&key).cloned().unwrap_or_default();
            gameyfin_core::DllOverrides::parse(&current)
                .merged_with(gameyfin_core::DllOverrides::parse(&value))
                .to_env()
        } else {
            value
        };
        config.environment.insert(key, value);
    }

    let command =
        gameyfin_core::resolve_command(&config).context("could not build the launch command")?;
    tracing::debug!(game_id, program = ?command.program, args = ?command.args, "resolved command");

    let stopper = state.processes.register(game_id);
    let teardown = command.wine_teardown();
    let running = Activity::Running {
        since: super::now_iso8601(),
        executable: Some(super::file_label(&executable)),
        runtime: launched_with,
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _claim = claim;
        let state = app.state::<AppState>();
        let library = state.library().clone();
        let supervisor = match gameyfin_core::Supervisor::spawn(&command) {
            Ok(supervisor) => supervisor,
            Err(e) => {
                state.processes.finish(game_id);
                library.fail(game_id, Stage::Launch, e.to_string());
                return notify(&app);
            }
        };
        library.set_activity(game_id, running);
        notify(&app);

        let ended = supervisor.wait_or_stop(&stopper, teardown).await;
        state.processes.finish(game_id);
        match ended {
            Ok(session) if session.is_meaningful() => {
                tracing::info!(game_id, duration = ?session.duration, end = ?session.end, "game ended");
                let minutes = session.minutes_played();
                library
                    .update_record(game_id, |r| {
                        r.minutes_played += minutes;
                        r.last_played_at = Some(super::now_iso8601());
                    })
                    .await;
                library.clear_activity(game_id);
                notify(&app);
                // A launch that died in seconds wrote no saves worth uploading.
                crate::saves::after_exit(&app, &state, game_id).await;
            }
            Ok(session) => match failed_to_start(&session) {
                Some(message) => {
                    tracing::error!(game_id, output = %session.error_output, "the game exited immediately");
                    library.fail(game_id, Stage::Launch, message);
                }
                None => library.clear_activity(game_id),
            },
            Err(e) => library.fail(game_id, Stage::Launch, e.to_string()),
        }
        notify(&app);
    });
    Ok(())
}

/// Stops an installer or a game, including a repack installer frozen out of memory.
#[tauri::command]
pub async fn stop_game(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<()> {
    if let Some(stopper) = state.processes.get(game_id) {
        stopper.stop();
    }
    notify(&app);
    Ok(())
}

/// Explains a non-zero exit from a short run. A clean quick exit is a launcher handing off.
fn failed_to_start(session: &gameyfin_core::Session) -> Option<String> {
    let code = match session.end {
        gameyfin_core::SessionEnd::Exited { code: Some(code) } if code != 0 => code,
        _ => return None,
    };
    // Wine prints pages of `fixme:` on a normal run; what went wrong is at the end.
    let lines: Vec<&str> = session
        .error_output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(12)..].join("\n");
    let advice = gameyfin_core::diagnose::explain(&session.error_output);
    Some(match (advice, tail.is_empty()) {
        (Some(advice), true) => advice.to_string(),
        (Some(advice), false) => format!("{advice}\n\nThe game exited with code {code}:\n{tail}"),
        (None, true) => {
            format!(
                "The game exited immediately with code {code} and wrote nothing that explains why."
            )
        }
        (None, false) => format!("The game exited immediately with code {code}:\n{tail}"),
    })
}

/// Settings that follow the game into its Windows runtime: library mounts and the umu id.
pub async fn apply_game_runtime(
    state: &AppState,
    game_id: i64,
    runtime: &WindowsRuntime,
    config: &mut gameyfin_core::LaunchConfig,
) {
    config
        .environment
        .extend(crate::proton::container_mounts(state, runtime));
    if let gameyfin_core::Runtime::Proton { umu_id, .. } = &mut config.runtime {
        *umu_id = state.umu_id_for_game(game_id).await;
    }
}

/// Finds a Windows runtime and readies the game's prefix, reporting it as `Preparing`
/// because a first run downloads hundreds of megabytes.
pub async fn ready_windows_prefix(
    app: &AppHandle,
    game_id: i64,
    program: Option<&Path>,
) -> CommandResult<(WindowsRuntime, PathBuf, PrefixState)> {
    let state = app.state::<AppState>();
    let config_dir = state.config_dir();
    let preparing = |message: String| {
        state
            .library()
            .set_activity(game_id, Activity::Preparing { message });
        super::notify_state(app, game_id);
    };

    // Wine when umu cannot run, or for a 32-bit program without 32-bit host libraries.
    let wine_only = crate::proton::umu_launcher().await.is_err()
        || (!gameyfin_core::runtime::has_32bit_support()
            && program.is_some_and(gameyfin_core::executable::is_32bit_windows_program));
    {
        let _runtime_lock = state.runtime_lock().await;
        let missing = if wine_only {
            gameyfin_core::wine::installed(&config_dir)
                .is_none()
                .then_some("Wine")
        } else {
            gameyfin_core::proton::installed(&config_dir)
                .is_empty()
                .then_some("Proton")
        };
        if let Some(name) = missing {
            preparing(format!(
                "Downloading {name} for Windows games. This happens once and can take several minutes."
            ));
            if wine_only {
                if let Err(e) = crate::wine::ensure(app, &state).await {
                    tracing::warn!(game_id, error = %e, "could not download Wine");
                }
            } else {
                crate::proton::ensure_default(app, &state).await;
            }
        }
    }

    let runtime = crate::proton::runtime_for_game(&state, game_id, program).await?;
    // Wine fails within milliseconds when the prefix's parent is missing.
    let prefix = super::layout_for(&state, game_id)?.prefix_dir(game_id);
    tokio::fs::create_dir_all(&prefix)
        .await
        .context("could not create the compatibility prefix")?;

    // A different runtime version must update the prefix, or reinstalls Wine's Direct3D DLLs.
    let runtime_identity = match &runtime {
        WindowsRuntime::Umu { build, .. } => format!("umu {build}"),
        _ => gameyfin_core::wine::installed(&config_dir).map_or_else(
            || runtime.kind().to_string(),
            |w| format!("{} {}", w.variant.as_str(), w.version),
        ),
    };
    let mut wanted = PrefixState {
        dpi: screen_dpi(app),
        runtime: runtime_identity,
        ..PrefixState::default()
    };
    let needs_setup = !gameyfin_core::prefix::is_prepared(&prefix, &wanted);
    if needs_setup {
        let container = if runtime.is_wine_family() {
            ""
        } else {
            " It may also download the Steam Runtime."
        };
        preparing(format!(
            "Setting up {} for this game. The first run can take several minutes.{container}",
            runtime.description()
        ));
    }

    // Proton manages its own DXVK; only resolved when needed, so a plain launch stays offline.
    let components = if !runtime.is_wine_family() {
        gameyfin_core::InstalledGraphics::default()
    } else if needs_setup {
        let _runtime_lock = state.runtime_lock().await;
        crate::graphics::ensure_installed(app, &state).await?
    } else {
        gameyfin_core::graphics::installed(&config_dir)
    };
    (wanted.dxvk, wanted.vkd3d) = components.versions();

    if !gameyfin_core::prefix::is_prepared(&prefix, &wanted) {
        let mounts = crate::proton::container_mounts(&state, &runtime);
        prepare_prefix(&runtime, &prefix, &wanted, &components, mounts)
            .await
            .map_err(CommandError::Message)?;
    }
    Ok((runtime, prefix, wanted))
}

async fn run_logged(command: &gameyfin_core::ResolvedCommand, what: &str) -> bool {
    match gameyfin_core::run_capturing(command).await {
        Ok(run) if run.success() => true,
        Ok(run) => {
            tracing::warn!(output = %run.tail(4), "could not {what}");
            false
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not {what}");
            false
        }
    }
}

/// Initialises a prefix once. The overrides suppress Mono and Gecko dialogs that hang boot.
async fn prepare_prefix(
    runtime: &WindowsRuntime,
    prefix: &Path,
    wanted: &PrefixState,
    components: &gameyfin_core::InstalledGraphics,
    mounts: Option<(String, String)>,
) -> Result<(), String> {
    use gameyfin_core::prefix;

    if !runtime.is_wine_family() {
        return prepare_umu_prefix(runtime, prefix, wanted, mounts).await;
    }
    tracing::info!(
        ?prefix,
        dpi = wanted.dpi,
        runtime = runtime.kind(),
        "preparing a Wine prefix"
    );

    match gameyfin_core::run_capturing(&prefix::boot_command(runtime, prefix)).await {
        Ok(run) if !run.success() => {
            tracing::warn!(?prefix, output = %run.tail(8), "prefix initialisation reported an error; continuing")
        }
        Ok(_) => {}
        Err(e) => {
            return Err(format!(
                "could not initialise the compatibility prefix: {e}"
            ))
        }
    }
    run_logged(
        &prefix::dpi_command(runtime, prefix, wanted.dpi),
        "set the prefix DPI",
    )
    .await;
    // Without an active theme Wine draws Windows 2000 controls.
    if prefix::has_bundled_theme(prefix) {
        for command in prefix::theme_commands(runtime, prefix) {
            run_logged(&command, "set the prefix theme").await;
        }
    }
    // wineboot can report success and still leave no drive_c.
    if !prefix::wine_root(prefix).join("drive_c").is_dir() {
        return Err(format!(
            "The compatibility prefix at {} was not set up correctly: drive_c is missing. Deleting that folder and trying again usually fixes it.",
            prefix.display()
        ));
    }

    // Last: wineboot reinstalls Wine's own Direct3D DLLs over anything copied earlier.
    let mut installed = PrefixState {
        dxvk: None,
        vkd3d: None,
        ..wanted.clone()
    };
    match gameyfin_core::graphics::install_into_prefix(prefix, components) {
        Ok(dlls) if !dlls.is_empty() => {
            let overrides = gameyfin_core::graphics::overrides_for(&dlls);
            for command in prefix::override_commands(runtime, prefix, &overrides) {
                run_logged(&command, "set a DLL override").await;
            }
            (installed.dxvk, installed.vkd3d) = components.versions();
        }
        Ok(_) => {}
        // Not fatal: WineD3D still runs, and recording nothing makes the next launch retry.
        Err(e) => tracing::warn!(?prefix, error = %e, "could not install the Direct3D components"),
    }
    if let Err(e) = prefix::mark_prepared(prefix, &installed) {
        tracing::debug!(?prefix, error = %e, "could not record prefix preparation");
    }
    Ok(())
}

/// Proton's script builds or upgrades the prefix itself, so setup is one registry import.
async fn prepare_umu_prefix(
    runtime: &WindowsRuntime,
    prefix: &Path,
    wanted: &PrefixState,
    mounts: Option<(String, String)>,
) -> Result<(), String> {
    use gameyfin_core::prefix;

    tracing::info!(?prefix, dpi = wanted.dpi, runtime = %runtime.description(), "preparing a prefix through umu");
    let file = prefix.join(prefix::REGISTRY_FILE);
    tokio::fs::write(&file, prefix::registry_file(wanted.dpi))
        .await
        .map_err(|e| format!("could not write the prefix setup: {e}"))?;
    let mut import = prefix::import_command(runtime, prefix, &file);
    import.env.extend(mounts);
    let output = match gameyfin_core::run_capturing(&import).await {
        Ok(run) if run.success() => String::new(),
        Ok(run) => run.tail(12),
        Err(e) => return Err(format!("could not start umu: {e}")),
    };
    let _ = tokio::fs::remove_file(&file).await;

    if !prefix::wine_root(prefix).join("drive_c").is_dir() {
        return Err(match gameyfin_core::diagnose::explain(&output) {
            Some(advice) => advice.to_string(),
            None => format!(
                "umu could not set up the compatibility prefix at {}:\n{output}",
                prefix.display()
            ),
        });
    }
    if let Err(e) = prefix::mark_prepared(prefix, wanted) {
        tracing::debug!(?prefix, error = %e, "could not record prefix preparation");
    }
    Ok(())
}

/// From the monitor the window is on, where an installer will open. No monitor, no guess.
fn screen_dpi(app: &AppHandle) -> u32 {
    let window = app.get_webview_window("main");
    let monitor = window
        .as_ref()
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    match monitor {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let height = (f64::from(monitor.size().height) / scale).round() as u32;
            gameyfin_core::dpi_for_screen(scale, Some(height))
        }
        None => gameyfin_core::dpi_for_screen(
            window.and_then(|w| w.scale_factor().ok()).unwrap_or(1.0),
            None,
        ),
    }
}
