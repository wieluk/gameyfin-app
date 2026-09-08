//! Desktop notifications.
//!
//! The events worth interrupting someone for are the ones they are not watching: a
//! download that took twenty minutes, an install that finished while they were in another
//! window, a failure that would otherwise sit as red text on a tab nobody is looking at.
//!
//! Every notification is filtered twice, by category and by whether the user asked for
//! that category at all, so turning the routine ones off still leaves failures audible.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::settings::Settings;
use crate::state::AppState;

/// What kind of event a notification reports, which decides the setting that gates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// A download or an install reached the end.
    Transfer,
    /// Something went wrong.
    Failure,
    /// A new version of the app exists.
    Update,
}

impl Category {
    fn enabled_in(self, settings: &Settings) -> bool {
        match self {
            Category::Transfer => settings.notify_transfers,
            Category::Failure => settings.notify_failures,
            Category::Update => settings.notify_updates,
        }
    }
}

/// Show a notification, if the user wants this kind.
///
/// Best effort throughout. A desktop with no notification daemon, a Flatpak whose portal
/// is unavailable, a Windows session with focus assist on: none of these are reasons to
/// fail the download that triggered the message.
pub async fn send(app: &AppHandle, category: Category, title: &str, body: &str) {
    let settings = app.state::<AppState>().settings().await;
    if !category.enabled_in(&settings) {
        tracing::debug!(?category, "notification suppressed by the user's settings");
        return;
    }

    // Suppressed while the user is looking at the window: telling someone what they can
    // already see is the fastest way to get every notification turned off.
    if category != Category::Failure && is_focused(app) {
        tracing::debug!(?category, "notification suppressed: the window is focused");
        return;
    }

    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::debug!("could not show a notification: {e}");
    }
}

/// Whether the main window is on screen and has the user's attention.
fn is_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|window| {
            let visible = window.is_visible().unwrap_or(false);
            let focused = window.is_focused().unwrap_or(false);
            let minimized = window.is_minimized().unwrap_or(false);
            visible && focused && !minimized
        })
        .unwrap_or(false)
}

/// A finished download, waiting to be installed.
pub async fn download_finished(app: &AppHandle, title: &str) {
    send(
        app,
        Category::Transfer,
        "Download finished",
        &format!("{title} is ready to install."),
    )
    .await;
}

/// A finished install, ready to play.
pub async fn install_finished(app: &AppHandle, title: &str) {
    send(
        app,
        Category::Transfer,
        "Ready to play",
        &format!("{title} is installed."),
    )
    .await;
}

/// Something failed.
///
/// The stage is in the title rather than the body because a notification is often read as
/// one line on a lock screen, and "Install failed" alone is more use than a truncated
/// sentence about which file could not be written.
pub async fn failed(app: &AppHandle, stage: &str, title: &str, reason: &str) {
    send(
        app,
        Category::Failure,
        &format!("{stage} failed"),
        &format!("{title}: {reason}"),
    )
    .await;
}

/// A new release exists.
pub async fn update_available(app: &AppHandle, version: &str) {
    send(
        app,
        Category::Update,
        "Update available",
        &format!("Gameyfin {version} has been released."),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_category_reads_its_own_setting() {
        let mut settings = Settings::default();
        assert!(Category::Transfer.enabled_in(&settings));
        assert!(Category::Failure.enabled_in(&settings));
        assert!(Category::Update.enabled_in(&settings));

        settings.notify_transfers = false;
        assert!(!Category::Transfer.enabled_in(&settings));
        // Turning the routine ones off must leave failures alone.
        assert!(Category::Failure.enabled_in(&settings));
        assert!(Category::Update.enabled_in(&settings));

        settings.notify_failures = false;
        assert!(!Category::Failure.enabled_in(&settings));
    }
}
