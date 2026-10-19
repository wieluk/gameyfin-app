//! Watches the server while the app runs, so a lost sign-in, an outage, new games and changed
//! game files are told even with the window in the tray. Uses only what a stock server serves.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

const WATCH_FILE: &str = "server-watch.json";

/// One request asking who is signed in, so it can run often.
const TICK: Duration = Duration::from_secs(2 * 60);
/// The catalogue is the whole library, so it is fetched every few ticks.
const CATALOG_EVERY: u32 = 5;
/// Failed checks in a row before an outage is told, so a blip or a resume from sleep is not.
const OUTAGE_AFTER: u32 = 2;

/// What the last check saw, kept across restarts so games added while the app was closed count.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Seen {
    /// The server and user the games belong to; another account starts afresh.
    account: Option<String>,
    game_ids: BTreeSet<i64>,
    /// The server size already told per game, so each change is told once.
    announced_sizes: BTreeMap<i64, u64>,
}

/// The connection as this run has seen it.
#[derive(Debug, Default)]
struct Link {
    failures: u32,
    told_outage: bool,
    told_sign_out: bool,
    /// `(authenticated, offline)`, so the window hears about a change straight away.
    last: Option<(bool, bool)>,
}

#[derive(Debug, PartialEq, Eq)]
enum LinkEvent {
    SignedOut,
    Unreachable,
    Back,
}

/// A catalogue entry, reduced to what the checks compare.
struct Listed<'a> {
    id: i64,
    title: &'a str,
    size: u64,
}

pub async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    let file = state.config_dir().join(WATCH_FILE);
    let mut seen: Seen = crate::persist::read_json_or_default(&file).await;
    let mut link = Link::default();
    let mut ticks = 0u32;
    loop {
        tokio::time::sleep(TICK).await;
        let settings = state.settings();
        // Signed out on purpose, or still connecting at startup: nothing to watch.
        if !settings.has_session() || state.client().is_none() {
            link = Link::default();
            continue;
        }
        let server = server_name(settings.server_url.as_deref());
        let (authenticated, offline) = state.check_session(true).await;
        if link.last != Some((authenticated, offline)) {
            link.last = Some((authenticated, offline));
            let _ = app.emit("connection-changed", ());
        }
        match link_event(&mut link, authenticated, offline) {
            Some(LinkEvent::SignedOut) => crate::notify::signed_out(&app, &server).await,
            Some(LinkEvent::Unreachable) => crate::notify::server_unreachable(&app, &server).await,
            Some(LinkEvent::Back) => crate::notify::server_back(&app, &server).await,
            None => {}
        }

        if authenticated && !offline && ticks.is_multiple_of(CATALOG_EVERY) {
            let account = format!(
                "{}|{}",
                settings.server_url.unwrap_or_default(),
                settings.username.unwrap_or_default()
            );
            let before = seen.clone();
            check_catalog(&app, &mut seen, &account).await;
            if seen != before {
                if let Err(e) = crate::persist::write_json(&file, &seen, false).await {
                    tracing::warn!("could not record what the server holds: {e}");
                }
            }
        }
        ticks = ticks.wrapping_add(1);
    }
}

async fn check_catalog(app: &AppHandle, seen: &mut Seen, account: &str) {
    let state = app.state::<AppState>();
    let games = match state.games().await {
        Ok(games) => games,
        Err(e) => return tracing::debug!("could not check the library for changes: {e}"),
    };
    let listed: Vec<Listed> = games
        .iter()
        .map(|g| Listed {
            id: g.id,
            title: &g.title,
            size: g.metadata.file_size,
        })
        .collect();

    let added = added_games(seen, account, &listed);
    if !added.is_empty() {
        crate::notify::new_games(app, &added).await;
    }

    let library = state.library();
    let sizes = changed_sizes(seen, &listed, |id| {
        let record = library.record(id);
        record.is_installed().then_some(record.source_bytes)
    });
    for (game_id, size) in sizes.baselines {
        library
            .update_record(game_id, |r| r.source_bytes = size)
            .await;
    }
    for (game_id, title) in sizes.changed {
        crate::notify::game_files_changed(app, game_id, &title).await;
    }
}

