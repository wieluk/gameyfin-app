//! Progress on the taskbar button, so a download is visible with the window covered. Windows
//! draws it on the taskbar entry, GNOME on the dock icon.

use std::sync::atomic::{AtomicI64, Ordering};

use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager};

/// The percentage last drawn, or -1 for a hidden bar. Each update crosses to the compositor
/// or to COM, so only a change is sent.
static SHOWN: AtomicI64 = AtomicI64::new(-1);

pub fn refresh(app: &AppHandle) {
    let percent = app
        .state::<crate::state::AppState>()
        .library()
        .overall_progress()
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
