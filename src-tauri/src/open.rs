//! Opening folders and links in front of Gameyfin. Started without an activation token, as
//! `xdg-open` and the Flatpak portal start them, a Wayland compositor keeps the window behind.

use std::path::Path;

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::error::{CommandResult, Context};

pub fn folder(app: &AppHandle, dir: &Path) -> CommandResult<()> {
    #[cfg(target_os = "linux")]
    if let Ok(uri) = url::Url::from_directory_path(dir) {
        if gtk_launch::show(app, uri.as_str()) {
            return Ok(());
        }
    }
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .context(format!("could not open {}", dir.display()))
}

pub fn link(app: &AppHandle, url: &url::Url) -> CommandResult<()> {
    #[cfg(target_os = "linux")]
    if gtk_launch::show(app, url.as_str()) {
        return Ok(());
    }
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .context("could not open the link")
}

#[cfg(target_os = "linux")]
mod gtk_launch {
    use std::sync::mpsc;
    use std::time::Duration;

    use tauri::{AppHandle, Manager};

    /// With our window as parent, GTK's launch context asks the compositor for the token and
    /// hands it on, portal included. False when it could not, for the plain opener to try.
    pub fn show(app: &AppHandle, uri: &str) -> bool {
        let Some(window) = app.get_webview_window("main") else {
            return false;
        };
        let (sent, answer) = mpsc::channel();
        let (parent_of, target) = (window.clone(), uri.to_string());
        // GTK only runs on the main thread.
        let queued = window.run_on_main_thread(move || {
            let shown = parent_of
                .gtk_window()
                .map_err(|e| e.to_string())
                .and_then(|parent| {
                    gtk::show_uri_on_window(
                        Some(&parent),
                        &target,
                        gtk::gdk::ffi::GDK_CURRENT_TIME as u32,
                    )
                    .map_err(|e| e.to_string())
                });
            let _ = sent.send(shown);
        });
        if queued.is_err() {
            return false;
        }
        match answer.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                tracing::debug!("GTK could not open {uri}: {e}");
                false
            }
            // Still queued, and falling back now would open it twice.
            Err(_) => true,
        }
    }
}
