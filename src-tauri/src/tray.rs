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

/// Whether a close request should end the application.
pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// Build the tray icon and its menu.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
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

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("the bundled window icon is missing".into())
        })?)
        .tooltip("Gameyfin")
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
