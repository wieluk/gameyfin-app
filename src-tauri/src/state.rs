//! Shared application state.
//!
//! The client is behind a lock because the server URL and credentials can change at
//! runtime (the user connects, signs out, or switches instance) without restarting.

use std::sync::Arc;

use gameyfin_api::{CookieSessionAuth, GameyfinClient};
use tokio::sync::RwLock;

#[derive(Default)]
pub struct AppState {
    client: RwLock<Option<GameyfinClient>>,
    /// Kept alongside the client so cookies harvested from the login webview can be
    /// pushed in after construction.
    cookie_auth: RwLock<Option<Arc<CookieSessionAuth>>>,
    settings: RwLock<crate::settings::Settings>,
    config_dir: RwLock<std::path::PathBuf>,
    /// Shared connection pool, also used for unauthenticated probes.
    http: RwLock<Option<reqwest::Client>>,
    /// Separate pool for downloads, which must not carry a total-request timeout.
    transfer_http: RwLock<Option<reqwest::Client>>,
    library: crate::library_state::SharedLibraryState,
    /// Cached game catalogue.
    ///
    /// `list_entries` is called on every local state change, several times a second
    /// during a transfer, and fetching the whole library from the server each time made
    /// the UI lag far behind the work it was reporting on.
    catalog: RwLock<Option<(std::time::Instant, Vec<gameyfin_api::Game>)>>,
    image_cache: RwLock<Option<std::sync::Arc<crate::image_cache::ImageCache>>>,
    /// The download speed cap, shared with every transfer in flight.
    ///
    /// Held here rather than read per download so changing it takes effect immediately:
    /// a speed control that only applies to the *next* download is not what the word
    /// means when someone drags it during a large transfer.
    download_limit: gameyfin_core::download::RateLimit,
    /// Stop signals for downloads currently running, keyed by game.
    cancels: Cancels,
}

/// The registry of stop signals for running downloads.
///
/// Cloneable and independent of the command's borrow of [`AppState`], because the task
/// that has to deregister a finished download outlives the command that started it.
#[derive(Clone, Default)]
pub struct Cancels(Arc<RwLock<std::collections::HashMap<i64, gameyfin_core::download::Cancel>>>);

impl Cancels {
    /// Create a stop signal for a download about to start.
    pub async fn register(&self, game_id: i64) -> gameyfin_core::download::Cancel {
        let cancel = gameyfin_core::download::Cancel::new();
        self.0.write().await.insert(game_id, cancel.clone());
        cancel
    }

    /// Forget a download that has finished, however it ended.
    pub async fn finish(&self, game_id: i64) {
        self.0.write().await.remove(&game_id);
    }

    /// Ask a running download to stop. False when there was nothing to stop.
    pub async fn cancel(&self, game_id: i64) -> bool {
        match self.0.read().await.get(&game_id) {
            Some(cancel) => {
                cancel.cancel();
                true
            }
            None => false,
        }
    }
}

impl AppState {
    /// Point the app at a server using a harvested browser session.
    ///
    /// This is the path that works against an unmodified 2.4.0 server.
    pub async fn connect_with_cookies(
        &self,
        base_url: &str,
        cookies: std::collections::HashMap<String, String>,
    ) -> gameyfin_api::ApiResult<()> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Gameyfin-Desktop/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let auth = Arc::new(CookieSessionAuth::new(base_url, http.clone()));
        auth.set_cookies(cookies).await;

