//! Notifications: a desktop popup filtered per category, and an in-app list that keeps every one,
//! so turning the popups off loses nothing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::settings::Settings;
use crate::state::AppState;

/// What kind of event a notification reports, which decides the setting that gates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, rename = "NotificationCategory")]
pub enum Category {
    Transfer,
    /// Waits on the user, so it shows even while the window is focused.
    Action,
    Failure,
    Update,
    Proton,
    /// The server stopped accepting the login, which only signing in again fixes.
    SignedOut,
    Connection,
    Saves,
    NewGames,
    /// An installed game whose files changed on the server.
    GameUpdate,
}

impl Category {
    fn enabled_in(self, settings: &Settings) -> bool {
        match self {
            Category::Transfer | Category::Action => settings.notify_transfers,
            Category::Failure => settings.notify_failures,
            Category::Update => settings.notify_updates,
            Category::Proton => settings.notify_proton,
            Category::SignedOut => settings.notify_sign_in,
            Category::Connection => settings.notify_connection,
            Category::Saves => settings.notify_saves,
            Category::NewGames => settings.notify_new_games,
            Category::GameUpdate => settings.notify_game_updates,
        }
    }

    fn shows_while_focused(self) -> bool {
        matches!(
            self,
            Category::Action | Category::Failure | Category::SignedOut
        )
    }
}

/// One entry in the in-app list.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppNotification {
    pub id: u64,
    pub category: Category,
    pub title: String,
    pub body: String,
    pub created_at: String,
    /// The page that deals with it, opened when it is clicked.
    pub route: Option<String>,
    pub read: bool,
}

/// What happened, and where in the app it is dealt with.
pub struct Note {
    pub category: Category,
    pub title: String,
    pub body: String,
    pub route: Option<String>,
    /// Whose cover the desktop popup shows.
    pub game_id: Option<i64>,
}

/// Enough to scroll back through without growing for as long as the app runs.
const KEEP: usize = 50;

/// The in-app list, newest first. In memory: it is about this session.
#[derive(Default)]
pub struct Inbox {
    items: Mutex<VecDeque<AppNotification>>,
    next_id: AtomicU64,
}

impl Inbox {
    fn push(&self, note: &Note, created_at: String) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut items = crate::state::lock(&self.items);
        items.push_front(AppNotification {
            id,
            category: note.category,
            title: note.title.clone(),
            body: note.body.clone(),
            created_at,
            route: note.route.clone(),
            read: false,
        });
        items.truncate(KEEP);
        id
    }

    fn list(&self) -> Vec<AppNotification> {
        crate::state::lock(&self.items).iter().cloned().collect()
    }

    /// One notification, or every one with `None`.
    fn dismiss(&self, id: Option<u64>) {
        let mut items = crate::state::lock(&self.items);
        match id {
            Some(id) => items.retain(|n| n.id != id),
            None => items.clear(),
        }
    }

    fn mark_read(&self) {
        for item in crate::state::lock(&self.items).iter_mut() {
            item.read = true;
        }
    }
}

#[tauri::command]
pub fn list_notifications(inbox: State<'_, Inbox>) -> Vec<AppNotification> {
    inbox.list()
}

/// `id` of `None` dismisses them all.
#[tauri::command]
pub fn dismiss_notification(app: AppHandle, inbox: State<'_, Inbox>, id: Option<u64>) {
    inbox.dismiss(id);
    let _ = app.emit("notifications-changed", ());
}

#[tauri::command]
pub fn mark_notifications_read(app: AppHandle, inbox: State<'_, Inbox>) {
    inbox.mark_read();
    let _ = app.emit("notifications-changed", ());
}

/// Best effort: a missing daemon is no reason to fail the transfer that reported.
pub async fn send(app: &AppHandle, note: Note) {
    let id = app.state::<Inbox>().push(&note, crate::ipc::now_iso8601());
    let _ = app.emit("notifications-changed", ());

    let settings = app.state::<AppState>().settings();
    if !note.category.enabled_in(&settings) {
        tracing::debug!(category = ?note.category, "notification popup suppressed by the user's settings");
        return;
    }
    // Suppressed while the window is focused: don't tell someone what they can already see.
    if !note.category.shows_while_focused() && is_focused(app) {
        tracing::debug!(category = ?note.category, "notification popup suppressed: the window is focused");
        return;
    }

    let image = match note.game_id {
        Some(game_id) => cover_file(app, game_id).await,
        None => None,
    };
    let app = app.clone();
    #[cfg(all(unix, not(target_os = "macos")))]
    tauri::async_runtime::spawn(desktop::show(app, note, id, image));
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    std::thread::spawn(move || desktop::show(&app, &note, id, image));
}

