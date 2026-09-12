//! Shared application state. Locks are std, never held across an await, so progress
//! callbacks and drop guards can update state synchronously.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use gameyfin_api::{CookieSessionAuth, Game, GameyfinClient, Library};
use serde::{Deserialize, Serialize};

use crate::error::{CommandError, CommandResult};
use crate::settings::Settings;

pub const CATALOG_FILE: &str = "catalog.json";
const SETTINGS_FILE: &str = "settings.json";
const USER_AGENT: &str = concat!("Gameyfin-Desktop/", env!("CARGO_PKG_VERSION"));

/// The last catalogue the server returned, so the library renders offline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CachedCatalog {
    pub games: Vec<Game>,
    pub libraries: Vec<Library>,
}

/// A one-way flag that tasks can wait on, for "startup has read the settings file".
#[derive(Debug)]
struct ReadyFlag {
    tx: tokio::sync::watch::Sender<bool>,
    rx: tokio::sync::watch::Receiver<bool>,
}

impl Default for ReadyFlag {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::watch::channel(false);
        Self { tx, rx }
    }
}

impl ReadyFlag {
    fn set(&self) {
        let _ = self.tx.send(true);
    }

    fn get(&self) -> bool {
        *self.rx.borrow()
    }

    async fn wait(&self) {
        let mut rx = self.rx.clone();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Per-game handles for work that can be stopped: download cancels, process stoppers.
pub struct Registry<T>(Arc<Mutex<HashMap<i64, T>>>);

impl<T> Clone for Registry<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<T: Clone + Default> Registry<T> {
    pub fn register(&self, game_id: i64) -> T {
        let handle = T::default();
        lock(&self.0).insert(game_id, handle.clone());
        handle
    }

    pub fn finish(&self, game_id: i64) {
        lock(&self.0).remove(&game_id);
    }

    pub fn get(&self, game_id: i64) -> Option<T> {
        lock(&self.0).get(&game_id).cloned()
    }
}

/// A poisoned lock only means a panic elsewhere; the data is still the best we have.
pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| e.into_inner())
}

pub fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
pub struct AppState {
    client: RwLock<Option<GameyfinClient>>,
    settings: RwLock<Settings>,
    /// Serialises read-modify-write of settings, so concurrent changes are not lost.
    settings_writer: tokio::sync::Mutex<()>,
    /// Set once the settings file has been read. Until then every field holds its default,
    /// which would read as "no server, no session".
    restored: ReadyFlag,
    config_dir: OnceLock<PathBuf>,
    http: OnceLock<reqwest::Client>,
    transfer_http: OnceLock<reqwest::Client>,
    library: crate::library_state::SharedLibraryState,
    /// Every Ludusavi run shares one config dir (the manifest is 17 MB), so runs are serial.
    ludusavi_lock: Arc<tokio::sync::Mutex<()>>,
    /// A scan of the whole machine runs for minutes, in a config directory of its own, so
    /// it never stands between a game and the save it is waiting for at launch.
    save_scan_lock: Arc<tokio::sync::Mutex<()>>,
    /// Games whose automatic sync the user has asked to stop. Read between steps, never
    /// mid-write, so skipping can never leave half a save behind.
    save_skips: Arc<Mutex<HashSet<i64>>>,
    /// Stops two launches downloading the same runtime into the same directory.
    runtime_lock: tokio::sync::Mutex<()>,
    /// Freshness cache: `list_entries` runs several times a second during a transfer.
    catalog: RwLock<Option<(Instant, Vec<Game>)>>,
    /// No expiry on purpose: stale titles beat an empty library when the server is down.
    offline_catalog: RwLock<CachedCatalog>,
    catalog_writer: tokio::sync::Mutex<()>,
    unreachable: AtomicBool,
    image_cache: OnceLock<Arc<crate::image_cache::ImageCache>>,
    umu: RwLock<gameyfin_core::umu::Database>,
    gamepad: OnceLock<crate::gamepad::Handle>,
    /// Shared with every transfer so a change applies mid-download.
    download_limit: gameyfin_core::download::RateLimit,
    pub downloads: Registry<gameyfin_core::download::Cancel>,
    pub processes: Registry<gameyfin_core::Stopper>,
}

