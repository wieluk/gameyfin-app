//! Listing games and libraries. Gameyfin 2.5.1 replaced `getAll` with a delta `sync` of summaries
//! plus `getByIds` for the full games; older servers only have `getAll`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::client::GameyfinClient;
use crate::error::{ApiError, ApiResult};
use crate::models::{Game, Library};

/// Full games are asked for in batches, so one request never carries thousands of ids.
const DETAILS_BATCH: usize = 500;

/// Which way the server lists, learned on the first call. A server never loses `sync`, and one
/// gaining it is a restart the app notices by reconnecting.
#[derive(Debug, Default)]
pub(crate) struct Listing(AtomicU8);

const UNKNOWN: u8 = 0;
const SYNC: u8 = 1;
const GET_ALL: u8 = 2;

impl Listing {
    fn get(&self) -> u8 {
        self.0.load(Ordering::Relaxed)
    }

    fn set(&self, how: u8) {
        self.0.store(how, Ordering::Relaxed);
    }
}

/// The whole catalogue with full games, and where the next delta sync starts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameCatalog {
    pub games: Vec<Game>,
    /// The server's `serverTime` from the sync that produced this, passed back as `since`. None
    /// from `getAll`, which has no deltas.
    pub synced_at: Option<String>,
}

/// A `GameSyncDto` or `LibrarySyncDto`. Only ids are read from game summaries: the full games
/// come from `getByIds`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sync<T> {
    server_time: String,
    full: bool,
    #[serde(alias = "games", alias = "libraries")]
    items: Vec<T>,
    #[serde(default)]
    all_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
struct Summary {
    id: i64,
}

/// A server without the method. Hilla refuses an unknown method as it does an unknown endpoint.
fn lacks_method(error: &ApiError) -> bool {
    matches!(error, ApiError::Unauthenticated(_))
        || matches!(error, ApiError::Status { status: 404, .. })
}

/// `previous` without the games the server no longer has or has changed, then the changed ones.
fn merged(previous: &[Game], changed: Vec<Game>, all_ids: &[i64]) -> Vec<Game> {
    let existing: HashSet<i64> = all_ids.iter().copied().collect();
    let replaced: HashSet<i64> = changed.iter().map(|g| g.id).collect();
    let mut games: Vec<Game> = previous
        .iter()
        .filter(|g| existing.contains(&g.id) && !replaced.contains(&g.id))
        .cloned()
        .collect();
    games.extend(changed);
    games.sort_by_key(|g| g.id);
    games
}

impl GameyfinClient {
    /// Every game in full. On a server with `sync`, only what changed since `previous` is fetched.
    pub async fn game_catalog(&self, previous: Option<&GameCatalog>) -> ApiResult<GameCatalog> {
        if self.rest_only() {
            return self.v1_game_catalog(previous).await;
        }
        if self.listing().get() != GET_ALL {
            let since = previous.and_then(|p| p.synced_at.as_deref());
            match self
                .call::<Sync<Summary>>("GameEndpoint", "sync", json!({ "since": since }))
                .await
            {
                Ok(sync) => {
                    self.listing().set(SYNC);
                    return self.apply_sync(previous, sync).await;
                }
                Err(e) if self.listing().get() == UNKNOWN && lacks_method(&e) => {
                    tracing::info!("GameEndpoint.sync is not there ({e}); listing with getAll");
                }
                Err(e) => return Err(e),
            }
        }
        let games = self.call("GameEndpoint", "getAll", json!({})).await?;
        self.listing().set(GET_ALL);
        Ok(GameCatalog {
            games,
            synced_at: None,
        })
    }

    async fn apply_sync(
        &self,
        previous: Option<&GameCatalog>,
        sync: Sync<Summary>,
    ) -> ApiResult<GameCatalog> {
        let ids: Vec<i64> = sync.items.iter().map(|s| s.id).collect();
        let changed = self.games_by_ids(&ids).await?;
        let games = match previous {
            Some(previous) if !sync.full => merged(&previous.games, changed, &sync.all_ids),
            _ => merged(&[], changed, &ids),
        };
        Ok(GameCatalog {
            games,
            synced_at: Some(sync.server_time),
        })
    }

    async fn games_by_ids(&self, ids: &[i64]) -> ApiResult<Vec<Game>> {
        let mut games = Vec::with_capacity(ids.len());
        for batch in ids.chunks(DETAILS_BATCH) {
            let found: Vec<Game> = self
                .call("GameEndpoint", "getByIds", json!({ "ids": batch }))
                .await?;
            games.extend(found);
        }
        Ok(games)
    }

