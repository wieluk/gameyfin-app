//! Progress on the taskbar button, so a download is visible with the window covered.
//!
//! Windows draws it on the taskbar entry, GNOME on the dock icon; elsewhere it is a no-op.

use std::sync::atomic::{AtomicI64, Ordering};

use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager};

/// The percentage last handed to the window, or -1 for a hidden bar.
///
/// Progress arrives several times a second, and each update crosses to the compositor or
/// to COM, so only a change worth drawing is sent. App-wide because the bar is: on Linux
/// it belongs to the desktop entry rather than to a window.
static SHOWN: AtomicI64 = AtomicI64::new(-1);

/// Push the combined progress of everything in flight to the taskbar.
pub async fn refresh(app: &AppHandle) {
    let library = app.state::<crate::state::AppState>().library_handle();
    let percent = library
        .overall_progress()
        .await
        .map(|p| p.clamp(0.0, 100.0).round() as i64)
        .unwrap_or(-1);

    if SHOWN.swap(percent, Ordering::Relaxed) == percent {
        return;
    }
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let state = if percent < 0 {
        ProgressBarState {
            status: Some(ProgressBarStatus::None),
            progress: None,
        }
    } else {
        ProgressBarState {
            status: Some(ProgressBarStatus::Normal),
            progress: Some(percent as u64),
        }
    };

    if let Err(e) = window.set_progress_bar(state) {
        tracing::debug!(error = %e, "could not set the taskbar progress");
    }
}

/// Refresh from a synchronous caller. See [`refresh`].
pub fn refresh_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { refresh(&app).await });
}
