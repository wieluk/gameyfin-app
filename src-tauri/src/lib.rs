//! Gameyfin desktop application shell.
//!
//! This crate is deliberately thin. It owns the window, the IPC surface and process-wide
//! setup; everything testable lives in the `gameyfin-*` crates, which build without any
//! GUI toolchain.

mod auth_flow;
mod cli;
mod downloads;
mod error;
mod gamepad;
mod image_cache;
mod images;
mod integrations;
mod ipc;
mod library_state;
mod notify;
mod saves;
mod settings;
mod state;
mod taskbar;
mod tray;
mod updater;

use tauri::{Emitter, Manager};

use state::AppState;

/// Lets the log level change at runtime, once the user's setting is known.
static LOG_RELOAD: std::sync::OnceLock<
    tracing_subscriber::reload::Handle<tracing_subscriber::EnvFilter, tracing_subscriber::Registry>,
> = std::sync::OnceLock::new();

/// Apply a log level chosen in Settings.
pub fn set_log_level(level: settings::LogLevel) -> Result<(), String> {
    let Some(handle) = LOG_RELOAD.get() else {
        return Err("logging is not initialised".into());
    };
    handle
        .reload(tracing_subscriber::EnvFilter::new(level.filter()))
        .map_err(|e| format!("could not change the log level: {e}"))?;
    tracing::info!(level = level.key(), "log level changed");
    Ok(())
}

/// Where log files are written, so the UI can point the user at them.
pub fn log_directory() -> std::path::PathBuf {
    dirs_log_dir().unwrap_or_else(std::env::temp_dir)
}

