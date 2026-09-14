//! gameyfin-app shell: the window, the IPC surface and process-wide setup.
//! Everything testable lives in the `gameyfin-*` crates, which build without a GUI toolchain.

mod auth_flow;
mod cli;
mod downloads;
mod error;
mod gamepad;
mod graphics;
mod image_cache;
mod images;
mod integrations;
mod ipc;
mod library_state;
mod notify;
mod persist;
mod progress;
mod proton;
mod saves;
mod settings;
mod sidecar;
mod state;
mod taskbar;
mod tray;
mod updater;
mod wine;

use tauri::{Emitter, Manager};

use state::AppState;

/// Lets the log level change once the user's setting is known.
static LOG_RELOAD: std::sync::OnceLock<
    tracing_subscriber::reload::Handle<tracing_subscriber::EnvFilter, tracing_subscriber::Registry>,
> = std::sync::OnceLock::new();

pub fn set_log_level(level: settings::LogLevel) -> Result<(), String> {
    let Some(handle) = LOG_RELOAD.get() else {
        return Err("logging is not initialised".into());
    };
    handle
        .reload(tracing_subscriber::EnvFilter::new(level.filter()))
        .map_err(|e| format!("could not change the log level: {e}"))?;
    tracing::info!(?level, "log level changed");
    Ok(())
}

pub fn log_directory() -> std::path::PathBuf {
    dirs_log_dir().unwrap_or_else(std::env::temp_dir)
}

/// Mirrors Tauri's own resolution without an AppHandle, so logging starts before the app.
fn dirs_log_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
            })
            .map(|base| base.join("org.gameyfin.gameyfin-app/logs"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(|base| std::path::PathBuf::from(base).join("org.gameyfin.gameyfin-app/logs"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        std::env::var_os("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Library/Logs/org.gameyfin.gameyfin-app"))
    }
}

/// Console and rolling file logging. The guard must live for the process, or buffered lines
/// are lost.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // RUST_LOG still wins, for when the app will not start far enough to change the setting.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(settings::LogLevel::default().filter())
    });
    let (filter, reload_handle) = tracing_subscriber::reload::Layer::new(filter);
    let _ = LOG_RELOAD.set(reload_handle);

    let dir = log_directory();
    let file_layer = match std::fs::create_dir_all(&dir) {
        Ok(()) => {
            let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(
                &dir,
                "gameyfin.log",
            ));
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
    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer());
    match file_layer {
        Some((layer, guard)) => {
            registry.with(layer).init();
            tracing::info!("logging to {dir:?}");
            Some(guard)
        }
        None => {
            registry.init();
            None
        }
    }
}

