//! Where synced saves are kept: a Gameyfin server, a plain folder something else
//! replicates, or a WebDAV share. The decisions in [`crate::save_sync`] do not care which.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use gameyfin_api::saves::{SaveVersion, UploadMetadata, UploadOutcome};
use gameyfin_api::{ApiError, GameyfinClient};
use serde::{Deserialize, Serialize};

/// Failures are reported as [`ApiError`] so the whole save path keeps one error type; a
/// store with no HTTP in it uses [`ApiError::Other`].
pub type StoreResult<T> = Result<T, ApiError>;

#[async_trait]
pub trait SaveStore: Send + Sync {
    /// Versions of a game's saves, newest first.
    async fn list(&self, game_id: i64) -> StoreResult<Vec<SaveVersion>>;

    async fn upload(
        &self,
        game_id: i64,
        archive: &Path,
        metadata: &UploadMetadata,
    ) -> StoreResult<UploadOutcome>;

    /// Copy one version's archive to `destination`.
    async fn fetch(&self, game_id: i64, version_id: &str, destination: &Path) -> StoreResult<()>;

    async fn delete(&self, game_id: i64, version_id: &str) -> StoreResult<()>;

    /// Marks a version exempt from retention pruning.
    async fn set_locked(&self, game_id: i64, version_id: &str, locked: bool) -> StoreResult<()>;

    /// Which games this store holds anything for. Used by migration.
    async fn games(&self) -> StoreResult<Vec<i64>>;

    /// Deletes every version the store holds, for every game, and returns what went.
    async fn delete_all(&self) -> StoreResult<Vec<SaveVersion>> {
        let mut deleted = Vec::new();
        for game_id in self.games().await? {
            for version in self.list(game_id).await? {
                self.delete(game_id, &version.id).await?;
                deleted.push(version);
            }
        }
        Ok(deleted)
    }

    /// Shown in the UI, so the user can tell which target they are looking at.
    fn describe(&self) -> String;
}

/// A URL with embedded credentials removed: `describe()` reaches the log, and a log is what
/// people attach to a bug report.
fn redacted(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    match rest.split_once('@') {
        // Userinfo cannot contain a slash, so an `@` later in the path is not one.
        Some((userinfo, host)) if !userinfo.contains('/') => format!("{scheme}://{host}"),
        _ => url.to_string(),
    }
}

/// A version id reduced to something safe as a filename: ids we mint already are, but one
/// that came back from a store must not be able to steer a local write.
pub fn safe_id(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    match cleaned.trim_matches('.') {
        "" => "unnamed".to_string(),
        trimmed => trimmed.to_string(),
    }
}

/// An id no existing version uses. Zero-padded epoch millis keep a lexical sort
/// chronological, and the collision check is what stops one upload replacing another.
fn next_version_id(existing: &[SaveVersion], installation: Option<&str>) -> StoreResult<String> {
    let installation = installation.unwrap_or("unknown");
    let mut millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(other)?
        .as_millis();
    loop {
        let id = format!("{millis:013}_{installation}");
        if !existing.iter().any(|v| v.id == id) {
            return Ok(id);
        }
        millis += 1;
    }
}

pub struct ServerStore {
    client: GameyfinClient,
}

