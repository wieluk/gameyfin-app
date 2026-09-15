//! The system tray icon. Downloads run in this process, so closing the window can hide it
//! instead of quitting; the setting decides.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

use crate::state::AppState;

/// Both closes arrive as the same event, so this is what tells hide from quit.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Not a given on Linux, where the appindicator library is often missing. Anything that
/// would strand the window behind a tray has to check this first.
static TRAY_PRESENT: AtomicBool = AtomicBool::new(false);

/// Readable from outside a sandbox, and created eagerly because the library will not.
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

pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    // The appindicator binding aborts rather than reporting a missing library, and no tray
    // is a degraded app where no window is a broken one.
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

    // Only the Linux branch below reassigns this.
    #[allow(unused_mut)]
    let mut builder = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("the bundled window icon is missing".into())
        })?)
        .tooltip("Gameyfin");

    // The icon is published as a path to a PNG. The default is $XDG_RUNTIME_DIR, which a
    // Flatpak keeps private, so the panel would draw an empty slot.
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(dir) = tray_icon_dir(app) {
        builder = builder.temp_dir_path(dir);
    }

    builder
        .menu(&menu)
        // On the left it would swallow the click that should toggle the window.
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
            // A left click toggles, the convention on every desktop this ships to.
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
    reveal_activated(app, route, None);
}

/// [`reveal`] with the launcher's activation token, which Wayland needs to raise the window.
pub fn reveal_activated(app: &AppHandle, route: Option<&str>, token: Option<String>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    if let Some(route) = route {
        // The frontend navigates: reloading the webview would drop every in-flight query.
        let _ = tauri::Emitter::emit(app, "navigate", route);
    }
    tauri::async_runtime::spawn(raise(window, token));
}

/// Focus is dropped for a window still hidden or minimised, and show and unminimise only land
/// once the event loop gets to them, so it is asked for a moment later.
async fn raise(window: tauri::WebviewWindow, token: Option<String>) {
    const SETTLE: Duration = Duration::from_millis(80);
    const CHECK: Duration = Duration::from_millis(250);

    tokio::time::sleep(SETTLE).await;
    match token {
        Some(token) => activate(&window, token),
        None => {
            let _ = window.set_focus();
        }
    }
    tokio::time::sleep(CHECK).await;
    if window.is_focused().unwrap_or(true) {
        return;
    }
    // Wayland refuses focus without an activation token, which a tray menu does not carry,
    // but hands it to a window mapped afresh.
    tracing::debug!("focus refused; mapping the window again");
    let _ = window.hide();
    let _ = window.show();
    tokio::time::sleep(SETTLE).await;
    let _ = window.set_focus();
    tokio::time::sleep(CHECK).await;
    if !window.is_focused().unwrap_or(true) {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

/// GTK spends the token on the next present.
#[cfg(target_os = "linux")]
fn activate(window: &tauri::WebviewWindow, token: String) {
    let target = window.clone();
    let queued = window.run_on_main_thread(move || {
        use gtk::glib::translate::ToGlibPtr;
        use gtk::prelude::*;

        let Ok(gtk_window) = target.gtk_window() else {
            return;
        };
        let display = gtk_window.display();
        if display.type_().name() == "GdkWaylandDisplay" {
            let Ok(id) = std::ffi::CString::new(token) else {
                return;
            };
            let raw: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
            // SAFETY: the display was just checked to be a GdkWaylandDisplay, and GTK copies the id.
            unsafe {
                gdk_wayland_sys::gdk_wayland_display_set_startup_notification_id(
                    raw.cast(),
                    id.as_ptr(),
                );
            }
        } else {
            gtk_window.set_startup_id(&token);
        }
        gtk_window.present_with_time(gtk::gdk::ffi::GDK_CURRENT_TIME as u32);
    });
    if queued.is_err() {
        let _ = window.set_focus();
    }
}

#[cfg(not(target_os = "linux"))]
fn activate(window: &tauri::WebviewWindow, _token: String) {
    let _ = window.set_focus();
}

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

pub fn quit_now(app: &AppHandle) {
    tracing::info!("quitting");
    QUITTING.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// True when the close was swallowed, so the caller knows to keep the window.
pub async fn intercept_close(app: &AppHandle) -> bool {
    if is_quitting() {
        return false;
    }
    // With no tray to restore from, hiding would leave a process the user cannot reach.
    if !has_tray() {
        QUITTING.store(true, Ordering::SeqCst);
        return false;
    }

    let settings = app.state::<AppState>().settings();
    if !settings.close_to_tray {
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
            // Prevented first and re-closed if it turns out to be a quit: blocking the
            // event loop on the decision would deadlock it.
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
