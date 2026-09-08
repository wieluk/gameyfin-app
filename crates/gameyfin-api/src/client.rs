//! The Gameyfin HTTP client. Two interfaces: **Hilla RPC**
//! (`POST /connect/<Endpoint>/<method>`, nearly the whole API) and **plain REST**
//! (`/images/**`, `/download/{gameId}`, the only byte-serving endpoints).

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::auth::AuthStrategy;
use crate::error::{ApiError, ApiResult};
use crate::models::{DownloadProvider, Game, Image, Library, UserInfo};

/// Hilla's RPC path prefix.
const HILLA_PREFIX: &str = "/connect";

#[derive(Debug, Clone)]
pub struct GameyfinClient {
    base_url: String,
    http: reqwest::Client,
    auth: Arc<dyn AuthStrategy>,
}

impl GameyfinClient {
    pub fn new(base_url: impl Into<String>, auth: Arc<dyn AuthStrategy>) -> ApiResult<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(ApiError::NoServerUrl);
        }
        // Validate eagerly so a typo surfaces at construction rather than at first call.
        let _ = url::Url::parse(&base_url)?;

        let http = reqwest::Client::builder()
            .user_agent(concat!("Gameyfin-Desktop/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            base_url,
            http,
            auth,
        })
    }

    /// Build a client over an existing HTTP client, so callers can share a connection pool.
    pub fn with_http(
        base_url: impl Into<String>,
        auth: Arc<dyn AuthStrategy>,
        http: reqwest::Client,
    ) -> ApiResult<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(ApiError::NoServerUrl);
        }
        let _ = url::Url::parse(&base_url)?;
        Ok(Self {
            base_url,
            http,
            auth,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Shared connection pool, so sibling modules issue REST calls without a second client.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub(crate) fn auth(&self) -> &Arc<dyn AuthStrategy> {
        &self.auth
    }

    /// Absolute URL for a REST path such as `/images/cover/7`.
    pub fn url_for(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    pub fn image_url(&self, image: &Image) -> String {
        self.url_for(&image.path())
    }

    pub fn download_url(&self, game: &Game, provider_key: &str) -> String {
        self.url_for(&game.download_path(provider_key))
    }

    /// Invoke a Hilla endpoint method and decode its result.
    ///
    /// Retries once when the server rejects the call and the auth strategy reports that it
    /// refreshed something, for cookie auth this covers the common stale-CSRF case.
    pub async fn call<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        method: &str,
        params: Value,
    ) -> ApiResult<T> {
        let label = format!("{endpoint}.{method}");
        let url = format!("{}{HILLA_PREFIX}/{endpoint}/{method}", self.base_url);

        for attempt in 0..2u8 {
            let req = self
                .http
                .post(&url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "application/json")
                .json(&params);

            let req = self.auth.apply(req).await?;
            let resp = req.send().await?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            if (status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN)
                && attempt == 0
                && self.auth.on_rejected().await?
            {
                tracing::debug!("{label} rejected with {status}; refreshed auth, retrying");
                continue;
            }

            return interpret(&label, status.as_u16(), &body);
        }

        unreachable!("loop returns on its final iteration")
    }

    /// All games visible to the current user.
    pub async fn games(&self) -> ApiResult<Vec<Game>> {
        self.call("GameEndpoint", "getAll", json!({})).await
    }

    /// All libraries visible to the current user.
    pub async fn libraries(&self) -> ApiResult<Vec<Library>> {
        self.call("LibraryEndpoint", "getAll", json!({})).await
    }

    /// Download providers, highest priority first.
    pub async fn download_providers(&self) -> ApiResult<Vec<DownloadProvider>> {
        let mut providers: Vec<DownloadProvider> = self
            .call("DownloadProviderEndpoint", "getProviders", json!({}))
            .await?;
        providers.sort_by_key(|p| std::cmp::Reverse(p.priority));
        Ok(providers)
    }

    /// Read one of the server-side user preferences, such as `preferred-download-method`.
    ///
    /// Shared with the web UI, so a choice made in either place is the one the other sees.
    pub async fn user_preference(&self, key: &str) -> ApiResult<Option<String>> {
        self.call("UserPreferencesEndpoint", "get", json!({ "key": key }))
            .await
    }

    /// Store one of the server-side user preferences. See [`Self::user_preference`].
    pub async fn set_user_preference(&self, key: &str, value: &str) -> ApiResult<()> {
        // Void Hilla methods answer with an empty body, which decodes as null.
        self.call::<Option<serde_json::Value>>(
            "UserPreferencesEndpoint",
            "set",
            json!({ "key": key, "value": value }),
        )
        .await?;
        Ok(())
    }

    /// The current user, or `None` when the server considers the caller anonymous.
    ///
    /// `getUserInfo` is `@AnonymousAllowed`, so an anonymous caller gets a null result
    /// rather than a rejection, which makes this the natural session probe.
    pub async fn user_info(&self) -> ApiResult<Option<UserInfo>> {
        self.call("UserEndpoint", "getUserInfo", json!({})).await
    }

    /// Read a user preference (`UserPreferencesEndpoint`, `@PermitAll`).
    pub async fn get_preference(&self, key: &str) -> ApiResult<Option<String>> {
        self.call("UserPreferencesEndpoint", "get", json!({ "key": key }))
            .await
    }

    /// Write a user preference.
    pub async fn set_preference(&self, key: &str, value: &str) -> ApiResult<()> {
        let _: Option<Value> = self
            .call(
                "UserPreferencesEndpoint",
                "set",
                json!({ "key": key, "value": value }),
            )
            .await?;
        Ok(())
    }

    /// True when the session is usable.
    pub async fn is_authenticated(&self) -> bool {
        matches!(self.user_info().await, Ok(Some(_)))
    }
}

/// Turn a transport-agnostic (status, body) pair into a decoded result or an error.
fn interpret<T: DeserializeOwned>(label: &str, status: u16, body: &str) -> ApiResult<T> {
    if status == 401 || status == 403 {
        return Err(ApiError::Unauthenticated(format!(
            "{label} rejected with HTTP {status}"
        )));
    }
    if status >= 400 {
        return Err(ApiError::Status {
            endpoint: label.to_string(),
            status,
            body: body.chars().take(200).collect(),
        });
    }

    // A void Hilla method answers 200 with an empty body; `Option`/`()` targets accept null.
    if body.trim().is_empty() {
        return serde_json::from_str("null").map_err(|source| ApiError::Decode {
            endpoint: label.to_string(),
            source,
        });
    }

    serde_json::from_str(body).map_err(|source| {
        // An HTML body here almost always means the login page was served instead of a
        // result, which Gameyfin does for an expired session on some configurations.
        if body
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("<!doctype")
            || body.trim_start().to_ascii_lowercase().starts_with("<html")
        {
            ApiError::Unauthenticated(format!("{label} returned a login page"))
        } else {
            ApiError::Decode {
                endpoint: label.to_string(),
                source,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Game;

    #[test]
    fn rejects_empty_base_url() {
        let auth = Arc::new(crate::auth::DeviceTokenAuth::new("t"));
        assert!(matches!(
            GameyfinClient::new("", auth),
            Err(ApiError::NoServerUrl)
        ));
    }

    #[test]
    fn strips_trailing_slash() {
        let auth = Arc::new(crate::auth::DeviceTokenAuth::new("t"));
        let c = GameyfinClient::new("http://host:8080/", auth).unwrap();
        assert_eq!(c.base_url(), "http://host:8080");
        assert_eq!(
            c.url_for("/images/cover/1"),
            "http://host:8080/images/cover/1"
        );
    }

    #[test]
    fn interpret_maps_401_to_auth_error() {
        let err = interpret::<Value>("X.y", 401, "nope").unwrap_err();
        assert!(err.is_auth());
    }

    #[test]
    fn interpret_maps_login_page_to_auth_error() {
        let err =
            interpret::<Vec<Game>>("X.y", 200, "<!DOCTYPE html><html>login</html>").unwrap_err();
        assert!(err.is_auth(), "got {err:?}");
    }

    #[test]
    fn interpret_maps_500_to_status_error() {
        let err = interpret::<Value>("X.y", 500, "boom").unwrap_err();
        assert!(matches!(err, ApiError::Status { status: 500, .. }));
    }

    #[test]
    fn interpret_accepts_empty_body_for_void_methods() {
        let out: Option<Value> = interpret("X.y", 200, "").unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn interpret_decodes_games() {
        let body = r#"[{"id":1,"title":"Celeste"}]"#;
        let games: Vec<Game> = interpret("GameEndpoint.getAll", 200, body).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title, "Celeste");
    }
}