impl ServerStore {
    pub fn new(client: GameyfinClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SaveStore for ServerStore {
    async fn list(&self, game_id: i64) -> StoreResult<Vec<SaveVersion>> {
        self.client.list_saves(game_id).await
    }

    async fn upload(
        &self,
        game_id: i64,
        archive: &Path,
        metadata: &UploadMetadata,
    ) -> StoreResult<UploadOutcome> {
        self.client.upload_save(game_id, archive, metadata).await
    }

    async fn fetch(&self, game_id: i64, version_id: &str, destination: &Path) -> StoreResult<()> {
        self.client
            .download_save(game_id, version_id, destination)
            .await?;
        Ok(())
    }

    async fn delete(&self, game_id: i64, version_id: &str) -> StoreResult<()> {
        self.client.delete_save(game_id, version_id).await
    }

    async fn set_locked(&self, _game_id: i64, version_id: &str, locked: bool) -> StoreResult<()> {
        self.client.set_save_locked(version_id, locked).await
    }

    async fn delete_all(&self) -> StoreResult<Vec<SaveVersion>> {
        // The user's whole list, so games no longer in the library go too.
        let saves = self.client.my_saves().await?;
        let ids: Vec<&str> = saves.iter().map(|save| save.id.as_str()).collect();
        if !ids.is_empty() {
            self.client.delete_saves(&ids).await?;
        }
        Ok(saves)
    }

    async fn games(&self) -> StoreResult<Vec<i64>> {
        // The server has no "which games have saves" route; the caller already knows the
        // library, so migration off a server iterates that instead.
        Ok(Vec::new())
    }

    fn describe(&self) -> String {
        format!("Gameyfin server at {}", self.client.base_url())
    }
}

/// `<root>/<gameId>/<epochMillis>_<installationId>.zip` plus a `.json` sidecar. Append-only,
/// so a replicated folder never has two machines writing one path.
pub struct FolderStore {
    root: PathBuf,
    max_versions: usize,
}

/// The sidecar written beside each archive. It is the whole version record, so listing a
/// directory is enough to describe every version without opening an archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sidecar {
    #[serde(flatten)]
    version: SaveVersion,
}

impl FolderStore {
    pub fn new(root: impl Into<PathBuf>, max_versions: usize) -> Self {
        Self {
            root: root.into(),
            max_versions: max_versions.max(1),
        }
    }

    fn game_dir(&self, game_id: i64) -> PathBuf {
        self.root.join(game_id.to_string())
    }

    fn archive_for(&self, game_id: i64, version_id: &str) -> PathBuf {
        self.game_dir(game_id)
            .join(format!("{}.zip", safe_id(version_id)))
    }

    fn sidecar_for(&self, game_id: i64, version_id: &str) -> PathBuf {
        self.game_dir(game_id)
            .join(format!("{}.json", safe_id(version_id)))
    }

    /// Locking writes a companion file rather than editing the sidecar, which a replicating
    /// tool could copy into a second file carrying the same id.
    fn marker_for(&self, game_id: i64, version_id: &str) -> PathBuf {
        self.game_dir(game_id)
            .join(format!("{}.locked", safe_id(version_id)))
    }

    /// Removes the oldest unlocked versions past the limit. A locked one occupies no slot,
    /// so pinning a save cannot squeeze the recent history out.
    async fn prune(&self, game_id: i64) -> StoreResult<()> {
        let versions = self.list(game_id).await?;
        let unlocked: Vec<_> = versions.into_iter().filter(|v| !v.locked).collect();

        for stale in unlocked.into_iter().skip(self.max_versions) {
            self.delete(game_id, &stale.id).await?;
        }
        Ok(())
    }
}

fn other(error: impl std::fmt::Display) -> ApiError {
    ApiError::Other(error.to_string())
}

#[async_trait]
impl SaveStore for FolderStore {
    async fn list(&self, game_id: i64) -> StoreResult<Vec<SaveVersion>> {
        let dir = self.game_dir(game_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = tokio::fs::read_dir(&dir).await.map_err(other)?;
        let mut versions = Vec::new();
        let mut locked = std::collections::HashSet::new();

        while let Some(entry) = entries.next_entry().await.map_err(other)? {
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("locked") => {
                    if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                        locked.insert(id.to_string());
                    }
                }
                Some("json") => {
                    let text = match tokio::fs::read_to_string(&path).await {
                        Ok(text) => text,
                        // A sidecar half-written by another machine's sync is skipped
                        // rather than failing the whole listing.
                        Err(_) => continue,
                    };
                    if let Ok(sidecar) = serde_json::from_str::<Sidecar>(&text) {
                        versions.push(sidecar.version);
                    }
                }
                _ => {}
            }
        }

        Ok(finish_listing(versions, &locked))
    }