        let client = GameyfinClient::with_http(base_url, auth.clone(), http)?;
        *self.client.write().await = Some(client);
        *self.cookie_auth.write().await = Some(auth);
        Ok(())
    }

    /// Drop the active session, used when signing out.
    pub async fn disconnect(&self) {
        *self.client.write().await = None;
        *self.cookie_auth.write().await = None;
    }

    /// The active client, or `None` when no server is configured yet.
    pub async fn client(&self) -> Option<GameyfinClient> {
        self.client.read().await.clone()
    }

    /// A shared HTTP client for short requests: probes, metadata, artwork.
    ///
    /// The timeout here covers the *whole* request, body included, so this client must
    /// never be used for a game download, a transfer longer than the timeout is aborted
    /// mid-stream and surfaces as a decode failure rather than anything recognisable.
    /// [`Self::transfer_http`] exists for that.
    pub async fn http(&self) -> reqwest::Client {
        if let Some(existing) = self.http.read().await.clone() {
            return existing;
        }
        let created = reqwest::Client::builder()
            .user_agent(concat!("Gameyfin-Desktop/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        *self.http.write().await = Some(created.clone());
        created
    }

    /// A client for long transfers.
    ///
    /// No overall timeout, a game can legitimately take hours, but a connect timeout so
    /// an unreachable server still fails promptly, and a read timeout so a genuinely
    /// stalled connection does not hang forever.
    pub async fn transfer_http(&self) -> reqwest::Client {
        if let Some(existing) = self.transfer_http.read().await.clone() {
            return existing;
        }
        let created = reqwest::Client::builder()
            .user_agent(concat!("Gameyfin-Desktop/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        *self.transfer_http.write().await = Some(created.clone());
        created
    }

    /// The game catalogue, from cache when it is fresh.
    pub async fn games(&self) -> gameyfin_api::ApiResult<Vec<gameyfin_api::Game>> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(30);

        if let Some((fetched, games)) = self.catalog.read().await.as_ref() {
            if fetched.elapsed() < TTL {
                return Ok(games.clone());
            }
        }

        let client = self
            .client()
            .await
            .ok_or_else(|| gameyfin_api::ApiError::Other("not connected".into()))?;
        let games = client.games().await?;
        *self.catalog.write().await = Some((std::time::Instant::now(), games.clone()));
        Ok(games)
    }

    /// Drop the cached catalogue, so the next read comes from the server.
    pub async fn invalidate_catalog(&self) {
        *self.catalog.write().await = None;
    }

    /// The on-disk artwork cache, created on first use.
    pub async fn image_cache(&self) -> std::sync::Arc<crate::image_cache::ImageCache> {
        if let Some(existing) = self.image_cache.read().await.clone() {
            return existing;
        }
        let dir = self.config_dir().await.join("image-cache");
        let cache = std::sync::Arc::new(crate::image_cache::ImageCache::new(dir));
        *self.image_cache.write().await = Some(cache.clone());
        cache
    }

    pub fn library(&self) -> &crate::library_state::LibraryState {
        &self.library
    }

    /// A handle that can outlive this borrow, for spawned tasks.
    pub fn library_handle(&self) -> crate::library_state::SharedLibraryState {
        self.library.clone()
    }

    pub async fn set_config_dir(&self, dir: std::path::PathBuf) {
        *self.config_dir.write().await = dir;
    }

    /// The shared download speed cap.
    pub fn download_limit(&self) -> gameyfin_core::download::RateLimit {
        self.download_limit.clone()
    }

    /// Register a stop signal for a download about to start.
    pub async fn register_download(&self, game_id: i64) -> gameyfin_core::download::Cancel {
        self.cancels.register(game_id).await
    }

    /// A handle to the registry that can be moved into the transfer task.
    pub fn cancel_handle(&self) -> Cancels {
        self.cancels.clone()
    }

    /// Ask a running download to stop. False when there was nothing to stop.
    pub async fn cancel_download(&self, game_id: i64) -> bool {
        self.cancels.cancel(game_id).await
    }

    pub async fn config_dir(&self) -> std::path::PathBuf {
        self.config_dir.read().await.clone()
    }

    pub async fn settings(&self) -> crate::settings::Settings {
        self.settings.read().await.clone()
    }

    /// Replace the settings and write them out.
    pub async fn update_settings(
        &self,
        settings: crate::settings::Settings,
    ) -> std::io::Result<()> {
        let dir = self.config_dir().await;
        settings.save(&dir).await?;
        *self.settings.write().await = settings;
        Ok(())
    }

    /// Load settings from disk and, if a session was stored, reconnect with it.
    ///
    /// A stored session may have expired server-side, so the caller is told whether the
    /// connection actually authenticated rather than assuming it did.
    pub async fn restore(&self, config_dir: std::path::PathBuf) -> bool {
        self.set_config_dir(config_dir.clone()).await;
        self.library.load(config_dir.clone()).await;
        let settings = crate::settings::Settings::load(&config_dir).await;
        *self.settings.write().await = settings.clone();

        let (Some(url), true) = (settings.server_url.clone(), settings.has_session()) else {
            return false;
        };

        if self
            .connect_with_cookies(&url, settings.cookies.clone())
            .await
            .is_err()
        {
            return false;
        }

        match self.client().await {
            Some(client) => client.is_authenticated().await,
            None => false,
        }
    }
}
