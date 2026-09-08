//! Shared application state.
//!
//! The client is behind a lock because the server URL and credentials can change at
//! runtime (the user connects, signs out, or switches instance) without restarting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gameyfin_api::{CookieSessionAuth, Game, GameyfinClient, Library};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Where the last known-good catalogue is written.
pub const CATALOG_FILE: &str = "catalog.json";

/// The catalogue as the server last described it.
///
/// Persisted so the library still renders when the server cannot be reached: the games
/// installed on this machine are on this machine whether or not there is a network, and
/// an app that shows an empty screen because a router is down is broken.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CachedCatalog {
    pub games: Vec<Game>,
    pub libraries: Vec<Library>,
}

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
    /// The last catalogue the server successfully returned, mirrored to disk.
    ///
    /// Separate from `catalog` because that one is a freshness cache with a TTL, and this
    /// one deliberately has no expiry: stale titles are better than no library at all.
    offline_catalog: RwLock<CachedCatalog>,
    /// Whether the last attempt to reach the server failed at the transport level.
    ///
    /// Starts false, the optimistic reading, so nothing claims the server is down before
    /// anything has tried to talk to it.
    unreachable: AtomicBool,
    image_cache: RwLock<Option<std::sync::Arc<crate::image_cache::ImageCache>>>,
    /// The umu fix database, so a launch resolves its game id without a network round
    /// trip. Empty until it has been fetched once, which resolves to the generic id.
    umu: RwLock<gameyfin_core::umu::Database>,
    /// Live controller state, shared with the polling thread.
    gamepad: RwLock<Option<crate::gamepad::Handle>>,
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
        // Timeouts matter here rather than being belt and braces. Without a connect
        // timeout, every RPC made while the network is down blocks for however long the
        // OS decides to wait, which the user sees as the app hanging on its splash rather
        // than as an offline library.
        let http = reqwest::Client::builder()
            .user_agent(concat!("Gameyfin-Desktop/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(8))
            .timeout(std::time::Duration::from_secs(30))
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

    /// Forget the cached catalogue, in memory and on disk.
    ///
    /// Called when the session or the server changes. The offline mirror exists so a
    /// signed-in user keeps their own library when the network goes; it must not outlive
    /// the account it came from, or a sign-out would leave one user's titles readable to
    /// whoever signs in next.
    pub async fn forget_catalog(&self) {
        *self.catalog.write().await = None;
        *self.offline_catalog.write().await = CachedCatalog::default();

        let path = self.config_dir().await.join(CATALOG_FILE);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => tracing::info!("cleared the cached catalogue"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("could not clear the cached catalogue at {path:?}: {e}"),
        }
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

    /// The game catalogue: fresh from cache, else the server, else what was last seen.
    ///
    /// The final fallback is the point. A user whose server is unreachable still has
    /// games installed on this machine, and they must still be able to see and launch
    /// them, so an unreachable server degrades the catalogue to a stale one rather than
    /// emptying the library.
    pub async fn games(&self) -> gameyfin_api::ApiResult<Vec<gameyfin_api::Game>> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(30);

        if let Some((fetched, games)) = self.catalog.read().await.as_ref() {
            if fetched.elapsed() < TTL {
                return Ok(games.clone());
            }
        }

        let Some(client) = self.client().await else {
            return self
                .cached_games()
                .await
                .ok_or_else(|| gameyfin_api::ApiError::Other("not connected".into()));
        };

        match client.games().await {
            Ok(games) => {
                self.set_reachable(true);
                *self.catalog.write().await = Some((std::time::Instant::now(), games.clone()));
                self.remember_games(games.clone()).await;
                Ok(games)
            }
            Err(e) if e.is_unreachable() => {
                self.set_reachable(false);
                // The stale in-memory copy first: it is the same data, without a read.
                let stale = match self.catalog.read().await.as_ref() {
                    Some((_, games)) => Some(games.clone()),
                    None => self.cached_games().await,
                };
                match stale {
                    Some(games) => {
                        tracing::debug!(
                            games = games.len(),
                            "server unreachable; serving the last known catalogue"
                        );
                        Ok(games)
                    }
                    None => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// The libraries, with the same fall back to what was last seen. See [`Self::games`].
    pub async fn libraries(&self) -> gameyfin_api::ApiResult<Vec<Library>> {
        let Some(client) = self.client().await else {
            return Ok(self.offline_catalog.read().await.libraries.clone());
        };

        match client.libraries().await {
            Ok(libraries) => {
                self.set_reachable(true);
                self.remember_libraries(libraries.clone()).await;
                Ok(libraries)
            }
            Err(e) if e.is_unreachable() => {
                self.set_reachable(false);
                Ok(self.offline_catalog.read().await.libraries.clone())
            }
            Err(e) => Err(e),
        }
    }

    /// The last catalogue seen, when there is one worth showing.
    async fn cached_games(&self) -> Option<Vec<Game>> {
        let games = self.offline_catalog.read().await.games.clone();
        (!games.is_empty()).then_some(games)
    }

    /// True when a library can be rendered without reaching the server.
    pub async fn has_cached_catalog(&self) -> bool {
        !self.offline_catalog.read().await.games.is_empty()
    }

    async fn remember_games(&self, games: Vec<Game>) {
        let changed = {
            let mut cached = self.offline_catalog.write().await;
            let changed = cached.games != games;
            cached.games = games;
            changed
        };
        if changed {
            self.persist_catalog().await;
        }
    }

    async fn remember_libraries(&self, libraries: Vec<Library>) {
        let changed = {
            let mut cached = self.offline_catalog.write().await;
            let changed = cached.libraries != libraries;
            cached.libraries = libraries;
            changed
        };
        if changed {
            self.persist_catalog().await;
        }
    }

    /// Mirror the catalogue to disk. Best effort: it is a cache, not a source of truth.
    async fn persist_catalog(&self) {
        let dir = self.config_dir().await;
        if dir.as_os_str().is_empty() {
            return;
        }
        let cached = self.offline_catalog.read().await.clone();

        if let Err(e) = async {
            tokio::fs::create_dir_all(&dir).await?;
            let json = serde_json::to_vec(&cached)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            tokio::fs::write(dir.join(CATALOG_FILE), json).await
        }
        .await
        {
            tracing::warn!("could not cache the catalogue: {e}");
        }
    }

    /// Read the mirrored catalogue back at startup.
    async fn load_catalog(&self, config_dir: &std::path::Path) {
        let path = config_dir.join(CATALOG_FILE);
        let Ok(bytes) = tokio::fs::read(&path).await else {
            return;
        };
        match serde_json::from_slice::<CachedCatalog>(&bytes) {
            Ok(cached) => {
                tracing::info!(games = cached.games.len(), "loaded the cached catalogue");
                *self.offline_catalog.write().await = cached;
            }
            // A shape change between versions must not stop the app starting; the next
            // successful fetch rewrites the file.
            Err(e) => tracing::warn!("ignoring an unreadable cached catalogue at {path:?}: {e}"),
        }
    }

    /// Record whether the server answered. See [`Self::is_unreachable`].
    pub fn set_reachable(&self, reachable: bool) {
        let was_unreachable = self.unreachable.swap(!reachable, Ordering::Relaxed);
        if was_unreachable == reachable {
            tracing::info!(reachable, "the server's reachability changed");
        }
    }

    /// True when the last attempt to reach the server did not get an answer.
    pub fn is_unreachable(&self) -> bool {
        self.unreachable.load(Ordering::Relaxed)
    }

    /// Drop the cached catalogue, so the next read comes from the server.
    ///
    /// Only the freshness cache: the offline mirror is deliberately kept, since the point
    /// of it is to survive exactly the case where the refetch fails.
    pub async fn invalidate_catalog(&self) {
        *self.catalog.write().await = None;
    }

    /// How many rows the umu database holds.
    pub async fn umu_entry_count(&self) -> usize {
        self.umu.read().await.len()
    }

    /// Load the cached umu database.
    ///
    /// Returns true when the copy on disk is stale or absent, so the caller can start a
    /// refresh. Whatever was cached is used immediately either way: a game launched
    /// thirty seconds after startup gets yesterday's fixes rather than waiting for
    /// today's, and the refresh lands before the one after it.
    pub async fn load_umu_database(&self, config_dir: &std::path::Path) -> bool {
        match gameyfin_core::umu::load_cache(config_dir) {
            Some(cached) => {
                tracing::info!(
                    entries = cached.database.len(),
                    "loaded the cached umu database"
                );
                let stale = cached.stale;
                *self.umu.write().await = cached.database;
                stale
            }
            None => true,
        }
    }

    /// Fetch the umu database and cache it. Returns how many rows it holds.
    pub async fn refresh_umu_database(&self) -> gameyfin_core::CoreResult<usize> {
        let http = self.http().await;
        let database =
            gameyfin_core::umu::fetch(&http, gameyfin_core::umu::DEFAULT_API_URL).await?;
        let count = database.len();

        let dir = self.config_dir().await;
        let to_save = database.clone();
        // Writing is blocking, and the caller may be a command the UI is waiting on.
        tokio::task::spawn_blocking(move || {
            if let Err(e) = gameyfin_core::umu::save_cache(&dir, &to_save) {
                tracing::warn!("could not cache the umu database: {e}");
            }
        });

        *self.umu.write().await = database;
        Ok(count)
    }

    /// The id to pass to umu for a game, honouring the user's setting.
    pub async fn umu_id_for(&self, title: &str, steam_app_id: Option<u32>) -> String {
        if !self.settings().await.umu_fixes {
            return gameyfin_core::umu::DEFAULT_ID.to_string();
        }
        self.umu.read().await.resolve(title, steam_app_id)
    }

    /// Register the controller handle, so settings changes can reach the poll thread.
    pub async fn set_gamepad(&self, handle: crate::gamepad::Handle) {
        *self.gamepad.write().await = Some(handle);
    }

    /// Apply the current gamepad settings to the running poll thread.
    pub async fn apply_gamepad_settings(&self) {
        let settings = self.settings().await;
        if let Some(handle) = self.gamepad.read().await.as_ref() {
            handle.set_enabled(settings.gamepad_enabled);
            handle.set_deadzone(settings.gamepad_deadzone);
        }
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
        self.load_catalog(&config_dir).await;
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

        // An unreachable server is not a rejected session. Reporting failure here would
        // send an offline user to the sign-in wizard, which is the one thing they cannot
        // complete without a network.
        match self.client().await {
            Some(client) => match client.user_info().await {
                Ok(user) => {
                    self.set_reachable(true);
                    user.is_some()
                }
                Err(e) if e.is_auth() => {
                    self.set_reachable(true);
                    false
                }
                Err(e) => {
                    tracing::info!("could not confirm the stored session: {e}");
                    self.set_reachable(false);
                    true
                }
            },
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-state-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn a_game(id: i64, title: &str) -> Game {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "title": title,
            "libraryId": 1,
            "platforms": [],
            "genres": [],
            "developers": [],
            "publishers": [],
            "metadata": { "fileSize": 0 },
        }))
        .expect("the fixture matches the model")
    }

    #[tokio::test]
    async fn with_no_client_the_catalogue_comes_from_the_cache() {
        // The offline case that matters: no client, but games are still listable.
        let state = AppState::default();
        assert!(!state.has_cached_catalog().await);
        assert!(state.games().await.is_err());

        state.remember_games(vec![a_game(1, "Celeste")]).await;

        assert!(state.has_cached_catalog().await);
        let games = state.games().await.expect("the cached catalogue is served");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title, "Celeste");
    }

    #[tokio::test]
    async fn libraries_fall_back_to_the_cache_too() {
        // Without this the library filter empties itself the moment the server is down.
        let state = AppState::default();
        assert!(state.libraries().await.unwrap().is_empty());

        state
            .remember_libraries(vec![Library {
                id: 3,
                name: "Shelf".into(),
                game_ids: vec![1],
            }])
            .await;

        let libraries = state.libraries().await.unwrap();
        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].name, "Shelf");
    }

    #[tokio::test]
    async fn the_catalogue_survives_a_restart() {
        let dir = scratch("catalog");
        let state = AppState::default();
        state.set_config_dir(dir.clone()).await;
        state.remember_games(vec![a_game(4, "Hades")]).await;
        state
            .remember_libraries(vec![Library {
                id: 1,
                name: "Main".into(),
                game_ids: vec![4],
            }])
            .await;

        let reloaded = AppState::default();
        reloaded.load_catalog(&dir).await;
        assert_eq!(reloaded.games().await.unwrap()[0].title, "Hades");
        assert_eq!(reloaded.libraries().await.unwrap()[0].name, "Main");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_corrupt_cache_is_ignored_rather_than_fatal() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CATALOG_FILE), b"{ not json").unwrap();

        let state = AppState::default();
        state.load_catalog(&dir).await;
        assert!(!state.has_cached_catalog().await);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn signing_out_takes_the_cached_catalogue_with_it() {
        // Otherwise one account's titles would still be listed to whoever signs in next.
        let dir = scratch("forget");
        let state = AppState::default();
        state.set_config_dir(dir.clone()).await;
        state.remember_games(vec![a_game(2, "Tunic")]).await;
        assert!(dir.join(CATALOG_FILE).exists());

        state.forget_catalog().await;

        assert!(!state.has_cached_catalog().await);
        assert!(!dir.join(CATALOG_FILE).exists());
        assert!(state.games().await.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reachability_starts_optimistic_and_tracks_the_last_answer() {
        let state = AppState::default();
        assert!(!state.is_unreachable());
        state.set_reachable(false);
        assert!(state.is_unreachable());
        state.set_reachable(true);
        assert!(!state.is_unreachable());
    }
}