    async fn upload(
        &self,
        game_id: i64,
        archive: &Path,
        metadata: &UploadMetadata,
    ) -> StoreResult<UploadOutcome> {
        let existing = self.list(game_id).await?;
        let newest = existing.first();

        if let Some(newest) = newest {
            if newest
                .content_hash
                .eq_ignore_ascii_case(&metadata.content_hash)
            {
                return Ok(UploadOutcome::Unchanged);
            }
            // No compare-and-swap is possible on a plain folder, so the clash is detected
            // rather than prevented: whoever writes second still writes, and both survive.
            if !metadata.force && Some(newest.id.as_str()) != metadata.base_save_id.as_deref() {
                return Ok(UploadOutcome::Conflict {
                    remote: Box::new(newest.clone()),
                    base_save_id: metadata.base_save_id.clone(),
                });
            }
        }

        let dir = self.game_dir(game_id);
        tokio::fs::create_dir_all(&dir).await.map_err(other)?;

        let id = next_version_id(&existing, metadata.installation_id.as_deref())?;

        let size = tokio::fs::metadata(archive).await.map_err(other)?.len();
        let version = SaveVersion {
            id: id.clone(),
            game_id,
            game_title: None,
            size_bytes: size,
            content_hash: metadata.content_hash.clone(),
            platform: metadata.platform.clone(),
            installation_id: metadata.installation_id.clone(),
            device_name: metadata.device_name.clone(),
            ludusavi_title: metadata.ludusavi_title.clone(),
            locked: false,
            created_at: Some(iso8601_now()),
        };

        tokio::fs::copy(archive, self.archive_for(game_id, &id))
            .await
            .map_err(other)?;
        // The sidecar lands second: a listing skips an archive with no sidecar, so a
        // half-finished upload is invisible rather than corrupt.
        let sidecar = serde_json::to_string_pretty(&Sidecar {
            version: version.clone(),
        })
        .map_err(other)?;
        tokio::fs::write(self.sidecar_for(game_id, &id), sidecar)
            .await
            .map_err(other)?;

        self.prune(game_id).await?;
        Ok(UploadOutcome::Stored(Box::new(version)))
    }

    async fn fetch(&self, game_id: i64, version_id: &str, destination: &Path) -> StoreResult<()> {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(other)?;
        }
        tokio::fs::copy(self.archive_for(game_id, version_id), destination)
            .await
            .map_err(other)?;
        Ok(())
    }

    async fn delete(&self, game_id: i64, version_id: &str) -> StoreResult<()> {
        let _ = tokio::fs::remove_file(self.archive_for(game_id, version_id)).await;
        let _ = tokio::fs::remove_file(self.sidecar_for(game_id, version_id)).await;
        let _ = tokio::fs::remove_file(self.marker_for(game_id, version_id)).await;
        Ok(())
    }

    async fn set_locked(&self, game_id: i64, version_id: &str, locked: bool) -> StoreResult<()> {
        let marker = self.marker_for(game_id, version_id);
        if locked {
            tokio::fs::create_dir_all(self.game_dir(game_id))
                .await
                .map_err(other)?;
            tokio::fs::write(&marker, b"").await.map_err(other)?;
        } else {
            let _ = tokio::fs::remove_file(&marker).await;
        }
        Ok(())
    }

    async fn games(&self) -> StoreResult<Vec<i64>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut entries = tokio::fs::read_dir(&self.root).await.map_err(other)?;
        let mut games = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(other)? {
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i64>().ok())
            {
                games.push(id);
            }
        }
        games.sort_unstable();
        Ok(games)
    }

    fn describe(&self) -> String {
        format!("Folder at {}", self.root.display())
    }
}

/// Applies lock markers, drops duplicate ids and orders newest first. A version listed
/// twice would let the user delete something that appears to still be there.
fn finish_listing(
    mut versions: Vec<SaveVersion>,
    locked: &std::collections::HashSet<String>,
) -> Vec<SaveVersion> {
    for version in &mut versions {
        version.locked = version.locked || locked.contains(&version.id);
    }
    // Ids lead with fixed-width epoch millis, so this is newest first.
    versions.sort_by(|a, b| b.id.cmp(&a.id));
    versions.dedup_by(|a, b| a.id == b.id);
    versions
}

fn iso8601_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// [`FolderStore`]'s layout over WebDAV. Worth having even where the share could be
/// mounted: a Flatpak reaches `$HOME` and removable media, but always has the network.
pub struct WebDavStore {
    base_url: String,
    username: Option<String>,
    password: Option<String>,
    http: reqwest::Client,
    max_versions: usize,
}

