//! Desktop notifications, filtered per category so turning the routine ones off still
//! leaves failures audible.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::settings::Settings;
use crate::state::AppState;

/// What kind of event a notification reports, which decides the setting that gates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Transfer,
    /// Waits on the user, so it shows even while the window is focused.
    Action,
    Failure,
    Update,
}

impl Category {
    fn enabled_in(self, settings: &Settings) -> bool {
        match self {
            Category::Transfer | Category::Action => settings.notify_transfers,
            Category::Failure => settings.notify_failures,
            Category::Update => settings.notify_updates,
        }
    }

    fn shows_while_focused(self) -> bool {
        matches!(self, Category::Action | Category::Failure)
    }
}

/// Best effort: a missing daemon is no reason to fail the transfer that reported.
pub async fn send(app: &AppHandle, category: Category, title: &str, body: &str) {
    let settings = app.state::<AppState>().settings();
    if !category.enabled_in(&settings) {
        tracing::debug!(?category, "notification suppressed by the user's settings");
        return;
    }

    // Suppressed while the window is focused: don't tell someone what they can already see.
    if !category.shows_while_focused() && is_focused(app) {
        tracing::debug!(?category, "notification suppressed: the window is focused");
        return;
    }

    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::debug!("could not show a notification: {e}");
    }
}

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

pub async fn download_finished(app: &AppHandle, title: &str) {
    send(
        app,
        Category::Transfer,
        "Download finished",
        &format!("{title} is ready to install."),
    )
    .await;
}

pub async fn setup_needed(app: &AppHandle, body: &str) {
    send(app, Category::Action, "Setup needed", body).await;
}

pub async fn install_finished(app: &AppHandle, title: &str) {
    send(
        app,
        Category::Transfer,
        "Ready to play",
        &format!("{title} is installed."),
    )
    .await;
}

/// The stage goes in the title: a notification is often read as one line.
pub async fn failed(app: &AppHandle, stage: &str, title: &str, reason: &str) {
    send(
        app,
        Category::Failure,
        &format!("{stage} failed"),
        &format!("{title}: {reason}"),
    )
    .await;
}

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
        assert!(Category::Failure.enabled_in(&settings));
        assert!(Category::Update.enabled_in(&settings));

        settings.notify_failures = false;
        assert!(!Category::Failure.enabled_in(&settings));
    }

    #[test]
    fn only_what_needs_the_user_interrupts_a_focused_window() {
        assert!(Category::Action.shows_while_focused() && Category::Failure.shows_while_focused());
        assert!(
            !Category::Transfer.shows_while_focused() && !Category::Update.shows_while_focused()
        );
    }
}