/// Whether the main window is the one the user is looking at.
pub fn is_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|window| {
            let visible = window.is_visible().unwrap_or(false);
            let focused = window.is_focused().unwrap_or(false);
            let minimized = window.is_minimized().unwrap_or(false);
            visible && focused && !minimized
        })
        .unwrap_or(false)
}

/// The game's cover as a file a notification daemon can read, fetched into the cache if needed.
async fn cover_file(app: &AppHandle, game_id: i64) -> Option<std::path::PathBuf> {
    let state = app.state::<AppState>();
    let game = state.game(game_id).await.ok()?;
    let path = game.cover.as_ref()?.path();
    crate::images::artwork(&state, &path).await.ok()?;
    state.image_cache()?.file_for(&path)
}

mod desktop {
    use super::Note;
    use std::path::PathBuf;
    use tauri::AppHandle;

    /// Open popups, oldest first. Each holds its own D-Bus connection, which GNOME needs kept.
    #[cfg(all(unix, not(target_os = "macos")))]
    static OPEN: std::sync::Mutex<
        std::collections::VecDeque<(u64, std::sync::Arc<notify_rust::NotificationHandle>)>,
    > = std::sync::Mutex::new(std::collections::VecDeque::new());

    /// A daemon that keeps popups in its history never reports them closed.
    #[cfg(all(unix, not(target_os = "macos")))]
    const MAX_OPEN: usize = 10;

    /// Waits for a click, then raises the window on the page it is about and takes it off the list.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub async fn show(app: AppHandle, note: Note, id: u64, image: Option<PathBuf>) {
        use notify_rust::NotificationResponse;
        use tauri::{Emitter, Manager};

        let mut popup = notify_rust::Notification::new();
        popup.summary(&note.title).body(&note.body);
        dress(&app, &mut popup, &note, image);
        let handle = match popup.show_async().await {
            Ok(handle) => std::sync::Arc::new(handle),
            Err(e) => return tracing::debug!("could not show a notification: {e}"),
        };
        let evicted = super::admit(
            &mut crate::state::lock(&OPEN),
            (id, handle.clone()),
            MAX_OPEN,
        );
        if let Some((_, oldest)) = evicted {
            // Its own wait ends once the daemon reports it closed.
            oldest.close_async().await;
        }
        handle
            .wait_for_action_async(|response| {
                let clicked = match response {
                    NotificationResponse::Default => true,
                    NotificationResponse::Action(action) => action == "default",
                    _ => false,
                };
                if clicked {
                    crate::tray::reveal(&app, note.route.as_deref());
                    app.state::<super::Inbox>().dismiss(Some(id));
                    let _ = app.emit("notifications-changed", ());
                }
            })
            .await;
        crate::state::lock(&OPEN).retain(|(open, _)| *open != id);
    }

    /// Windows activates the app itself, which reaches the running copy through single-instance.
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    pub fn show(app: &AppHandle, note: &Note, _id: u64, image: Option<PathBuf>) {
        let mut popup = notify_rust::Notification::new();
        popup.summary(&note.title).body(&note.body);
        dress(app, &mut popup, note, image);
        if let Err(e) = popup.show() {
            tracing::debug!("could not show a notification: {e}");
        }
    }

    /// Filed under the app's desktop entry with its icon, or a Flatpak shows a blank file icon.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn dress(
        app: &AppHandle,
        popup: &mut notify_rust::Notification,
        note: &Note,
        image: Option<PathBuf>,
    ) {
        use notify_rust::Hint;

        let (entry, icon) = super::identity(std::env::var("FLATPAK_ID").ok().as_deref());
        popup.appname("Gameyfin").hint(Hint::DesktopEntry(entry));
        if let Some(icon) = icon.or_else(|| icon_file(app)) {
            popup.icon(&icon);
        }
        if note.category == super::Category::Transfer {
            popup.hint(Hint::Category("transfer.complete".into()));
        }
        if let Some(image) = image {
            popup.image_path(&image.to_string_lossy());
        }
        // A click is the default action, and without one the daemon has nothing to call back.
        popup.action("default", "Open");
    }

    /// A dev build has no registered app id, so Windows would drop the toast.
    #[cfg(windows)]
    fn dress(
        app: &AppHandle,
        popup: &mut notify_rust::Notification,
        _note: &Note,
        _image: Option<PathBuf>,
    ) {
        if !tauri::is_dev() {
            popup.app_id(&app.config().identifier);
        }
    }

    #[cfg(target_os = "macos")]
    fn dress(
        app: &AppHandle,
        _popup: &mut notify_rust::Notification,
        _note: &Note,
        _image: Option<PathBuf>,
    ) {
        let _ = notify_rust::set_application(if tauri::is_dev() {
            "com.apple.Terminal"
        } else {
            &app.config().identifier
        });
    }

    /// The app's icon as a file, since only a Flatpak or a package puts one in the icon theme.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn icon_file(app: &AppHandle) -> Option<String> {
        static ICON: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        use tauri::Manager;

        ICON.get_or_init(|| {
            let dir = app.path().app_data_dir().ok()?;
            std::fs::create_dir_all(&dir).ok()?;
            let file = dir.join("notification-icon.png");
            std::fs::write(&file, include_bytes!("../icons/128x128.png")).ok()?;
            Some(file.to_string_lossy().into_owned())
        })
        .clone()
    }
}