impl WebDavStore {
    pub fn new(
        base_url: impl Into<String>,
        username: Option<String>,
        password: Option<String>,
        http: reqwest::Client,
        max_versions: usize,
    ) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        Self {
            base_url: base,
            username,
            password,
            http,
            max_versions: max_versions.max(1),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn authorized(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.username {
            Some(user) => req.basic_auth(user, self.password.clone()),
            None => req,
        }
    }

    async fn request(&self, method: reqwest::Method, path: &str) -> StoreResult<reqwest::Response> {
        let req = self.http.request(method, self.url(path));
        self.authorized(req)
            .send()
            .await
            .map_err(ApiError::Transport)
    }

    /// Names of the immediate children of a collection, or an empty list when it is absent.
    async fn children(&self, path: &str) -> StoreResult<Vec<String>> {
        let method = reqwest::Method::from_bytes(b"PROPFIND").map_err(other)?;
        let req = self
            .http
            .request(method, self.url(path))
            .header("Depth", "1")
            .header(reqwest::header::CONTENT_TYPE, "application/xml");

        let response = self
            .authorized(req)
            .send()
            .await
            .map_err(ApiError::Transport)?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthenticated("webdav".into()));
        }
        if !response.status().is_success() {
            return Err(ApiError::Status {
                endpoint: "webdav propfind".into(),
                status: response.status().as_u16(),
                body: String::new(),
            });
        }

        let body = response.text().await.map_err(ApiError::Transport)?;
        Ok(hrefs(&body)
            .into_iter()
            .filter_map(|href| last_segment(&href))
            .collect())
    }

    async fn ensure_collection(&self, path: &str) -> StoreResult<()> {
        let method = reqwest::Method::from_bytes(b"MKCOL").map_err(other)?;
        let response = self.request(method, path).await?;
        // 405 is "it already exists", which is exactly what we wanted.
        if response.status().is_success() || response.status().as_u16() == 405 {
            return Ok(());
        }
        Err(ApiError::Status {
            endpoint: "webdav mkcol".into(),
            status: response.status().as_u16(),
            body: String::new(),
        })
    }

    async fn prune(&self, game_id: i64) -> StoreResult<()> {
        let versions = self.list(game_id).await?;
        let unlocked: Vec<_> = versions.into_iter().filter(|v| !v.locked).collect();
        for stale in unlocked.into_iter().skip(self.max_versions) {
            self.delete(game_id, &stale.id).await?;
        }
        Ok(())
    }
}

/// Pulls `href` values out of a PROPFIND body. Not a full XML parse: the element is always
/// `href`, prefixed or not, and its payload is a path.
fn hrefs(xml: &str) -> Vec<String> {
    let mut found = Vec::new();
    let lowered = xml.to_ascii_lowercase();
    let mut cursor = 0;

    while let Some(open) = lowered[cursor..].find("href") {
        let after_name = cursor + open + "href".len();
        // Skip to the end of the opening tag, guarding against "href" inside an attribute.
        let Some(tag_end) = lowered[after_name..].find('>') else {
            break;
        };
        let value_start = after_name + tag_end + 1;
        let Some(value_end) = lowered[value_start..].find('<') else {
            break;
        };
        let value = xml[value_start..value_start + value_end].trim();
        if !value.is_empty() {
            found.push(value.to_string());
        }
        cursor = value_start + value_end;
    }
    found
}

/// The final path component of an href, ignoring any trailing slash on a collection.
fn last_segment(href: &str) -> Option<String> {
    let trimmed = href.trim_end_matches('/');
    let segment = trimmed.rsplit('/').next()?;
    if segment.is_empty() {
        None
    } else {
        Some(segment.to_string())
    }
}

#[async_trait]
impl SaveStore for WebDavStore {
    async fn list(&self, game_id: i64) -> StoreResult<Vec<SaveVersion>> {
        let dir = game_id.to_string();
        let names = self.children(&dir).await?;

        let locked: std::collections::HashSet<String> = names
            .iter()
            .filter_map(|name| name.strip_suffix(".locked"))
            .map(str::to_string)
            .collect();

        let mut versions = Vec::new();
        for name in names.iter().filter(|n| n.ends_with(".json")) {
            let response = self
                .request(reqwest::Method::GET, &format!("{dir}/{name}"))
                .await?;
            if !response.status().is_success() {
                continue;
            }
            let text = response.text().await.map_err(ApiError::Transport)?;
            if let Ok(sidecar) = serde_json::from_str::<Sidecar>(&text) {
                versions.push(sidecar.version);
            }
        }

        Ok(finish_listing(versions, &locked))
    }

