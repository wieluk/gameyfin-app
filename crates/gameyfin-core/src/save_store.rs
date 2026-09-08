//! Where synced saves are kept.
//!
//! A Gameyfin server is one option, but not everyone runs a version that has the feature,
//! so a plain folder (any rclone or Syncthing or NextCloud directory) and a WebDAV share
//! are equally valid targets. The decisions in [`crate::save_sync`] do not care which.

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

    /// Store a packed archive as a new version.
    async fn upload(
        &self,
        game_id: i64,
        archive: &Path,
        metadata: &UploadMetadata,
    ) -> StoreResult<UploadOutcome>;

    /// Copy one version's archive to `destination`.
    async fn fetch(&self, game_id: i64, version_id: &str, destination: &Path) -> StoreResult<()>;

    async fn delete(&self, game_id: i64, version_id: &str) -> StoreResult<()>;

    /// Which games this store holds anything for. Used by migration.
    async fn games(&self) -> StoreResult<Vec<i64>>;

    /// Shown in the UI, so the user can tell which target they are looking at.
    fn describe(&self) -> String;
}

// --- The Gameyfin server ---------------------------------------------------------------

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

    async fn games(&self) -> StoreResult<Vec<i64>> {
        // The server has no "which games have saves" route; the caller already knows the
        // library, so migration off a server iterates that instead.
        Ok(Vec::new())
    }

    fn describe(&self) -> String {
        format!("Gameyfin server at {}", self.client.base_url())
    }
}

// --- A plain folder --------------------------------------------------------------------

/// Layout, deliberately append-only:
///
/// ```text
/// <root>/<gameId>/<epochMillis>_<installationId>.zip
/// <root>/<gameId>/<epochMillis>_<installationId>.json
/// ```
///
/// No file is ever rewritten, which is what makes this safe inside a folder that Dropbox
/// or Syncthing is replicating: two machines never write the same path, so neither tool
/// has anything to declare a conflicted copy over. Fixed-width millis means a plain
/// descending filename sort is newest-first.
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
        self.game_dir(game_id).join(format!("{version_id}.zip"))
    }

    fn sidecar_for(&self, game_id: i64, version_id: &str) -> PathBuf {
        self.game_dir(game_id).join(format!("{version_id}.json"))
    }

    /// Removes the oldest unlocked versions past the limit.
    ///
    /// A locked version does not occupy a slot, matching how the server prunes, so pinning
    /// a save cannot quietly squeeze the recent history out.
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

        while let Some(entry) = entries.next_entry().await.map_err(other)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = match tokio::fs::read_to_string(&path).await {
                Ok(text) => text,
                // A sidecar half-written by another machine's sync is skipped rather than
                // failing the whole listing.
                Err(_) => continue,
            };
            if let Ok(sidecar) = serde_json::from_str::<Sidecar>(&text) {
                versions.push(sidecar.version);
            }
        }

        // Ids lead with fixed-width epoch millis, so this is newest first.
        versions.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(versions)
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

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(other)?
            .as_millis();
        let installation = metadata.installation_id.as_deref().unwrap_or("unknown");
        // Zero-padded so a lexical sort stays chronological.
        let id = format!("{now:013}_{installation}");

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

fn iso8601_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod folder_tests {
    use super::*;

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