impl AppState {
    pub async fn connect_with_cookies(
        &self,
        base_url: &str,
        cookies: HashMap<String, String>,
    ) -> gameyfin_api::ApiResult<()> {
        // A connect timeout keeps an offline start from hanging on the splash.
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(30))
            .build()?;
        let auth = Arc::new(CookieSessionAuth::new(base_url, http.clone()));
        auth.set_cookies(cookies).await;
        *write(&self.client) = Some(GameyfinClient::with_http(base_url, auth, http)?);
        Ok(())
    }

    pub fn disconnect(&self) {
        *write(&self.client) = None;
    }

    pub fn client(&self) -> Option<GameyfinClient> {
        read(&self.client).clone()
    }

    pub fn require_client(&self) -> CommandResult<GameyfinClient> {
        self.client().ok_or(CommandError::NotConnected)
    }

    /// Short requests. Its whole-request timeout would abort a long download.
    pub fn http(&self) -> reqwest::Client {
        self.http
            .get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(USER_AGENT)
                    .timeout(Duration::from_secs(20))
                    .build()
                    .unwrap_or_default()
            })
            .clone()
    }

    /// Long transfers: no overall timeout, but a stalled connection still fails.
    pub fn transfer_http(&self) -> reqwest::Client {
        self.transfer_http
            .get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(USER_AGENT)
                    .connect_timeout(Duration::from_secs(30))
                    .read_timeout(Duration::from_secs(120))
                    .build()
                    .unwrap_or_default()
            })
            .clone()
    }

    /// Fresh cache, else the server, else the offline mirror when the server is unreachable.
    pub async fn games(&self) -> gameyfin_api::ApiResult<Vec<Game>> {
        const TTL: Duration = Duration::from_secs(30);

        if let Some((fetched, games)) = read(&self.catalog).as_ref() {
            if fetched.elapsed() < TTL {
                return Ok(games.clone());
            }
        }

        let Some(client) = self.client() else {
            return self
                .cached_games()
                .ok_or_else(|| gameyfin_api::ApiError::Other("not connected".into()));
        };

        match client.games().await {
            Ok(games) => {
                self.set_reachable(true);
                *write(&self.catalog) = Some((Instant::now(), games.clone()));
                self.remember(|cached| cached.games = games.clone()).await;
                Ok(games)
            }
            Err(e) if e.is_unreachable() => {
                self.set_reachable(false);
                let stale = read(&self.catalog)
                    .as_ref()
                    .map(|(_, games)| games.clone())
                    .or_else(|| self.cached_games());
                stale.ok_or(e)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn libraries(&self) -> gameyfin_api::ApiResult<Vec<Library>> {
        let Some(client) = self.client() else {
            return Ok(read(&self.offline_catalog).libraries.clone());
        };
        match client.libraries().await {
            Ok(libraries) => {
                self.set_reachable(true);
                self.remember(|cached| cached.libraries = libraries.clone())
                    .await;
                Ok(libraries)
            }
            Err(e) if e.is_unreachable() => {
                self.set_reachable(false);
                Ok(read(&self.offline_catalog).libraries.clone())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn game(&self, game_id: i64) -> CommandResult<Game> {
        self.games()
            .await?
            .into_iter()
            .find(|g| g.id == game_id)
            .ok_or_else(|| CommandError::msg("That game is no longer in the library."))
    }

    /// A game's title for messages, never failing.
    pub async fn title(&self, game_id: i64) -> String {
        self.game(game_id)
            .await
            .map(|g| g.title)
            .unwrap_or_else(|_| format!("Game {game_id}"))
    }

    fn cached_games(&self) -> Option<Vec<Game>> {
        let games = read(&self.offline_catalog).games.clone();
        (!games.is_empty()).then_some(games)
    }

    pub fn has_cached_catalog(&self) -> bool {
        !read(&self.offline_catalog).games.is_empty()
    }

    /// Updates the offline mirror, writing it out only when something changed.
    async fn remember(&self, change: impl FnOnce(&mut CachedCatalog)) {
        let _writer = self.catalog_writer.lock().await;
        let snapshot = {
            let mut cached = write(&self.offline_catalog);
            let before = (cached.games.clone(), cached.libraries.clone());
            change(&mut cached);
            (before != (cached.games.clone(), cached.libraries.clone())).then(|| cached.clone())
        };
        let (Some(snapshot), Some(dir)) = (snapshot, self.config_dir.get()) else {
            return;
        };
        if let Err(e) = crate::persist::write_json(&dir.join(CATALOG_FILE), &snapshot, false).await
        {
            tracing::warn!("could not cache the catalogue: {e}");
        }
    }

    /// Drops the catalogue in memory and on disk, so one account's titles never outlive it.
    pub async fn forget_catalog(&self) {
        let _writer = self.catalog_writer.lock().await;
        *write(&self.catalog) = None;
        *write(&self.offline_catalog) = CachedCatalog::default();
        let Some(dir) = self.config_dir.get() else {
            return;
        };
        match tokio::fs::remove_file(dir.join(CATALOG_FILE)).await {
            Ok(()) => tracing::info!("cleared the cached catalogue"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("could not clear the cached catalogue: {e}"),
        }
    }

    /// Only the freshness cache: the offline mirror must survive a failed refetch.
    pub fn invalidate_catalog(&self) {
        *write(&self.catalog) = None;
    }

    pub fn set_reachable(&self, reachable: bool) {
        if self.unreachable.swap(!reachable, Ordering::Relaxed) == reachable {
            tracing::info!(reachable, "the server's reachability changed");
        }
    }

    pub fn is_unreachable(&self) -> bool {
        self.unreachable.load(Ordering::Relaxed)
    }

    /// Asks the server about the session. An unreachable server keeps a stored session.
    pub async fn check_session(&self, has_session: bool) -> (bool, bool) {
        let Some(client) = self.client() else {
            // No client yet with a stored session means startup is still connecting.
            return (has_session, has_session);
        };
        match client.user_info().await {
            Ok(user) => {
                self.set_reachable(true);
                (user.is_some(), false)
            }
            Err(e) if e.is_auth() => {
                self.set_reachable(true);
                (false, false)
            }
            Err(e) => {
                tracing::debug!("could not reach the server: {e}");
                self.set_reachable(false);
                (has_session, true)
            }
        }
    }

    pub fn umu_entry_count(&self) -> usize {
        read(&self.umu).len()
    }

    /// Stale is fine: an early launch gets yesterday's fixes rather than none.
    pub fn load_umu_database(&self, config_dir: &Path) {
        if let Some(cached) = gameyfin_core::umu::load_cache(config_dir) {
            tracing::info!(
                entries = cached.database.len(),
                stale = cached.stale,
                "loaded the cached umu database"
            );
            *write(&self.umu) = cached.database;
        }
    }

    pub async fn refresh_umu_database(&self) -> gameyfin_core::CoreResult<usize> {
        let database =
            gameyfin_core::umu::fetch(&self.http(), gameyfin_core::umu::DEFAULT_API_URL).await?;
        let count = database.len();
        if let Some(dir) = self.config_dir.get().cloned() {
            let to_save = database.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = gameyfin_core::umu::save_cache(&dir, &to_save) {
                    tracing::warn!("could not cache the umu database: {e}");
                }
            });
        }
        *write(&self.umu) = database;
        Ok(count)
    }

    /// The umu id for a title, honouring the fixes setting.
    pub fn umu_id_for(&self, title: &str, steam_app_id: Option<u32>) -> String {
        if !self.settings().umu_fixes {
            return gameyfin_core::umu::DEFAULT_ID.to_string();
        }
        read(&self.umu).resolve(title, steam_app_id)
    }

    /// The umu id for a game, falling back to the generic one.
    pub async fn umu_id_for_game(&self, game_id: i64) -> String {
        match self.game(game_id).await {
            Ok(game) => self.umu_id_for(&game.title, game.steam_app_id()),
            Err(_) => gameyfin_core::umu::DEFAULT_ID.to_string(),
        }
    }

    pub fn set_gamepad(&self, handle: crate::gamepad::Handle) {
        let _ = self.gamepad.set(handle);
    }

    pub fn apply_gamepad_settings(&self) {
        let settings = self.settings();
        if let Some(handle) = self.gamepad.get() {
            handle.set_enabled(settings.gamepad_enabled);
            handle.set_deadzone(settings.gamepad_deadzone);
        }
    }

    /// `None` until startup has restored, so nothing is cached relative to the working dir.
    pub fn image_cache(&self) -> Option<Arc<crate::image_cache::ImageCache>> {
        self.image_cache.get().cloned()
    }

    pub fn ludusavi_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.ludusavi_lock.clone()
    }

    pub fn save_scan_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.save_scan_lock.clone()
    }

    pub fn save_skips(&self) -> Arc<Mutex<HashSet<i64>>> {
        self.save_skips.clone()
    }

    pub async fn runtime_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.runtime_lock.lock().await
    }

    pub fn library(&self) -> &crate::library_state::SharedLibraryState {
        &self.library
    }

    pub fn download_limit(&self) -> gameyfin_core::download::RateLimit {
        self.download_limit.clone()
    }

    /// Empty until startup has restored; nothing is written anywhere before that.
    pub fn config_dir(&self) -> PathBuf {
        self.config_dir.get().cloned().unwrap_or_default()
    }

    pub fn settings(&self) -> Settings {
        read(&self.settings).clone()
    }

    /// Applies a change under the writer lock and persists it; memory changes only on success.
    pub async fn update_settings<R>(
        &self,
        change: impl FnOnce(&mut Settings) -> Result<R, String>,
    ) -> CommandResult<R> {
        let _writer = self.settings_writer.lock().await;
        // Writing before the file has been read would persist the defaults over whatever it
        // holds, which for a stored session means signing the user out.
        let dir = self
            .config_dir
            .get()
            .filter(|_| self.restored.get())
            .ok_or_else(|| CommandError::msg("Gameyfin is still starting. Try again."))?;
        let mut next = self.settings();
        let result = change(&mut next).map_err(CommandError::Message)?;
        crate::persist::write_json(&dir.join(SETTINGS_FILE), &next, true)
            .await
            .map_err(|e| CommandError::msg(format!("could not save settings: {e}")))?;
        *write(&self.settings) = next;
        Ok(result)
    }

    pub async fn set_settings(&self, change: impl FnOnce(&mut Settings)) -> CommandResult<()> {
        self.update_settings(|s| {
            change(s);
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    pub fn is_restored(&self) -> bool {
        self.restored.get()
    }

    /// Waits for startup to have read the settings file, so a command cannot answer from
    /// defaults. Bounded: a startup that never finishes must not hang the window forever.
    pub async fn wait_until_restored(&self) -> bool {
        const GIVE_UP_AFTER: Duration = Duration::from_secs(20);
        if self.restored.get() {
            return true;
        }
        tokio::time::timeout(GIVE_UP_AFTER, self.restored.wait())
            .await
            .is_ok()
    }

    /// Loads everything from disk and reconnects a stored session. True when it authenticated.
    pub async fn restore(&self, config_dir: PathBuf) -> bool {
        let _ = self.config_dir.set(config_dir.clone());
        let _ = self
            .image_cache
            .set(Arc::new(crate::image_cache::ImageCache::new(
                config_dir.join("image-cache"),
            )));
        self.library.load(config_dir.clone()).await;

        let catalog: CachedCatalog =
            crate::persist::read_json_or_default(&config_dir.join(CATALOG_FILE)).await;
        *write(&self.offline_catalog) = catalog;

        let settings =
            match crate::persist::read_json::<Settings>(&config_dir.join(SETTINGS_FILE)).await {
                Ok(settings) => settings.unwrap_or_default(),
                Err(e) => {
                    // Kept aside so the next save cannot silently destroy what could be recovered.
                    tracing::error!("unreadable settings, starting fresh: {e}");
                    let path = config_dir.join(SETTINGS_FILE);
                    let _ = tokio::fs::rename(&path, path.with_extension("json.broken")).await;
                    Settings::default()
                }
            };
        *write(&self.settings) = settings.clone();

        let (Some(url), true) = (settings.server_url.clone(), settings.has_session()) else {
            self.restored.set();
            return false;
        };
        let connected = self
            .connect_with_cookies(&url, settings.cookies.clone())
            .await
            .is_ok();
        // Before the session check, which talks to the server: callers wait for the stored
        // answer, not for a round trip that a slow or absent network can stretch out.
        self.restored.set();
        connected && self.check_session(true).await.0
    }

    #[cfg(test)]
    pub fn set_config_dir(&self, dir: PathBuf) {
        let _ = self.config_dir.set(dir);
        self.restored.set();
    }
}

#[cfg(test)]
pub fn write_settings_for_test(state: &AppState, change: impl FnOnce(&mut Settings)) {
    change(&mut write(&state.settings));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
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

    fn shelf(id: i64, name: &str) -> Library {
        Library {
            id,
            name: name.into(),
            game_ids: vec![1],
        }
    }

    #[tokio::test]
    async fn with_no_client_the_catalogue_comes_from_the_cache() {
        let state = AppState::default();
        assert!(!state.has_cached_catalog());
        assert!(state.games().await.is_err());

        state
            .remember(|c| c.games = vec![a_game(1, "Celeste")])
            .await;

        assert!(state.has_cached_catalog());
        assert_eq!(state.games().await.unwrap()[0].title, "Celeste");
        assert_eq!(state.title(1).await, "Celeste");
        assert_eq!(state.title(2).await, "Game 2");
    }

    #[tokio::test]
    async fn libraries_fall_back_to_the_cache_too() {
        let state = AppState::default();
        assert!(state.libraries().await.unwrap().is_empty());
        state
            .remember(|c| c.libraries = vec![shelf(3, "Shelf")])
            .await;
        assert_eq!(state.libraries().await.unwrap()[0].name, "Shelf");
    }

    #[tokio::test]
    async fn the_catalogue_survives_a_restart() {
        let dir = scratch("catalog");
        let state = AppState::default();
        state.set_config_dir(dir.clone());
        state.remember(|c| c.games = vec![a_game(4, "Hades")]).await;
        state
            .remember(|c| c.libraries = vec![shelf(1, "Main")])
            .await;

        let reloaded = AppState::default();
        reloaded.restore(dir.clone()).await;
        assert_eq!(reloaded.games().await.unwrap()[0].title, "Hades");
        assert_eq!(reloaded.libraries().await.unwrap()[0].name, "Main");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn signing_out_takes_the_cached_catalogue_with_it() {
        let dir = scratch("forget");
        let state = AppState::default();
        state.set_config_dir(dir.clone());
        state.remember(|c| c.games = vec![a_game(2, "Tunic")]).await;
        assert!(dir.join(CATALOG_FILE).exists());

        state.forget_catalog().await;

        assert!(!state.has_cached_catalog());
        assert!(!dir.join(CATALOG_FILE).exists());
        assert!(state.games().await.is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn settings_changes_before_startup_are_refused_not_written_elsewhere() {
        let state = AppState::default();
        assert!(state
            .update_settings(|s| {
                s.auto_install = true;
                Ok(())
            })
            .await
            .is_err());
        assert!(!state.settings().auto_install);
    }

    #[tokio::test]
    async fn a_write_racing_startup_cannot_persist_defaults_over_the_stored_session() {
        let dir = scratch("settings-startup-race");
        let stored = AppState::default();
        stored.set_config_dir(dir.clone());
        stored
            .set_settings(|s| {
                s.server_url = Some("https://games.example".into());
                s.cookies = HashMap::from([("JSESSIONID".to_string(), "abc".to_string())]);
            })
            .await
            .unwrap();

        // The window is up and firing commands while startup is still reading the file.
        let state = AppState::default();
        assert!(!state.is_restored());
        assert!(state
            .update_settings(|s| {
                s.auto_install = true;
                Ok(())
            })
            .await
            .is_err());

        state.restore(dir.clone()).await;
        assert!(state.settings().has_session());
        assert!(state.is_restored());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn concurrent_settings_changes_are_all_kept() {
        let dir = scratch("settings-race");
        let state = Arc::new(AppState::default());
        state.set_config_dir(dir.clone());

        let a = {
            let state = state.clone();
            tokio::spawn(async move {
                state
                    .update_settings(|s| {
                        s.auto_install = true;
                        Ok(())
                    })
                    .await
            })
        };
        let b = {
            let state = state.clone();
            tokio::spawn(async move {
                state
                    .update_settings(|s| {
                        s.close_to_tray = true;
                        Ok(())
                    })
                    .await
            })
        };
        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();

        let reloaded = AppState::default();
        reloaded.restore(dir.clone()).await;
        assert!(reloaded.settings().auto_install && reloaded.settings().close_to_tray);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn corrupt_settings_are_set_aside_rather_than_overwritten() {
        let dir = scratch("settings-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SETTINGS_FILE), b"{ not json").unwrap();

        let state = AppState::default();
        state.restore(dir.clone()).await;
        assert_eq!(state.settings(), Settings::default());
        assert!(dir.join("settings.json.broken").exists());
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