fn dirs_log_dir() -> Option<std::path::PathBuf> {
    // Mirrors Tauri's own resolution without needing an AppHandle, so logging can start
    // before the app is built.
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
            })
            .map(|base| base.join("org.gameyfin.desktop/logs"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(|base| std::path::PathBuf::from(base).join("org.gameyfin.desktop/logs"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        std::env::var_os("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Library/Logs/org.gameyfin.desktop"))
    }
}

/// Set up console and rolling file logging.
///
/// The returned guard must be held for the process lifetime; dropping it stops the
/// background writer and loses buffered lines.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // RUST_LOG still wins when set, for the case where the app will not start far enough
    // to change the setting.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(settings::LogLevel::default().filter())
    });
    let (filter, reload_handle) = tracing_subscriber::reload::Layer::new(filter);
    let _ = LOG_RELOAD.set(reload_handle);

    let dir = log_directory();
    let file_layer = match std::fs::create_dir_all(&dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&dir, "gameyfin.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            Some((
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(writer),
                guard,
            ))
        }
        Err(e) => {
            eprintln!("could not create log directory {dir:?}: {e}");
            None
        }
    };

    match file_layer {
        Some((layer, guard)) => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .with(layer)
                .init();
            tracing::info!("logging to {dir:?}");
            Some(guard)
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .init();
            None
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // A packaged app has nowhere useful to write stdout, launched from a desktop entry
    // it is simply discarded, so logs go to a file as well. Without this a failure in
    // the field leaves nothing to look at.
    let _log_guard = init_logging();

    tauri::Builder::default()
        // A second copy would fight the first over the library file and the download
        // checkpoints. Launching from a desktop shortcut while the app is already open is
        // the common case, so the second process hands its arguments over and exits.
        // Registered first: plugins set up in the order they are added, and this one only
        // stops the duplicate during its own setup, so anything before it runs twice.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match cli::launch_target(&argv) {
                    Some(game_id) => cli::handle_launch(&app, game_id).await,
                    // Starting it again with no argument is how people ask for the window
                    // back when it is hidden in the tray.
                    None => tray::reveal(&app, None),
                }
            });
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // `--hidden` matches what the autostart entry passes, so a login launch goes
        // straight to the tray rather than opening a window on top of the desktop.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .register_asynchronous_uri_scheme_protocol(images::SCHEME, images::handle)
        .manage(AppState::default())
        .setup(|app| {
            tray::install(app.handle())?;
            tray::guard_window(app.handle());

            // Reconnect with a stored session before the window asks, so a returning
            // user lands on their library rather than flashing the wizard first.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let config_dir = handle
                    .path()
                    .app_config_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."));
                let restored = state.restore(config_dir.clone()).await;

                // The umu database decides which per-title Proton fixes a launch gets.
                // Loaded from cache first so an early launch is not delayed by a fetch.
                if state.load_umu_database(&config_dir).await {
                    let refresh = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = refresh.state::<AppState>();
                        if let Err(e) = state.refresh_umu_database().await {
                            // Advisory data: a failure costs per-title fixes, not the
                            // ability to play anything.
                            tracing::info!("could not refresh the umu database: {e}");
                        }
                    });
                }
                // Keep the artwork cache within bounds on startup, when it costs nothing.
                let cache = state.image_cache().await;
                tokio::task::spawn_blocking(move || cache.prune());
                // Apply the stored verbosity now that settings have been read.
                let level = state.settings().await.log_level;
                if let Err(e) = set_log_level(level) {
                    tracing::warn!("{e}");
                }
                tracing::info!("stored session restored: {restored}");

                let settings = state.settings().await;

                // Controllers are polled on their own thread; the handle lets a settings
                // change reach it without restarting anything.
                let pad = gamepad::Handle::new(settings.gamepad_enabled, settings.gamepad_deadzone);
                state.set_gamepad(pad.clone()).await;
                gamepad::spawn(handle.clone(), pad);

                // Asked for once at startup rather than polled: releases are not frequent
                // enough for anything else to be worth the requests.
                if settings.check_for_updates {
                    let updates = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let status = updater::check(&updates.state::<AppState>()).await;
                        if status.available {
                            if let Some(version) = status.latest_version.clone() {
                                notify::update_available(&updates, &version).await;
                            }
                            let _ = updates.emit("update-available", status);
                        }
                    });
                }

                // Starting hidden only makes sense with somewhere to be hidden *to*,
                // which the tray provides. `--hidden` is what the autostart entry passes,
                // so a login launch is quiet even when the setting is off.
                let launched_hidden = std::env::args().any(|arg| arg == "--hidden");
                if (settings.start_minimized || launched_hidden) && tray::has_tray() {
                    if let Some(window) = handle.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }

                // The UI waits for this before deciding what to show.
                let _ = handle.emit("connection-restored", restored);

                // Acted on last, once the session is back and the library is readable.
                // A shortcut that started the app has to wait for that; one that reached
                // an already-running copy goes through the single-instance hook instead.
                if let Some(game_id) = cli::launch_target(&std::env::args().collect::<Vec<_>>()) {
                    cli::handle_launch(&handle, game_id).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::connection_status,
            ipc::probe_server,
            ipc::set_server_url,
            ipc::begin_login,
            ipc::poll_login,
            ipc::cancel_login,
            ipc::reset_login,
            ipc::sign_out,
            ipc::get_settings,
            ipc::suggest_library_root,
            ipc::set_log_level,
            ipc::set_installer_memory_limit,
            ipc::cancel_download,
            ipc::wine_status,
            ipc::install_wine,
            ipc::remove_wine,
            ipc::set_wine_variant,
            ipc::set_wine_prompt_dismissed,
            ipc::download_providers,
            ipc::set_download_provider,
            ipc::image_cache_size,
            ipc::clear_image_cache,
            ipc::set_download_limit,
            ipc::config_directory,
            ipc::prefix_info,
            ipc::clear_prefixes,
            ipc::log_directory,
            ipc::set_library_root,
            ipc::list_library_roots,
            ipc::add_library_root,
            ipc::remove_library_root,
            ipc::set_default_library_root,
            ipc::set_game_options,
            ipc::game_options,
            ipc::set_extraction_options,
            ipc::set_theme,
            ipc::set_autostart,
            ipc::list_libraries,
            ipc::list_entries,
            ipc::start_download,
            ipc::install_options,
            ipc::locate_install,
            ipc::run_setup,
            ipc::run_setup_path,
            ipc::run_setup_elevated,
            ipc::rescan_library,
            ipc::install_game,
            ipc::delete_staging,
            ipc::uninstall_game,
            ipc::find_game_uninstaller,
            ipc::delete_download,
            ipc::open_path,
            ipc::open_game_folder,
            ipc::open_library_folder,
            ipc::set_game_executable,
            ipc::list_executables,
            ipc::launch_game,
            ipc::set_notification_options,
            ipc::set_window_options,
            ipc::set_auto_install,
            ipc::set_gamepad_options,
            ipc::set_umu_fixes,
            ipc::set_update_checking,
            ipc::quit_app,
            integrations::shortcut_status,
            integrations::set_shortcut,
            integrations::set_steam_shortcut,
            integrations::list_prefixes,
            integrations::delete_prefix,
            integrations::open_prefix_tool,
            integrations::umu_status,
            integrations::refresh_umu_database,
            saves::save_state,
            saves::list_save_versions,
            saves::backup_saves,
            saves::restore_saves,
            saves::resolve_save_conflict,
            saves::search_save_titles,
            saves::set_save_title,
            saves::save_paths,
            saves::set_save_cross_os,
            saves::set_save_mapping,
            saves::delete_save_version,
            saves::set_save_locked,
            saves::set_save_sync_settings,
            saves::migrate_saves,
            saves::save_tool_status,
            saves::update_save_manifest,
            saves::install_save_tool,
            saves::remove_save_tool,
            saves::test_save_store,
            updater::update_status,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Gameyfin");
}