/// Refreshes the umu database when the cached copy is a day old, so an app left running for
/// days still picks up new fixes.
async fn keep_umu_database_fresh(app: tauri::AppHandle) {
    const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(60 * 60);
    let state = app.state::<AppState>();
    loop {
        let stale = gameyfin_core::umu::cache_age(&state.config_dir())
            .is_none_or(|age| age > gameyfin_core::umu::CACHE_TTL);
        if stale && state.settings().umu_auto_update {
            // Advisory data: a failure costs per-title fixes, not the ability to play.
            if let Err(e) = state.refresh_umu_database().await {
                tracing::info!("could not refresh the umu database: {e}");
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

/// Everything that needs the settings, the session or the library, once they are readable.
async fn start_up(app: tauri::AppHandle, launched_hidden: bool) {
    let state = app.state::<AppState>();
    let config_dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("org.gameyfin.gameyfin-app"));
    let restored = state.restore(config_dir.clone()).await;
    tracing::info!(restored, "stored session restored");

    // A sign-in profile that was locked when it was cleared can be deleted now, before
    // anything has it open.
    auth_flow::sweep_stale_profiles(&app).await;

    // From cache first, so an early launch is not delayed by a fetch.
    state.load_umu_database(&config_dir);
    tauri::async_runtime::spawn(keep_umu_database_fresh(app.clone()));
    if let Some(cache) = state.image_cache() {
        tokio::task::spawn_blocking(move || cache.prune());
    }
    let settings = state.settings();
    if let Err(e) = set_log_level(settings.log_level) {
        tracing::warn!("{e}");
    }

    // Controllers are polled on their own thread; the handle carries settings changes to it.
    let pad = gamepad::Handle::new(settings.gamepad_enabled, settings.gamepad_deadzone);
    state.set_gamepad(pad.clone());
    gamepad::spawn(app.clone(), pad);

    // Asked once at startup: releases are not frequent enough to poll for.
    if settings.check_for_updates {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let status = updater::check(&app.state::<AppState>()).await;
            if let (true, Some(version)) = (status.available, status.latest_version.clone()) {
                notify::update_available(&app, &version).await;
                let _ = app.emit("update-available", status);
            }
        });
    }
    // The first fetch is 17 MB, so the first backup should not be what waits for it.
    if settings.save_sync_enabled {
        tauri::async_runtime::spawn(saves::ensure_manifest(app.clone()));
    }

    // Hiding needs somewhere to be hidden to, which only the tray provides.
    if (settings.start_minimized || launched_hidden) && tray::has_tray() {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
    }
    // The UI waits for this before deciding what to show.
    let _ = app.emit("connection-restored", restored);

    // Last, once the library is readable. A shortcut that reached a running copy goes
    // through the single-instance hook instead.
    if let Some(game_id) = cli::launch_target(&std::env::args().collect::<Vec<_>>()) {
        cli::handle_launch(&app, game_id).await;
    }
}

pub fn run() {
    // Before logging and before the single-instance plugin, which would hand the argument
    // over and leave this process printing nothing.
    let args: Vec<String> = std::env::args().collect();
    if cli::is_vulkan_probe(&args) {
        cli::run_vulkan_probe();
    }
    // A packaged app's stdout goes nowhere, so a failure in the field needs a log file.
    let _log_guard = init_logging();
    let launched_hidden = args.iter().any(|arg| arg == "--hidden");

    let mut single_instance =
        tauri_plugin_single_instance::Builder::new().callback(|app, argv, _cwd| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match cli::launch_target(&argv) {
                    Some(game_id) => cli::handle_launch(&app, game_id).await,
                    // An autostart launch must not reveal a window the user hid.
                    None if argv.iter().any(|arg| arg == "--hidden") => {}
                    // Starting it again is how people ask for the window back.
                    None => tray::reveal(&app, None),
                }
            });
        });
    // A sandbox may only own D-Bus names under its app id, and the plugin ignores being
    // refused the default one, so every Flatpak launch would start another copy.
    if let Ok(app_id) = std::env::var("FLATPAK_ID") {
        single_instance = single_instance.dbus_id(app_id);
    }

    tauri::Builder::default()
        // Registered first: it only stops the duplicate during its own setup.
        .plugin(single_instance.build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // `--hidden` matches what the autostart entry passes, so a login launch is quiet.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .register_asynchronous_uri_scheme_protocol(images::SCHEME, images::handle)
        .manage(AppState::default())
        .setup(move |app| {
            tray::install(app.handle())?;
            tray::guard_window(app.handle());
            // Found once: the bundled umu-run cannot move while the app runs.
            proton::remember_launcher(app.handle());
            // Reconnecting before the window asks lands a returning user on their library.
            tauri::async_runtime::spawn(start_up(app.handle().clone(), launched_hidden));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::session::connection_status,
            ipc::session::probe_server,
            ipc::session::set_server_url,
            ipc::session::begin_login,
            ipc::session::poll_login,
            ipc::session::cancel_login,
            ipc::session::reset_login,
            ipc::session::sign_out,
            ipc::session::quit_app,
            ipc::session::get_settings,
            ipc::session::update_settings,
            ipc::session::suggest_library_root,
            ipc::session::config_directory,
            ipc::session::log_directory,
            ipc::session::report_crash,
            ipc::session::image_cache_size,
            ipc::session::clear_image_cache,
            ipc::session::memory_info,
            ipc::session::download_providers,
            ipc::session::set_download_provider,
            ipc::session::list_library_roots,
            ipc::session::add_library_root,
            ipc::session::remove_library_root,
            ipc::session::set_default_library_root,
            ipc::session::list_libraries,
            ipc::library::list_entries,
            ipc::library::start_download,
            ipc::library::cancel_download,
            ipc::library::rescan_library,
            ipc::library::delete_staging,
            ipc::library::delete_download,
            ipc::library::open_folder,
            ipc::library::open_url,
            ipc::library::open_game_folder,
            ipc::library::open_library_folder,
            ipc::library::game_options,
            ipc::library::set_game_options,
            ipc::library::set_game_executable,
            ipc::library::list_executables,
            ipc::install::install_options,
            ipc::install::install_game,
            ipc::install::locate_install,
            ipc::install::run_setup,
            ipc::install::run_setup_path,
            ipc::install::run_setup_elevated,
            ipc::install::find_game_uninstaller,
            ipc::install::uninstall_game,
            ipc::launch::launch_game,
            ipc::launch::stop_game,
            wine::wine_status,
            wine::install_wine,
            wine::remove_wine,
            graphics::graphics_status,
            graphics::install_graphics,
            graphics::remove_graphics,
            proton::proton_status,
            proton::install_proton,
            proton::install_32bit_support,
            proton::remove_proton,
            integrations::shortcut_status,
            integrations::set_shortcut,
            integrations::set_steam_shortcut,
            integrations::list_prefixes,
            integrations::delete_prefix,
            integrations::open_prefix_tool,
            integrations::umu_status,
            integrations::refresh_umu_database,
            saves::save_state,
            saves::save_overview,
            saves::list_save_versions,
            saves::backup_saves,
            saves::restore_saves,
            saves::delete_save_versions,
            saves::set_save_locked,
            saves::delete_all_saves,
            saves::resolve_save_conflict,
            saves::search_save_titles,
            saves::set_save_title,
            saves::detected_device_name,
            saves::save_paths,
            saves::save_locations,
            saves::scan_this_pc,
            saves::skip_save_sync,
            saves::set_save_cross_os,
            saves::set_save_mapping,
            saves::migrate_saves,
            saves::save_tool_status,
            saves::update_save_manifest,
            saves::install_save_tool,
            saves::remove_save_tool,
            saves::test_save_store,
            saves::answer_save_pull_offer,
            updater::update_status,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Gameyfin");
}