    async fn upload(
        &self,
        game_id: i64,
        archive: &Path,
        metadata: &UploadMetadata,
    ) -> StoreResult<UploadOutcome> {
        let existing = self.list(game_id).await?;
        if let Some(newest) = existing.first() {
            if newest
                .content_hash
                .eq_ignore_ascii_case(&metadata.content_hash)
            {
                return Ok(UploadOutcome::Unchanged);
            }
            if !metadata.force && Some(newest.id.as_str()) != metadata.base_save_id.as_deref() {
                return Ok(UploadOutcome::Conflict {
                    remote: Box::new(newest.clone()),
                    base_save_id: metadata.base_save_id.clone(),
                });
            }
        }

        let dir = game_id.to_string();
        self.ensure_collection(&dir).await?;

        let id = next_version_id(&existing, metadata.installation_id.as_deref())?;

        let bytes = tokio::fs::read(archive).await.map_err(other)?;
        let version = SaveVersion {
            id: id.clone(),
            game_id,
            game_title: None,
            size_bytes: bytes.len() as u64,
            content_hash: metadata.content_hash.clone(),
            platform: metadata.platform.clone(),
            installation_id: metadata.installation_id.clone(),
            device_name: metadata.device_name.clone(),
            ludusavi_title: metadata.ludusavi_title.clone(),
            locked: false,
            created_at: Some(iso8601_now()),
        };

        let put = |path: String, body: Vec<u8>| {
            let req = self.http.put(self.url(&path)).body(body);
            self.authorized(req).send()
        };

        let response = put(format!("{dir}/{id}.zip"), bytes)
            .await
            .map_err(ApiError::Transport)?;
        if !response.status().is_success() {
            return Err(ApiError::Status {
                endpoint: "webdav put".into(),
                status: response.status().as_u16(),
                body: String::new(),
            });
        }

        // Sidecar second, so an interrupted upload leaves an archive nothing lists.
        let sidecar = serde_json::to_vec(&Sidecar {
            version: version.clone(),
        })
        .map_err(other)?;
        let response = put(format!("{dir}/{id}.json"), sidecar)
            .await
            .map_err(ApiError::Transport)?;
        if !response.status().is_success() {
            return Err(ApiError::Status {
                endpoint: "webdav put".into(),
                status: response.status().as_u16(),
                body: String::new(),
            });
        }

        self.prune(game_id).await?;
        Ok(UploadOutcome::Stored(Box::new(version)))
    }

    async fn fetch(&self, game_id: i64, version_id: &str, destination: &Path) -> StoreResult<()> {
        let response = self
            .request(reqwest::Method::GET, &format!("{game_id}/{version_id}.zip"))
            .await?;
        if !response.status().is_success() {
            return Err(ApiError::Status {
                endpoint: "webdav get".into(),
                status: response.status().as_u16(),
                body: String::new(),
            });
        }

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(other)?;
        }
        let bytes = response.bytes().await.map_err(ApiError::Transport)?;
        tokio::fs::write(destination, &bytes).await.map_err(other)?;
        Ok(())
    }

    async fn delete(&self, game_id: i64, version_id: &str) -> StoreResult<()> {
        for extension in ["zip", "json", "locked"] {
            let _ = self
                .request(
                    reqwest::Method::DELETE,
                    &format!("{game_id}/{version_id}.{extension}"),
                )
                .await;
        }
        Ok(())
    }

    async fn set_locked(&self, game_id: i64, version_id: &str, locked: bool) -> StoreResult<()> {
        let path = format!("{game_id}/{version_id}.locked");
        let response = if locked {
            let req = self.http.put(self.url(&path)).body(Vec::new());
            self.authorized(req)
                .send()
                .await
                .map_err(ApiError::Transport)?
        } else {
            self.request(reqwest::Method::DELETE, &path).await?
        };

        // Unlocking something already unlocked is a 404, which is the state we wanted.
        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(ApiError::Status {
            endpoint: "webdav lock".into(),
            status: response.status().as_u16(),
            body: String::new(),
        })
    }

    async fn games(&self) -> StoreResult<Vec<i64>> {
        let mut games: Vec<i64> = self
            .children("")
            .await?
            .into_iter()
            .filter_map(|name| name.parse().ok())
            .collect();
        games.sort_unstable();
        Ok(games)
    }

    fn describe(&self) -> String {
        format!("WebDAV share at {}", redacted(&self.base_url))
    }
}