/// Adds `item`, handing back the oldest once there are more than `max`.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn admit<T>(open: &mut VecDeque<T>, item: T, max: usize) -> Option<T> {
    open.push_back(item);
    (open.len() > max).then(|| open.pop_front()).flatten()
}

/// The desktop entry a popup is filed under, and its icon name. `None` means the bundled file.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn identity(flatpak_id: Option<&str>) -> (String, Option<String>) {
    match flatpak_id.filter(|id| !id.is_empty()) {
        // A Flatpak exports both its desktop file and its icon under the app id.
        Some(id) => (id.to_string(), Some(id.to_string())),
        None => ("gameyfin-app".to_string(), None),
    }
}

/// Launch failures and trouble with an installed game are dealt with under Installed.
fn failure_route(stage: &str, installed: bool) -> &'static str {
    if stage == "Launch" || (stage == "Install" && installed) {
        "/installed"
    } else {
        "/downloads"
    }
}

pub async fn download_finished(app: &AppHandle, game_id: i64, title: &str) {
    send(
        app,
        Note {
            category: Category::Transfer,
            title: "Download finished".into(),
            body: format!("{title} is ready to install."),
            route: Some("/downloads".into()),
            game_id: Some(game_id),
        },
    )
    .await;
}

/// `route` is where the waiting setup is run from.
pub async fn setup_needed(app: &AppHandle, game_id: i64, route: &'static str, body: &str) {
    send(
        app,
        Note {
            category: Category::Action,
            title: "Setup needed".into(),
            body: body.to_string(),
            route: Some(route.into()),
            game_id: Some(game_id),
        },
    )
    .await;
}

pub async fn install_finished(app: &AppHandle, game_id: i64, title: &str) {
    send(
        app,
        Note {
            category: Category::Transfer,
            title: "Ready to play".into(),
            body: format!("{title} is installed."),
            route: Some("/installed".into()),
            game_id: Some(game_id),
        },
    )
    .await;
}

/// The stage goes in the title: a notification is often read as one line.
pub async fn failed(app: &AppHandle, game_id: i64, stage: &str, title: &str, reason: &str) {
    let installed = app
        .state::<AppState>()
        .library()
        .record(game_id)
        .is_installed();
    send(
        app,
        Note {
            category: Category::Failure,
            title: format!("{stage} failed"),
            body: format!("{title}: {reason}"),
            route: Some(failure_route(stage, installed).into()),
            game_id: Some(game_id),
        },
    )
    .await;
}

pub async fn update_available(app: &AppHandle, version: &str) {
    send(
        app,
        Note {
            category: Category::Update,
            title: "Update available".into(),
            body: format!("Gameyfin {version} has been released."),
            route: Some("/settings?tab=about".into()),
            game_id: None,
        },
    )
    .await;
}