    /// Libraries are few, so they are listed whole every time.
    pub async fn libraries(&self) -> ApiResult<Vec<Library>> {
        if self.rest_only() {
            return self.v1_libraries().await;
        }
        if self.listing().get() != GET_ALL {
            match self
                .call::<Sync<Library>>("LibraryEndpoint", "sync", json!({ "since": null }))
                .await
            {
                Ok(sync) => {
                    self.listing().set(SYNC);
                    return Ok(sync.items);
                }
                Err(e) if self.listing().get() == UNKNOWN && lacks_method(&e) => {
                    tracing::info!("LibraryEndpoint.sync is not there ({e}); listing with getAll");
                }
                Err(e) => return Err(e),
            }
        }
        let libraries = self.call("LibraryEndpoint", "getAll", json!({})).await?;
        self.listing().set(GET_ALL);
        Ok(libraries)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::auth::DeviceTokenAuth;

    fn game(id: i64, title: &str) -> Game {
        serde_json::from_value(json!({ "id": id, "title": title })).unwrap()
    }

    fn client(server: &mockito::Server) -> GameyfinClient {
        GameyfinClient::new(server.url(), Arc::new(DeviceTokenAuth::new("t"))).unwrap()
    }

    #[test]
    fn a_delta_drops_deleted_games_and_replaces_changed_ones() {
        let previous = [game(1, "Celeste"), game(2, "Hades"), game(3, "Old")];
        let games = merged(
            &previous,
            vec![game(2, "Hades II"), game(4, "New")],
            &[1, 2, 4],
        );
        let titles: Vec<&str> = games.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Celeste", "Hades II", "New"]);
    }

    #[tokio::test]
    async fn a_first_sync_fetches_every_game_in_full() {
        let mut server = mockito::Server::new_async().await;
        let sync = server
            .mock("POST", "/connect/GameEndpoint/sync")
            .match_body(mockito::Matcher::Json(json!({ "since": null })))
            .with_body(
                r#"{"serverTime":"2026-10-22T10:00:00Z","full":true,
                    "games":[{"id":1,"title":"Celeste"},{"id":2,"title":"Hades"}],"allIds":[]}"#,
            )
            .create_async()
            .await;
        let details = server
            .mock("POST", "/connect/GameEndpoint/getByIds")
            .match_body(mockito::Matcher::Json(json!({ "ids": [1, 2] })))
            .with_body(r#"[{"id":1,"title":"Celeste","summary":"Climb"},{"id":2,"title":"Hades"}]"#)
            .create_async()
            .await;

        let catalog = client(&server).game_catalog(None).await.unwrap();
        sync.assert_async().await;
        details.assert_async().await;
        assert_eq!(catalog.games.len(), 2);
        assert_eq!(catalog.games[0].summary.as_deref(), Some("Climb"));
        assert_eq!(catalog.synced_at.as_deref(), Some("2026-10-22T10:00:00Z"));
    }

    #[tokio::test]
    async fn a_later_sync_asks_only_for_what_changed() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/connect/GameEndpoint/sync")
            .match_body(mockito::Matcher::Json(
                json!({ "since": "2026-10-22T10:00:00Z" }),
            ))
            .with_body(
                r#"{"serverTime":"2026-10-22T10:05:00Z","full":false,
                    "games":[{"id":2,"title":"Hades II"}],"allIds":[2]}"#,
            )
            .create_async()
            .await;
        server
            .mock("POST", "/connect/GameEndpoint/getByIds")
            .match_body(mockito::Matcher::Json(json!({ "ids": [2] })))
            .with_body(r#"[{"id":2,"title":"Hades II"}]"#)
            .create_async()
            .await;

        let previous = GameCatalog {
            games: vec![game(1, "Celeste"), game(2, "Hades")],
            synced_at: Some("2026-10-22T10:00:00Z".into()),
        };
        let catalog = client(&server).game_catalog(Some(&previous)).await.unwrap();
        let titles: Vec<&str> = catalog.games.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Hades II"]);
        assert_eq!(catalog.synced_at.as_deref(), Some("2026-10-22T10:05:00Z"));
    }

    #[tokio::test]
    async fn a_server_without_sync_is_listed_with_get_all_from_then_on() {
        let mut server = mockito::Server::new_async().await;
        let sync = server
            .mock("POST", "/connect/GameEndpoint/sync")
            .with_status(403)
            .expect(1)
            .create_async()
            .await;
        let all = server
            .mock("POST", "/connect/GameEndpoint/getAll")
            .with_body(r#"[{"id":1,"title":"Celeste"}]"#)
            .expect(2)
            .create_async()
            .await;
        server
            .mock("POST", "/connect/LibraryEndpoint/getAll")
            .with_body(r#"[{"id":1,"name":"PC","gameIds":[1]}]"#)
            .create_async()
            .await;

        let client = client(&server);
        let first = client.game_catalog(None).await.unwrap();
        assert_eq!(first.games[0].title, "Celeste");
        assert_eq!(first.synced_at, None);
        client.game_catalog(Some(&first)).await.unwrap();
        // Libraries skip the probe too, now the server's kind is known.
        assert_eq!(client.libraries().await.unwrap()[0].name, "PC");
        sync.assert_async().await;
        all.assert_async().await;
    }

    #[tokio::test]
    async fn a_refusal_after_sync_worked_is_an_error_not_a_fallback() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/connect/LibraryEndpoint/sync")
            .with_body(r#"{"serverTime":"t","full":true,"libraries":[{"id":1,"name":"PC"}]}"#)
            .create_async()
            .await;
        server
            .mock("POST", "/connect/GameEndpoint/sync")
            .with_status(403)
            .create_async()
            .await;
        let all = server
            .mock("POST", "/connect/GameEndpoint/getAll")
            .expect(0)
            .create_async()
            .await;

        let client = client(&server);
        assert_eq!(client.libraries().await.unwrap()[0].name, "PC");
        // A signed-out session, not an old server: the caller has to see it.
        assert!(client.game_catalog(None).await.unwrap_err().is_auth());
        all.assert_async().await;
    }
}