#[cfg(test)]
mod folder_tests {
    use super::*;

    #[test]
    fn a_store_supplied_id_cannot_steer_a_local_write() {
        // A hostile or broken store answers with whatever it likes.
        assert_eq!(safe_id("../../.bashrc"), "_.._.bashrc");
        assert_eq!(safe_id("0001700000000_abc"), "0001700000000_abc");
        assert_eq!(safe_id("/"), "_");
        assert_eq!(safe_id(".."), "unnamed");
        assert!(!safe_id("a/b\\c").contains(['/', '\\']));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive(dir: &Path, bytes: &[u8]) -> PathBuf {
        let path = dir.join("upload.zip");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn meta(hash: &str, base: Option<&str>, force: bool) -> UploadMetadata {
        UploadMetadata {
            content_hash: hash.into(),
            platform: "WINDOWS".into(),
            installation_id: Some("device-a".into()),
            device_name: Some("desktop".into()),
            ludusavi_title: Some("Celeste".into()),
            base_save_id: base.map(str::to_string),
            force,
        }
    }

    async fn store_with(root: &Path, max: usize) -> FolderStore {
        FolderStore::new(root, max)
    }

    #[test]
    fn a_password_in_the_address_is_kept_out_of_the_description() {
        assert_eq!(
            "https://cloud.example/dav",
            redacted("https://user:secret@cloud.example/dav")
        );
        assert_eq!(
            "https://cloud.example/dav",
            redacted("https://cloud.example/dav")
        );
        // An `@` in the path is not credentials, and must survive.
        assert_eq!(
            "https://cloud.example/dav/me@example.com",
            redacted("https://cloud.example/dav/me@example.com")
        );
        assert_eq!("not a url", redacted("not a url"));
    }

    #[tokio::test]
    async fn an_empty_folder_lists_nothing() {
        let root = scratch("empty");
        let store = store_with(&root, 5).await;
        assert!(store.list(42).await.unwrap().is_empty());
        assert!(store.games().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_first_upload_is_stored_and_listed() {
        let root = scratch("first");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");

        let outcome = store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap();

        let stored = match outcome {
            UploadOutcome::Stored(v) => v,
            other => panic!("expected a stored version, got {other:?}"),
        };
        let listed = store.list(42).await.unwrap();
        assert_eq!(1, listed.len());
        assert_eq!(stored.id, listed[0].id);
        assert_eq!("aaa", listed[0].content_hash);
        assert_eq!(vec![42], store.games().await.unwrap());
    }

    #[tokio::test]
    async fn two_uploads_in_the_same_millisecond_both_survive() {
        let root = scratch("collide");
        let store = store_with(&root, 5).await;

        // No sleep between them: the ids would otherwise collide and one would be lost.
        for (body, hash) in [
            (&b"PK\x03\x04one"[..], "aaa"),
            (&b"PK\x03\x04two"[..], "bbb"),
        ] {
            let file = archive(&root, body);
            store
                .upload(42, &file, &meta(hash, None, true))
                .await
                .unwrap();
        }

        let listed = store.list(42).await.unwrap();
        assert_eq!(2, listed.len());
        // Newest first, so the second upload still sorts ahead of the first.
        assert_eq!("bbb", listed[0].content_hash);
    }

    #[tokio::test]
    async fn identical_content_is_not_stored_again() {
        let root = scratch("same");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");
        let first = store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap();
        let base = match first {
            UploadOutcome::Stored(v) => v.id,
            other => panic!("got {other:?}"),
        };

        let again = store
            .upload(42, &file, &meta("aaa", Some(&base), false))
            .await
            .unwrap();

        assert_eq!(UploadOutcome::Unchanged, again);
        assert_eq!(1, store.list(42).await.unwrap().len());
    }

    #[tokio::test]
    async fn uploading_against_a_stale_base_is_a_conflict() {
        let root = scratch("conflict");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");
        store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap();

        // Another machine's turn, still believing the folder was empty.
        let outcome = store
            .upload(42, &file, &meta("bbb", None, false))
            .await
            .unwrap();

        match outcome {
            UploadOutcome::Conflict { remote, .. } => assert_eq!("aaa", remote.content_hash),
            other => panic!("expected a conflict, got {other:?}"),
        }
        assert_eq!(
            1,
            store.list(42).await.unwrap().len(),
            "nothing was written"
        );
    }

    #[tokio::test]
    async fn forcing_past_a_conflict_keeps_both_versions() {
        let root = scratch("force");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");
        store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap();

        store
            .upload(42, &file, &meta("bbb", None, true))
            .await
            .unwrap();

        let listed = store.list(42).await.unwrap();
        assert_eq!(2, listed.len(), "the losing version is kept, not replaced");
    }

    #[tokio::test]
    async fn two_devices_never_write_the_same_file() {
        // The whole point of the layout: a folder replicated by Dropbox or Syncthing must
        // never see two machines touch one path, or it invents "conflicted copy" files.
        let root = scratch("devices");
        let store = store_with(&root, 10).await;
        let file = archive(&root, b"PK\x03\x04one");

        let mut a = meta("aaa", None, true);
        a.installation_id = Some("device-a".into());
        let mut b = meta("bbb", None, true);
        b.installation_id = Some("device-b".into());

        store.upload(42, &file, &a).await.unwrap();
        store.upload(42, &file, &b).await.unwrap();

        let ids: Vec<_> = store
            .list(42)
            .await
            .unwrap()
            .into_iter()
            .map(|v| v.id)
            .collect();
        assert_eq!(2, ids.len());
        assert!(ids.iter().any(|id| id.ends_with("device-a")));
        assert!(ids.iter().any(|id| id.ends_with("device-b")));
    }

    #[tokio::test]
    async fn retention_skips_a_locked_version() {
        // Locking is written into the sidecar, so it is set up directly here rather than
        // through upload, which never locks anything.
        let root = scratch("locked");
        let store = store_with(&root, 1).await;
        let file = archive(&root, b"PK\x03\x04one");
        std::fs::create_dir_all(root.join("42")).unwrap();

        for (id, hash, locked) in [
            ("0000000000001_device-a", "old-locked", true),
            ("0000000000002_device-a", "middle", false),
        ] {
            let version = SaveVersion {
                id: id.into(),
                game_id: 42,
                game_title: None,
                size_bytes: 4,
                content_hash: hash.into(),
                platform: "WINDOWS".into(),
                installation_id: Some("device-a".into()),
                device_name: None,
                ludusavi_title: None,
                locked,
                created_at: None,
            };
            std::fs::write(
                root.join(format!("42/{id}.json")),
                serde_json::to_string(&Sidecar { version }).unwrap(),
            )
            .unwrap();
            std::fs::write(root.join(format!("42/{id}.zip")), b"PK\x03\x04").unwrap();
        }

        // A limit of one, so retention would drop everything but the newest upload.
        store
            .upload(42, &file, &meta("new", None, true))
            .await
            .unwrap();

        let hashes: Vec<_> = store
            .list(42)
            .await
            .unwrap()
            .into_iter()
            .map(|v| v.content_hash)
            .collect();
        assert!(
            hashes.contains(&"old-locked".to_string()),
            "locked survived: {hashes:?}"
        );
        assert!(
            hashes.contains(&"new".to_string()),
            "newest survived: {hashes:?}"
        );
        assert!(
            !hashes.contains(&"middle".to_string()),
            "unlocked was pruned: {hashes:?}"
        );
    }

    #[tokio::test]
    async fn retention_drops_the_oldest_unlocked_versions() {
        let root = scratch("retention");
        let store = store_with(&root, 2).await;
        let file = archive(&root, b"PK\x03\x04one");

        for hash in ["a", "b", "c", "d"] {
            store
                .upload(42, &file, &meta(hash, None, true))
                .await
                .unwrap();
            // Distinct millis, so ordering is unambiguous.
            tokio::time::sleep(std::time::Duration::from_millis(3)).await;
        }

        let listed = store.list(42).await.unwrap();
        assert_eq!(2, listed.len());
        assert_eq!("d", listed[0].content_hash, "newest survives");
        assert_eq!("c", listed[1].content_hash);
    }

    #[tokio::test]
    async fn a_version_round_trips_through_fetch() {
        let root = scratch("roundtrip");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04payload");
        let stored = match store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap()
        {
            UploadOutcome::Stored(v) => v,
            other => panic!("got {other:?}"),
        };

        let destination = root.join("fetched.zip");
        store.fetch(42, &stored.id, &destination).await.unwrap();

        assert_eq!(
            b"PK\x03\x04payload".to_vec(),
            std::fs::read(&destination).unwrap()
        );
    }

    #[tokio::test]
    async fn deleting_removes_both_the_archive_and_its_sidecar() {
        let root = scratch("delete");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");
        let stored = match store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap()
        {
            UploadOutcome::Stored(v) => v,
            other => panic!("got {other:?}"),
        };

        store.delete(42, &stored.id).await.unwrap();

        assert!(store.list(42).await.unwrap().is_empty());
        let left: Vec<_> = std::fs::read_dir(root.join("42")).unwrap().collect();
        assert!(left.is_empty(), "no stray files behind");
    }

    #[tokio::test]
    async fn an_archive_with_no_sidecar_is_invisible() {
        // An upload interrupted midway leaves a zip with no sidecar. Listing must ignore
        // it rather than offering a version whose metadata nobody knows.
        let root = scratch("halfwritten");
        let store = store_with(&root, 5).await;
        std::fs::create_dir_all(root.join("42")).unwrap();
        std::fs::write(root.join("42/0000000000001_device-a.zip"), b"PK\x03\x04").unwrap();

        assert!(store.list(42).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn locking_writes_a_marker_and_survives_pruning() {
        let root = scratch("lockmarker");
        let store = store_with(&root, 1).await;
        let file = archive(&root, b"PK\x03\x04one");

        let first = match store
            .upload(42, &file, &meta("keep-me", None, true))
            .await
            .unwrap()
        {
            UploadOutcome::Stored(v) => v,
            other => panic!("got {other:?}"),
        };
        store.set_locked(42, &first.id, true).await.unwrap();
        assert!(root.join(format!("42/{}.locked", first.id)).exists());

        let listed = store.list(42).await.unwrap();
        assert!(listed[0].locked, "the marker is reflected in the listing");

        // A limit of one, so an unlocked version in its place would have been pruned.
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;
        store
            .upload(42, &file, &meta("newer", None, true))
            .await
            .unwrap();

        let hashes: Vec<_> = store
            .list(42)
            .await
            .unwrap()
            .into_iter()
            .map(|v| v.content_hash)
            .collect();
        assert!(
            hashes.contains(&"keep-me".to_string()),
            "locked survived: {hashes:?}"
        );
    }

    #[tokio::test]
    async fn unlocking_removes_the_marker() {
        let root = scratch("unlock");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");
        let stored = match store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap()
        {
            UploadOutcome::Stored(v) => v,
            other => panic!("got {other:?}"),
        };

        store.set_locked(42, &stored.id, true).await.unwrap();
        store.set_locked(42, &stored.id, false).await.unwrap();

        assert!(!root.join(format!("42/{}.locked", stored.id)).exists());
        assert!(!store.list(42).await.unwrap()[0].locked);
    }

    #[tokio::test]
    async fn a_duplicated_sidecar_lists_the_version_once() {
        // What a replicating tool leaves behind: the same record under a second name. Two
        // entries for one id would let the user delete something that still appears present.
        let root = scratch("dupe");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");
        let stored = match store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap()
        {
            UploadOutcome::Stored(v) => v,
            other => panic!("got {other:?}"),
        };

        let original = std::fs::read(root.join(format!("42/{}.json", stored.id))).unwrap();
        std::fs::write(root.join("42/0000000000001_conflicted copy.json"), original).unwrap();

        assert_eq!(1, store.list(42).await.unwrap().len());
    }

    #[tokio::test]
    async fn a_corrupt_sidecar_does_not_break_the_listing() {
        let root = scratch("corrupt");
        let store = store_with(&root, 5).await;
        let file = archive(&root, b"PK\x03\x04one");
        store
            .upload(42, &file, &meta("aaa", None, false))
            .await
            .unwrap();
        std::fs::write(root.join("42/9999999999999_device-x.json"), b"{ not json").unwrap();

        let listed = store.list(42).await.unwrap();
        assert_eq!(1, listed.len(), "the readable version still lists");
    }
}
