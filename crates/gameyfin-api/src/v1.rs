//! Gameyfin 2.6's REST API under `/v1`. Its personal access tokens work only there, so a client
//! signed in with one lists, probes and downloads through these routes instead of Hilla.

use std::collections::HashMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::json;

use crate::catalog::GameCatalog;
use crate::client::GameyfinClient;
use crate::error::{ApiError, ApiResult};
use crate::models::{DownloadProvider, Game, GameMetadata, Image, Library, UserInfo};

/// The most the server returns in one page.
const PAGE_SIZE: usize = 200;

/// Personal access tokens from `CredentialEndpoint.create`, sent as `X-API-Key`.
pub const API_KEY_PREFIX: &str = "gf_pat_";

pub fn is_api_key(token: &str) -> bool {
    token.starts_with(API_KEY_PREFIX)
}

/// The credential id inside `gf_pat_<id>.<secret>`, which revoking it needs.
pub fn api_key_id(token: &str) -> Option<&str> {
    token
        .strip_prefix(API_KEY_PREFIX)?
        .split_once('.')
        .map(|(id, _)| id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageV1<T> {
    items: Vec<T>,
    total_elements: u64,
}

#[derive(Debug, Deserialize)]
struct ImageV1 {
    #[serde(deserialize_with = "numeric_id")]
    id: i64,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ExternalIdV1 {
    provider: String,
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameV1 {
    #[serde(deserialize_with = "numeric_id")]
    id: i64,
    #[serde(deserialize_with = "numeric_id")]
    library_id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    user_rating: Option<i32>,
    #[serde(default)]
    critic_rating: Option<i32>,
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    publishers: Vec<String>,
    #[serde(default)]
    developers: Vec<String>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    perspectives: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    cover: Option<ImageV1>,
    #[serde(default)]
    header: Option<ImageV1>,
    #[serde(default)]
    images: Vec<ImageV1>,
    #[serde(default)]
    video_urls: Vec<String>,
    #[serde(default)]
    external_ids: Vec<ExternalIdV1>,
    #[serde(default)]
    file_size: Option<u64>,
}

/// Kept on the API's route: `/images/**` wants the web session that a key replaces.
fn image(image: ImageV1, kind: &str) -> Image {
    Image {
        id: image.id,
        r#type: kind.to_string(),
        blurhash: None,
        url: Some(image.url),
    }
}

/// A catalogue cached from Hilla has image paths a key cannot fetch, so it is read again whole.
fn from_rest_api(catalog: &GameCatalog) -> bool {
    catalog.games.iter().all(|g| {
        g.cover
            .iter()
            .chain(&g.header)
            .chain(&g.images)
            .all(|i| i.url.is_some())
    })
}

impl From<GameV1> for Game {
    fn from(g: GameV1) -> Self {
        let original_ids: HashMap<String, String> = g
            .external_ids
            .into_iter()
            .map(|e| (e.provider, e.id))
            .collect();
        Game {
            id: g.id,
            title: g.title.unwrap_or_default(),
            library_id: g.library_id,
            summary: g.summary,
            comment: g.comment,
            release: g.release,
            created_at: g.created_at,
            updated_at: g.updated_at,
            user_rating: g.user_rating,
            critic_rating: g.critic_rating,
            platforms: g.platforms,
            genres: g.genres,
            themes: g.themes,
            publishers: g.publishers,
            developers: g.developers,
            features: g.features,
            keywords: g.keywords,
            perspectives: g.perspectives,
            collection_ids: Vec::new(),
            cover: g.cover.map(|i| image(i, "COVER")),
            header: g.header.map(|i| image(i, "HEADER")),
            images: g
                .images
                .into_iter()
                .map(|i| image(i, "SCREENSHOT"))
                .collect(),
            video_urls: g.video_urls,
            metadata: GameMetadata {
                file_size: g.file_size.unwrap_or_default(),
                original_ids: (!original_ids.is_empty()).then_some(original_ids),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct LibraryV1 {
    #[serde(deserialize_with = "numeric_id")]
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct MeV1 {
    #[serde(deserialize_with = "numeric_id")]
    id: i64,
    username: String,
}

#[derive(Debug, Deserialize)]
struct ProviderV1 {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    priority: i32,
}

#[derive(Debug, Deserialize)]
struct InstanceV1 {
    version: String,
}

#[derive(Debug, Deserialize)]
struct CreatedCredential {
    secret: String,
}

/// The API sends ids as strings; the rest of the app keys everything by the numeric id.
fn numeric_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let id = String::deserialize(deserializer)?;
    id.parse().map_err(serde::de::Error::custom)
}

/// Orders the server's `Instant`s, which drop trailing zeros from the fraction.
fn instant_key(instant: &str) -> Option<(&str, u32)> {
    let instant = instant.strip_suffix('Z')?;
    let (seconds, fraction) = instant.split_once('.').unwrap_or((instant, ""));
    if seconds.len() != 19 || fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let nanos = format!("{fraction:0<9}").parse().ok()?;
    Some((seconds, nanos))
}

/// Where the next delta starts: the newest change in the catalogue.
fn newest_update(games: &[Game]) -> Option<String> {
    games
        .iter()
        .filter_map(|g| g.updated_at.as_deref())
        .filter_map(|at| instant_key(at).map(|key| (key, at)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, at)| at.to_string())
}

/// `previous` with `changed` swapped in by id.
fn replaced(previous: &[Game], changed: Vec<Game>) -> Vec<Game> {
    let mut games: HashMap<i64, Game> = previous.iter().map(|g| (g.id, g.clone())).collect();
    games.extend(changed.into_iter().map(|g| (g.id, g)));
    let mut games: Vec<Game> = games.into_values().collect();
    games.sort_by_key(|g| g.id);
    games
}

impl GameyfinClient {
    async fn v1_get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> ApiResult<T> {
        let label = format!("GET {path}");
        let mut url = url::Url::parse(&self.url_for(path))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let req = self
            .http()
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json");
        let resp = self.auth().apply(req).await?.send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        crate::client::interpret(&label, status, &body)
    }

    pub(crate) async fn v1_user_info(&self) -> ApiResult<Option<UserInfo>> {
        let me: MeV1 = self.v1_get("/v1/me", &[]).await?;
        Ok(Some(UserInfo {
            id: Some(me.id),
            username: me.username,
            email: None,
            roles: Vec::new(),
            avatar: None,
        }))
    }

    pub(crate) async fn v1_server_version(&self) -> ApiResult<Option<String>> {
        let instance: InstanceV1 = self.v1_get("/v1/instance", &[]).await?;
        Ok(Some(instance.version))
    }

    pub(crate) async fn v1_download_providers(&self) -> ApiResult<Vec<DownloadProvider>> {
        let providers: Vec<ProviderV1> = self.v1_get("/v1/download-providers", &[]).await?;
        Ok(providers
            .into_iter()
            .map(|p| DownloadProvider {
                key: p.id,
                name: p.name,
                priority: p.priority,
                description: p.description,
                short_description: None,
            })
            .collect())
    }

    pub(crate) async fn v1_libraries(&self) -> ApiResult<Vec<Library>> {
        let libraries: Vec<LibraryV1> = self.v1_pages("/v1/libraries", "id", |_| true).await?.0;
        Ok(libraries
            .into_iter()
            .map(|l| Library {
                id: l.id,
                name: l.name,
                game_ids: Vec::new(),
            })
            .collect())
    }

    /// Pages through `path` while `more` accepts the last page. Also returns the total.
    async fn v1_pages<T: DeserializeOwned>(
        &self,
        path: &str,
        sort: &str,
        mut more: impl FnMut(&[T]) -> bool,
    ) -> ApiResult<(Vec<T>, u64)> {
        let size = PAGE_SIZE.to_string();
        let mut items = Vec::new();
        for page in 0.. {
            let page = page.to_string();
            let found: PageV1<T> = self
                .v1_get(path, &[("page", &page), ("size", &size), ("sort", sort)])
                .await?;
            let last = found.items.len() < PAGE_SIZE
                || items.len() + found.items.len() >= found.total_elements as usize;
            let go_on = more(&found.items);
            items.extend(found.items);
            if last || !go_on {
                return Ok((items, found.total_elements));
            }
        }
        unreachable!("pages run out before the counter does")
    }

    /// Every game in full. With a `previous` catalogue only the games changed since are read,
    /// unless the count shows some were removed, which the API cannot list.
    pub(crate) async fn v1_game_catalog(
        &self,
        previous: Option<&GameCatalog>,
    ) -> ApiResult<GameCatalog> {
        let since = previous.filter(|p| from_rest_api(p)).and_then(|p| {
            let key = instant_key(p.synced_at.as_deref()?)?;
            Some((p, key))
        });
        if let Some((previous, since)) = since {
            // Equal counts as changed: a change in the same instant as the last one is not lost.
            let newer = |g: &GameV1| {
                g.updated_at
                    .as_deref()
                    .and_then(instant_key)
                    .is_none_or(|at| at >= since)
            };
            let (changed, total) = self
                .v1_pages("/v1/games", "updatedAt,desc", |page: &[GameV1]| {
                    page.iter().all(newer)
                })
                .await?;
            let changed: Vec<Game> = changed
                .into_iter()
                .filter(|g| newer(g))
                .map(Game::from)
                .collect();
            let games = replaced(&previous.games, changed);
            if games.len() as u64 == total {
                return Ok(GameCatalog {
                    synced_at: newest_update(&games).or_else(|| previous.synced_at.clone()),
                    games,
                });
            }
            tracing::debug!(
                had = games.len(),
                total,
                "games were removed; listing them all"
            );
        }
        let (games, _) = self.v1_pages::<GameV1>("/v1/games", "id", |_| true).await?;
        let games: Vec<Game> = games.into_iter().map(Game::from).collect();
        Ok(GameCatalog {
            synced_at: newest_update(&games),
            games,
        })
    }

    /// A personal access token for this device, from Gameyfin 2.6 on. Needs a cookie session.
    pub async fn create_api_token(&self, name: &str) -> ApiResult<String> {
        let created: CreatedCredential = self
            .call(
                "CredentialEndpoint",
                "create",
                json!({ "service": false, "input": { "name": name, "expiresAt": null, "scopes": [] } }),
            )
            .await?;
        if !is_api_key(&created.secret) {
            return Err(ApiError::Status {
                endpoint: "CredentialEndpoint.create".into(),
                status: 200,
                body: "the server answered with a token of an unknown kind".into(),
            });
        }
        Ok(created.secret)
    }

    /// Revokes and removes a token this device made. Needs a cookie session: a token cannot.
    pub async fn delete_api_token(&self, token: &str) -> ApiResult<()> {
        let Some(id) = api_key_id(token) else {
            return Ok(());
        };
        let params = json!({ "id": id, "service": false });
        self.call::<Option<serde_json::Value>>("CredentialEndpoint", "revoke", params.clone())
            .await?;
        self.call::<Option<serde_json::Value>>("CredentialEndpoint", "delete", params)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::auth::ApiKeyAuth;

    fn client(server: &mockito::Server) -> GameyfinClient {
        GameyfinClient::new(server.url(), Arc::new(ApiKeyAuth::new("gf_pat_k.s"))).unwrap()
    }

    fn game_v1(id: i64, title: &str, updated_at: &str) -> serde_json::Value {
        json!({
            "id": id.to_string(), "libraryId": "1", "title": title, "updatedAt": updated_at,
            "platforms": [], "publishers": [], "developers": [], "genres": [], "themes": [],
            "features": [], "perspectives": [], "keywords": [], "images": [], "videoUrls": [],
            "mediaOrder": [], "externalIds": []
        })
    }

    fn page(items: Vec<serde_json::Value>, total: u64) -> String {
        json!({ "items": items, "page": 0, "size": 200, "totalElements": total }).to_string()
    }

    #[test]
    fn a_v1_game_maps_onto_the_hilla_shape() {
        let mut v1 = game_v1(7, "Celeste", "2026-10-01T10:00:00Z");
        v1["cover"] = json!({ "id": "11", "url": "/v1/games/7/images/11" });
        v1["images"] = json!([{ "id": "12", "url": "/v1/games/7/images/12" }]);
        v1["externalIds"] =
            json!([{ "provider": "org.gameyfin.plugins.metadata.steam", "id": "504230" }]);
        v1["fileSize"] = json!(42);
        let game: Game = serde_json::from_value::<GameV1>(v1).unwrap().into();

        assert_eq!(
            (game.id, game.library_id, game.title.as_str()),
            (7, 1, "Celeste")
        );
        assert_eq!(game.cover.as_ref().unwrap().path(), "/v1/games/7/images/11");
        assert_eq!(game.images[0].r#type, "SCREENSHOT");
        assert_eq!(game.steam_app_id(), Some(504230));
        assert_eq!(game.metadata.file_size, 42);
    }

    #[test]
    fn instants_order_whatever_their_fraction() {
        let key = |s| instant_key(s).unwrap();
        assert!(key("2026-10-01T10:00:00.5Z") > key("2026-10-01T10:00:00Z"));
        assert!(key("2026-10-01T10:00:00.123Z") < key("2026-10-01T10:00:00.1234Z"));
        assert_eq!(instant_key("2026-10-01T10:00:00+01:00"), None);
    }

    #[test]
    fn the_credential_id_is_read_from_a_token() {
        assert_eq!(api_key_id("gf_pat_abc.secret"), Some("abc"));
        assert_eq!(api_key_id("gyf_abc"), None);
    }

    #[tokio::test]
    async fn the_first_listing_reads_every_page_with_the_key() {
        let mut server = mockito::Server::new_async().await;
        let first: Vec<_> = (1..=200)
            .map(|i| game_v1(i, "G", "2026-10-01T10:00:00Z"))
            .collect();
        let p0 = server
            .mock("GET", "/v1/games?page=0&size=200&sort=id")
            .match_header("x-api-key", "gf_pat_k.s")
            .with_body(page(first, 201))
            .create_async()
            .await;
        let p1 = server
            .mock("GET", "/v1/games?page=1&size=200&sort=id")
            .with_body(page(
                vec![game_v1(201, "Last", "2026-10-02T10:00:00.25Z")],
                201,
            ))
            .create_async()
            .await;

        let catalog = client(&server).game_catalog(None).await.unwrap();

        p0.assert_async().await;
        p1.assert_async().await;
        assert_eq!(catalog.games.len(), 201);
        assert_eq!(
            catalog.synced_at.as_deref(),
            Some("2026-10-02T10:00:00.25Z")
        );
    }

    #[tokio::test]
    async fn a_later_listing_reads_only_what_changed() {
        let mut server = mockito::Server::new_async().await;
        let previous = GameCatalog {
            games: vec![
                Game::from(
                    serde_json::from_value::<GameV1>(game_v1(1, "Celeste", "2026-10-01T10:00:00Z"))
                        .unwrap(),
                ),
                Game::from(
                    serde_json::from_value::<GameV1>(game_v1(2, "Hades", "2026-10-02T10:00:00Z"))
                        .unwrap(),
                ),
            ],
            synced_at: Some("2026-10-02T10:00:00Z".into()),
        };
        let delta = server
            .mock("GET", "/v1/games?page=0&size=200&sort=updatedAt%2Cdesc")
            .with_body(page(
                vec![
                    game_v1(2, "Hades II", "2026-10-03T10:00:00Z"),
                    game_v1(1, "Celeste", "2026-10-01T10:00:00Z"),
                ],
                2,
            ))
            .create_async()
            .await;

        let catalog = client(&server).game_catalog(Some(&previous)).await.unwrap();

        delta.assert_async().await;
        let titles: Vec<&str> = catalog.games.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Celeste", "Hades II"]);
        assert_eq!(catalog.synced_at.as_deref(), Some("2026-10-03T10:00:00Z"));
    }

    #[tokio::test]
    async fn a_removed_game_triggers_a_full_listing() {
        let mut server = mockito::Server::new_async().await;
        let previous = GameCatalog {
            games: vec![
                Game::from(
                    serde_json::from_value::<GameV1>(game_v1(1, "Celeste", "2026-10-01T10:00:00Z"))
                        .unwrap(),
                ),
                Game::from(
                    serde_json::from_value::<GameV1>(game_v1(2, "Gone", "2026-10-01T10:00:00Z"))
                        .unwrap(),
                ),
            ],
            synced_at: Some("2026-10-01T10:00:00Z".into()),
        };
        server
            .mock("GET", "/v1/games?page=0&size=200&sort=updatedAt%2Cdesc")
            .with_body(page(vec![game_v1(1, "Celeste", "2026-10-01T10:00:00Z")], 1))
            .create_async()
            .await;
        let full = server
            .mock("GET", "/v1/games?page=0&size=200&sort=id")
            .with_body(page(vec![game_v1(1, "Celeste", "2026-10-01T10:00:00Z")], 1))
            .create_async()
            .await;

        let catalog = client(&server).game_catalog(Some(&previous)).await.unwrap();

        full.assert_async().await;
        assert_eq!(catalog.games.len(), 1);
    }

    #[tokio::test]
    async fn a_catalogue_cached_from_hilla_is_read_again_whole() {
        let mut server = mockito::Server::new_async().await;
        let hilla: Game = serde_json::from_value(
            json!({ "id": 1, "title": "Celeste", "cover": { "id": 11, "type": "COVER" } }),
        )
        .unwrap();
        let previous = GameCatalog {
            games: vec![hilla],
            synced_at: Some("2026-10-01T10:00:00Z".into()),
        };
        let mut v1 = game_v1(1, "Celeste", "2026-10-01T10:00:00Z");
        v1["cover"] = json!({ "id": "11", "url": "/v1/games/1/images/11" });
        let full = server
            .mock("GET", "/v1/games?page=0&size=200&sort=id")
            .with_body(page(vec![v1], 1))
            .create_async()
            .await;

        let catalog = client(&server).game_catalog(Some(&previous)).await.unwrap();

        full.assert_async().await;
        assert_eq!(
            catalog.games[0].cover.as_ref().unwrap().path(),
            "/v1/games/1/images/11"
        );
    }

    #[tokio::test]
    async fn a_revoked_key_reads_as_signed_out() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/v1/me")
            .with_status(401)
            .with_header("content-type", "application/problem+json")
            .with_body(r#"{"code":"unauthorized"}"#)
            .create_async()
            .await;

        let err = client(&server).user_info().await.unwrap_err();
        assert!(err.is_auth(), "got {err:?}");
    }

    #[tokio::test]
    async fn hilla_is_not_asked_with_a_key() {
        let server = mockito::Server::new_async().await;
        let err = client(&server)
            .set_user_preference("k", "v")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ApiError::Status { status: 404, .. }),
            "got {err:?}"
        );
    }
}