/// Leads to the Proton builds, which head the Compatibility tab.
pub async fn proton_update(app: &AppHandle, family: &str, tag: &str) {
    send(
        app,
        Note {
            category: Category::Proton,
            title: format!("New {family}"),
            body: format!("{tag} is ready to download."),
            route: Some("/settings?tab=compatibility".into()),
            game_id: None,
        },
    )
    .await;
}

/// No route: signed out, the window shows the sign-in wherever it was.
pub async fn signed_out(app: &AppHandle, server: &str) {
    send(
        app,
        Note {
            category: Category::SignedOut,
            title: "Signed out".into(),
            body: format!("Sign in to {server} again to keep downloads and save sync going."),
            route: None,
            game_id: None,
        },
    )
    .await;
}

pub async fn server_unreachable(app: &AppHandle, server: &str) {
    send(
        app,
        Note {
            category: Category::Connection,
            title: "Server unreachable".into(),
            body: format!("{server} is not answering. Installed games still play."),
            route: None,
            game_id: None,
        },
    )
    .await;
}

pub async fn server_back(app: &AppHandle, server: &str) {
    send(
        app,
        Note {
            category: Category::Connection,
            title: "Back online".into(),
            body: format!("{server} is answering again."),
            route: None,
            game_id: None,
        },
    )
    .await;
}

pub async fn save_sync_failed(app: &AppHandle, game_id: i64, title: &str, reason: &str) {
    send(
        app,
        Note {
            category: Category::Saves,
            title: "Save sync failed".into(),
            body: format!("{title}: {reason}"),
            route: Some("/saves".into()),
            game_id: Some(game_id),
        },
    )
    .await;
}

/// `route` is `None` when the question is a dialog over whatever page is open.
pub async fn save_choice_needed(app: &AppHandle, game_id: i64, title: &str, route: Option<String>) {
    send(
        app,
        Note {
            category: Category::Saves,
            title: "Choose a save".into(),
            body: format!("{title}: pick which save to use."),
            route,
            game_id: Some(game_id),
        },
    )
    .await;
}

/// One notification for a batch, since a scan can add many games at once.
pub async fn new_games(app: &AppHandle, games: &[(i64, String)]) {
    let titles: Vec<&str> = games.iter().map(|(_, title)| title.as_str()).collect();
    let Some((title, body)) = new_games_text(&titles) else {
        return;
    };
    send(
        app,
        Note {
            category: Category::NewGames,
            title,
            body,
            route: Some("/".into()),
            game_id: (games.len() == 1).then(|| games[0].0),
        },
    )
    .await;
}

fn new_games_text(titles: &[&str]) -> Option<(String, String)> {
    Some(match titles {
        [] => return None,
        [one] => (
            "New game".into(),
            format!("{one} was added to the library."),
        ),
        [a, b] => ("2 new games".into(), format!("{a} and {b} were added.")),
        [a, b, rest @ ..] => (
            format!("{} new games", titles.len()),
            format!("{a}, {b} and {} more were added.", rest.len()),
        ),
    })
}