/// The server's host, which is how people know it.
fn server_name(url: Option<&str>) -> String {
    url.and_then(|u| reqwest::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "Gameyfin".to_string())
}

/// What to tell about this check, at most once per change.
fn link_event(link: &mut Link, authenticated: bool, offline: bool) -> Option<LinkEvent> {
    if offline {
        link.failures += 1;
        if link.failures >= OUTAGE_AFTER && !link.told_outage {
            link.told_outage = true;
            return Some(LinkEvent::Unreachable);
        }
        return None;
    }
    link.failures = 0;
    let back = std::mem::take(&mut link.told_outage);
    if !authenticated {
        // Back but signed out: signing in is the news.
        if !link.told_sign_out {
            link.told_sign_out = true;
            return Some(LinkEvent::SignedOut);
        }
        return None;
    }
    link.told_sign_out = false;
    back.then_some(LinkEvent::Back)
}

/// Games new since the last check, after which `seen` holds the current library. The first
/// check for an account only records it, or every game would be new.
fn added_games(seen: &mut Seen, account: &str, games: &[Listed]) -> Vec<(i64, String)> {
    // An empty answer is more likely a hiccup than a wiped library, and taking it as one
    // would make the whole library new on the next check.
    if games.is_empty() {
        return Vec::new();
    }
    let known = seen.account.as_deref() == Some(account) && !seen.game_ids.is_empty();
    if seen.account.as_deref() != Some(account) {
        seen.announced_sizes.clear();
    }
    let added = if known {
        games
            .iter()
            .filter(|g| !seen.game_ids.contains(&g.id))
            .map(|g| (g.id, g.title.to_string()))
            .collect()
    } else {
        Vec::new()
    };
    seen.account = Some(account.to_string());
    seen.game_ids = games.iter().map(|g| g.id).collect();
    added
}

#[derive(Debug, Default, PartialEq)]
struct SizeChanges {
    /// Installed before sizes were recorded, so today's size is taken as what is installed.
    baselines: Vec<(i64, u64)>,
    changed: Vec<(i64, String)>,
}

