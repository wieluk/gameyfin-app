//! The system tray icon. Downloads run in this process, so a window close is a hide;
//! quitting is explicit, via the tray's Quit item.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

use crate::state::AppState;

/// Set once the user has genuinely asked to quit.
///
/// Without it there is no way to tell the close that should hide the window from the one
/// that should end the process, since both arrive as the same event.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Whether a tray icon actually exists.
///
/// Not a given on Linux: the appindicator library is absent on plenty of desktops and in
/// sandboxes whose runtime omits it. Everything that would strand the window behind a
/// tray that is not there has to check this first.
static TRAY_PRESENT: AtomicBool = AtomicBool::new(false);

/// A directory for the tray icon PNG that the desktop outside a sandbox can also read.
///
/// Created eagerly, because the tray library writes into it without creating it first.
#[cfg(all(unix, not(target_os = "macos")))]
fn tray_icon_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_data_dir().ok()?.join("tray");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Whether the window can be hidden and got back again.
pub fn has_tray() -> bool {
    TRAY_PRESENT.load(Ordering::SeqCst)
}

/// Whether a close request should end the application.
pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// Build the tray icon and its menu.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    // The tray is a convenience; the application is not. On Linux the appindicator
    // library it needs is frequently missing, and the binding aborts the process rather
    // than reporting it, so the whole construction is isolated: no tray is a degraded
    // app, no window at all is a broken one.
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(app)));

    match built {
        Ok(Ok(())) => {
            TRAY_PRESENT.store(true, Ordering::SeqCst);
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "no tray icon; continuing without one");
        }
        Err(_) => {
            tracing::warn!(
                "the system tray is unavailable on this desktop; continuing without one"
            );
        }
    }

    Ok(())
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open Gameyfin", true, None::<&str>)?;
    let library = MenuItem::with_id(app, "library", "Library", true, None::<&str>)?;
    let downloads = MenuItem::with_id(app, "downloads", "Downloads", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&show, &library, &downloads, &settings, &separator, &quit],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("the bundled window icon is missing".into())
        })?)
        .tooltip("Gameyfin");

    // The tray icon is published to the desktop as an absolute path to a PNG the library
    // writes out. It defaults to $XDG_RUNTIME_DIR, which inside a Flatpak is this app's
    // private mount, so the panel drawing the tray cannot read it and shows an empty slot.
    // Somewhere under the app's own data directory is visible to both.
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(dir) = tray_icon_dir(app) {
        builder = builder.temp_dir_path(dir);
    }

    builder
        .menu(&menu)
        // The menu belongs on the right button only. On the left it swallows the click
        // that should toggle the window, which is what most people try first.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => reveal(app, None),
            "library" => reveal(app, Some("/")),
            "downloads" => reveal(app, Some("/downloads")),
            "settings" => reveal(app, Some("/settings")),
            "quit" => quit_now(app),
            other => tracing::debug!("unhandled tray menu item {other}"),
        })
        .on_tray_icon_event(|tray, event| {
            // A plain left click toggles, which is the convention on every desktop this
            // ships to.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// Show the window, raise it, and optionally send it to a route.
pub fn reveal(app: &AppHandle, route: Option<&str>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    if let Some(route) = route {
        // The frontend listens for this and navigates; the alternative, reloading the
        // webview at a URL, would throw away every in-flight query and the download list.
        let _ = tauri::Emitter::emit(app, "navigate", route);
    }
}

/// Hide the window if it is showing, otherwise bring it back.
fn toggle(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let showing = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if showing {
        let _ = window.hide();
    } else {
        reveal(app, None);
    }
}

/// End the application, for real.
pub fn quit_now(app: &AppHandle) {
    tracing::info!("quitting from the tray");
    QUITTING.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// Decide what a window close means.
///
/// Called from the window's event handler. Returns true when the close was swallowed, so
/// the caller knows to prevent it.
pub async fn intercept_close(app: &AppHandle) -> bool {
    if is_quitting() {
        return false;
    }
    // Hiding with no tray to restore from would leave a running process the user cannot
    // reach, so without one the close button quits whatever the preference says.
    if !has_tray() {
        QUITTING.store(true, Ordering::SeqCst);
        return false;
    }

    let settings = app.state::<AppState>().settings().await;
    if !settings.close_to_tray {
        // The user asked for the close button to mean quit. Honour it, and mark the exit
        // as deliberate so nothing else tries to intercept it.
        QUITTING.store(true, Ordering::SeqCst);
        return false;
    }

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    tracing::debug!("window hidden to the tray rather than closed");
    true
}

/// Wire the main window's close button to [`intercept_close`].
pub fn guard_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            // The decision needs the settings, which are behind an async lock, so the
            // close is always prevented first and the window re-closed if it turns out
            // the user wanted a quit. Blocking here would deadlock the event loop.
            api.prevent_close();
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                if !intercept_close(&handle).await {
                    handle.exit(0);
                }
            });
        }
    });
}