/// Stock servers have no versions, so new files show as the game's size changing.
pub async fn game_files_changed(app: &AppHandle, game_id: i64, title: &str) {
    send(
        app,
        Note {
            category: Category::GameUpdate,
            title: "Game updated on the server".into(),
            body: format!("{title} has new files. Download it again to update."),
            route: Some(format!("/installed?game={game_id}")),
            game_id: Some(game_id),
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str) -> Note {
        Note {
            category: Category::Transfer,
            title: title.into(),
            body: String::new(),
            route: Some("/downloads".into()),
            game_id: None,
        }
    }

    #[test]
    fn each_category_reads_its_own_setting() {
        let mut settings = Settings::default();
        assert!(Category::Transfer.enabled_in(&settings));
        assert!(Category::Failure.enabled_in(&settings));
        assert!(Category::Update.enabled_in(&settings));
        assert!(Category::Proton.enabled_in(&settings));

        settings.notify_transfers = false;
        assert!(!Category::Transfer.enabled_in(&settings));
        assert!(Category::Failure.enabled_in(&settings));
        assert!(Category::Update.enabled_in(&settings));

        settings.notify_failures = false;
        assert!(!Category::Failure.enabled_in(&settings));

        settings.notify_proton = false;
        assert!(!Category::Proton.enabled_in(&settings));
        assert!(Category::Update.enabled_in(&settings));
    }

    #[test]
    fn the_server_watch_has_a_switch_per_kind() {
        let watched = [
            Category::SignedOut,
            Category::Connection,
            Category::Saves,
            Category::NewGames,
            Category::GameUpdate,
        ];
        let mut settings = Settings::default();
        assert!(watched.iter().all(|c| c.enabled_in(&settings)));

        settings.notify_sign_in = false;
        settings.notify_connection = false;
        settings.notify_saves = false;
        settings.notify_new_games = false;
        settings.notify_game_updates = false;
        assert!(watched.iter().all(|c| !c.enabled_in(&settings)));
        assert!(Category::Failure.enabled_in(&settings));
    }

    #[test]
    fn only_what_needs_the_user_interrupts_a_focused_window() {
        assert!(Category::Action.shows_while_focused() && Category::Failure.shows_while_focused());
        assert!(Category::SignedOut.shows_while_focused());
        assert!(
            !Category::Connection.shows_while_focused() && !Category::Saves.shows_while_focused()
        );
        assert!(
            !Category::Transfer.shows_while_focused()
                && !Category::Update.shows_while_focused()
                && !Category::Proton.shows_while_focused()
        );
    }

    #[test]
    fn the_inbox_keeps_the_newest_and_forgets_what_is_dismissed() {
        let inbox = Inbox::default();
        let first = inbox.push(&note("first"), String::new());
        let second = inbox.push(&note("second"), String::new());
        let titles: Vec<String> = inbox.list().into_iter().map(|n| n.title).collect();
        assert_eq!(titles, ["second", "first"]);
        assert_ne!(first, second);

        inbox.mark_read();
        assert!(inbox.list().iter().all(|n| n.read));
        inbox.dismiss(Some(first));
        assert_eq!(inbox.list().len(), 1);
        inbox.dismiss(None);
        assert!(inbox.list().is_empty());

        for i in 0..KEEP + 5 {
            inbox.push(&note(&i.to_string()), String::new());
        }
        assert_eq!(inbox.list().len(), KEEP);
        assert_eq!(inbox.list()[0].title, (KEEP + 4).to_string());
    }

    #[test]
    fn a_flatpak_popup_is_filed_under_its_app_id() {
        assert_eq!(
            identity(Some("org.gameyfin.gameyfin-app")),
            (
                "org.gameyfin.gameyfin-app".to_string(),
                Some("org.gameyfin.gameyfin-app".to_string())
            )
        );
        assert_eq!(identity(None), ("gameyfin-app".to_string(), None));
        assert_eq!(identity(Some("")), ("gameyfin-app".to_string(), None));
    }

    #[test]
    fn only_the_newest_popups_stay_open() {
        let mut open = VecDeque::new();
        assert_eq!(admit(&mut open, 1, 2), None);
        assert_eq!(admit(&mut open, 2, 2), None);
        assert_eq!(admit(&mut open, 3, 2), Some(1));
        assert_eq!(Vec::from(open), [2, 3]);
    }

    #[test]
    fn new_games_are_summed_up_in_one_line() {
        assert_eq!(new_games_text(&[]), None);
        assert_eq!(
            new_games_text(&["Hades"]).unwrap().1,
            "Hades was added to the library."
        );
        assert_eq!(
            new_games_text(&["Hades", "Celeste"]).unwrap(),
            ("2 new games".into(), "Hades and Celeste were added.".into())
        );
        assert_eq!(
            new_games_text(&["Hades", "Celeste", "Tunic", "Inside"]).unwrap(),
            (
                "4 new games".into(),
                "Hades, Celeste and 2 more were added.".into()
            )
        );
    }

    #[test]
    fn a_failure_leads_to_where_it_is_fixed() {
        assert_eq!(failure_route("Launch", true), "/installed");
        assert_eq!(failure_route("Install", true), "/installed");
        assert_eq!(failure_route("Install", false), "/downloads");
        assert_eq!(failure_route("Download", true), "/downloads");
    }
}