/// Installed games whose server size is no longer the size that was downloaded. `downloaded`
/// gives that size for an installed game, zero when it was never recorded.
fn changed_sizes(
    seen: &mut Seen,
    games: &[Listed],
    downloaded: impl Fn(i64) -> Option<u64>,
) -> SizeChanges {
    let mut out = SizeChanges::default();
    for game in games {
        // A server that has not measured the game yet reports nothing to compare.
        if game.size == 0 {
            continue;
        }
        match downloaded(game.id) {
            Some(0) => out.baselines.push((game.id, game.size)),
            Some(local) if local != game.size => {
                if seen.announced_sizes.get(&game.id) != Some(&game.size) {
                    seen.announced_sizes.insert(game.id, game.size);
                    out.changed.push((game.id, game.title.to_string()));
                }
            }
            // Up to date again, or no longer installed.
            _ => {
                seen.announced_sizes.remove(&game.id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(games: &[(i64, &'static str, u64)]) -> Vec<Listed<'static>> {
        games
            .iter()
            .map(|&(id, title, size)| Listed { id, title, size })
            .collect()
    }

    #[test]
    fn an_outage_is_told_once_after_two_failed_checks() {
        let mut link = Link::default();
        assert_eq!(link_event(&mut link, true, false), None);
        assert_eq!(link_event(&mut link, true, true), None);
        assert_eq!(
            link_event(&mut link, true, true),
            Some(LinkEvent::Unreachable)
        );
        assert_eq!(link_event(&mut link, true, true), None);
        assert_eq!(link_event(&mut link, true, false), Some(LinkEvent::Back));
        assert_eq!(link_event(&mut link, true, false), None);
    }

    #[test]
    fn a_blip_is_not_an_outage() {
        let mut link = Link::default();
        assert_eq!(link_event(&mut link, true, true), None);
        assert_eq!(link_event(&mut link, true, false), None);
        assert_eq!(link_event(&mut link, true, true), None);
    }

    #[test]
    fn a_lost_login_is_told_once_until_signed_in_again() {
        let mut link = Link::default();
        assert_eq!(
            link_event(&mut link, false, false),
            Some(LinkEvent::SignedOut)
        );
        assert_eq!(link_event(&mut link, false, false), None);
        assert_eq!(link_event(&mut link, true, false), None);
        assert_eq!(
            link_event(&mut link, false, false),
            Some(LinkEvent::SignedOut)
        );
    }

    #[test]
    fn coming_back_signed_out_tells_only_the_sign_out() {
        let mut link = Link::default();
        link_event(&mut link, true, true);
        link_event(&mut link, true, true);
        assert_eq!(
            link_event(&mut link, false, false),
            Some(LinkEvent::SignedOut)
        );
    }

    #[test]
    fn the_first_look_at_a_library_only_records_it() {
        let mut seen = Seen::default();
        let games = listed(&[(1, "Hades", 10), (2, "Celeste", 20)]);
        assert!(added_games(&mut seen, "a", &games).is_empty());

        let games = listed(&[(1, "Hades", 10), (2, "Celeste", 20), (3, "Tunic", 30)]);
        assert_eq!(added_games(&mut seen, "a", &games), [(3, "Tunic".into())]);
        assert!(added_games(&mut seen, "a", &games).is_empty());
    }

    #[test]
    fn another_account_or_an_empty_answer_announces_nothing() {
        let mut seen = Seen::default();
        added_games(&mut seen, "a", &listed(&[(1, "Hades", 10)]));
        assert!(added_games(&mut seen, "a", &[]).is_empty());
        assert_eq!(seen.game_ids.len(), 1);

        let other = listed(&[(5, "Tunic", 10), (6, "Inside", 10)]);
        assert!(added_games(&mut seen, "b", &other).is_empty());
        assert_eq!(seen.account.as_deref(), Some("b"));
    }

    #[test]
    fn a_changed_size_of_an_installed_game_is_told_once() {
        let mut seen = Seen::default();
        // 1 is installed at the server's size, 2 at an older one, 3 is not installed.
        let installed = |id| match id {
            1 => Some(10),
            2 => Some(15),
            _ => None,
        };
        let games = listed(&[(1, "Hades", 10), (2, "Celeste", 20), (3, "Tunic", 30)]);
        let changes = changed_sizes(&mut seen, &games, installed);
        assert_eq!(changes.changed, [(2, "Celeste".into())]);
        assert!(changes.baselines.is_empty());
        assert!(changed_sizes(&mut seen, &games, installed)
            .changed
            .is_empty());

        // Another change is news again.
        let games = listed(&[(2, "Celeste", 25)]);
        assert_eq!(
            changed_sizes(&mut seen, &games, installed).changed,
            [(2, "Celeste".into())]
        );
        // Downloaded again, so the record is forgotten.
        changed_sizes(&mut seen, &games, |_| Some(25));
        assert!(seen.announced_sizes.is_empty());
    }

    #[test]
    fn an_unrecorded_or_unmeasured_size_is_not_a_change() {
        let mut seen = Seen::default();
        let games = listed(&[(1, "Hades", 10), (2, "Celeste", 0)]);
        let changes = changed_sizes(&mut seen, &games, |_| Some(0));
        assert_eq!(changes.baselines, [(1, 10)]);
        assert!(changes.changed.is_empty());
    }

    #[test]
    fn the_server_is_named_by_its_host() {
        assert_eq!(
            server_name(Some("https://games.example.org:8080")),
            "games.example.org"
        );
        assert_eq!(server_name(None), "Gameyfin");
    }
}
