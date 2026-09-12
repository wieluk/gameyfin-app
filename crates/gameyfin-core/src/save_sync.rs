//! Deciding what a game's saves need, and doing it. The decisions are pure functions over
//! local and remote state, testable without a server or the helper; [`SaveSync`] transfers.

use std::path::{Path, PathBuf};

use gameyfin_api::saves::{SaveVersion, UploadMetadata, UploadOutcome};
use gameyfin_api::ApiError;

use crate::save_store::SaveStore;
use gameyfin_saves::{SavePlatform, SaveResult};
use serde::{Deserialize, Serialize};

/// What this machine knows about a game's saves, as recorded after the last sync.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalSaveState {
    /// Ludusavi title this game resolved to. Absent means it was never matched.
    pub ludusavi_title: Option<String>,
    /// Whether identification has been attempted, so a game the helper does not recognise
    /// stops re-running the whole search on every refresh.
    pub match_attempted: bool,
    /// Near misses from that search, kept so the user can be offered them without
    /// searching again.
    pub match_candidates: Vec<String>,
    /// Save folders the user named by hand, for a game the database does not cover or
    /// covers incompletely. Registered with Ludusavi as a custom game.
    #[serde(default)]
    pub custom_paths: Vec<String>,
    /// The version this machine last restored from or uploaded, as the store names it.
    #[serde(deserialize_with = "gameyfin_api::saves::lenient_optional_id")]
    pub last_synced_save_id: Option<String>,
    /// Content hash of the last backup taken here.
    pub last_backup_hash: Option<String>,
    pub last_backup_at: Option<String>,
    pub platform: Option<SavePlatform>,
    /// Whether the first-play offer has been answered. Declining leaves no other trace, so
    /// without this the offer would return on every launch.
    #[serde(default)]
    pub pull_offer_answered: bool,
}

/// Whether this machine has never synced this game, so save files on disk may predate sync
/// and no backup has captured them. Restoring over them without asking is silent data loss.
pub fn never_synced(local: &LocalSaveState) -> bool {
    local.last_synced_save_id.is_none() && local.last_backup_hash.is_none()
}

/// What the UI shows for one game. Mirrored by hand in `src/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SaveSyncState {
    /// Switched off in this app's settings. Separate from [`Self::Unsupported`]: blaming the
    /// server for a local toggle sends people to their administrator over their own checkbox.
    Off,
    /// The server has no save sync at all, so it predates the feature.
    Unsupported,
    /// The server could sync saves but an administrator has switched it off.
    Disabled,
    /// Ludusavi has no manifest entry for this game, so there is nothing to back up yet.
    Unmatched {
        candidates: Vec<String>,
    },
    NeverSynced,
    /// The backup helper ran and captured nothing. Distinct from never having tried: it
    /// means the game was recognised but no save files were found where it expected them,
    /// which is a different problem with a different remedy.
    NothingToBackUp {
        /// The title it searched under, which is what the user needs to judge whether the
        /// game was matched to the wrong entry or simply has nothing saved yet.
        title: String,
        /// Whether the helper's database had anything to say about that title at all.
        known: bool,
    },
    InSync {
        last_synced_at: Option<String>,
    },
    /// A local backup the server has not seen.
    LocalNewer {
        local_at: Option<String>,
    },
    /// A version uploaded by another machine.
    RemoteNewer {
        remote_at: Option<String>,
        device: Option<String>,
    },
    /// Both sides moved since the last sync. Only the user can choose.
    Conflict {
        local_at: Option<String>,
        remote: Box<SaveVersion>,
    },
    /// The newest remote version was taken on a platform this one cannot restore directly.
    PlatformMismatch {
        local: SavePlatform,
        remote: SavePlatform,
        /// Whether Ludusavi's Wine translation could bridge it, as opposed to needing a
        /// hand-written mapping.
        cross_os_available: bool,
    },
    Failed {
        message: String,
    },
}

/// How the user resolved a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ConflictChoice {
    /// Upload the local backup, keeping the remote version in the history.
    KeepLocal,
    /// Restore the remote version, discarding the local backup.
    KeepRemote,
    /// Upload, and keep both. Identical to KeepLocal on the server, named for what the
    /// user is choosing rather than for what the request does.
    KeepBoth,
}

/// Decides what a game's saves need. `local_changed` comes from the caller's hashing, so
/// this stays a pure function.
pub fn decide(
    local: &LocalSaveState,
    remote_newest: Option<&SaveVersion>,
    local_changed: bool,
    this_platform: SavePlatform,
) -> SaveSyncState {
    let Some(remote) = remote_newest else {
        return if local.last_backup_hash.is_some() {
            SaveSyncState::LocalNewer {
                local_at: local.last_backup_at.clone(),
            }
        } else {
            SaveSyncState::NeverSynced
        };
    };

    let remote_platform = remote
        .platform
        .parse::<SavePlatform>()
        .unwrap_or(SavePlatform::Unknown);

    // A save that cannot be restored here is worth saying so about before anything else,
    // otherwise the UI would offer a restore that would scatter files into wrong paths.
    if !remote_platform.interchangeable_with(this_platform) {
        return SaveSyncState::PlatformMismatch {
            local: this_platform,
            remote: remote_platform,
            cross_os_available: bridgeable_by_wine_translation(remote_platform, this_platform),
        };
    }

    let remote_is_ours = local.last_synced_save_id.as_deref() == Some(remote.id.as_str());

    match (remote_is_ours, local_changed) {
        (true, false) => SaveSyncState::InSync {
            last_synced_at: remote.created_at.clone(),
        },
        (true, true) => SaveSyncState::LocalNewer {
            local_at: local.last_backup_at.clone(),
        },
        (false, false) => SaveSyncState::RemoteNewer {
            remote_at: remote.created_at.clone(),
            device: remote.device_name.clone(),
        },
        (false, true) => SaveSyncState::Conflict {
            local_at: local.last_backup_at.clone(),
            remote: Box::new(remote.clone()),
        },
    }
}

/// Whether Ludusavi's Wine translation could bridge these two: it maps Windows paths onto a
/// prefix, so exactly one side has to be native Windows.
fn bridgeable_by_wine_translation(a: SavePlatform, b: SavePlatform) -> bool {
    use SavePlatform::*;
    matches!((a, b), (Windows, Linux) | (Linux, Windows))
}

/// Whether a state needs the user to decide before anything can happen.
pub fn needs_attention(state: &SaveSyncState) -> bool {
    matches!(
        state,
        SaveSyncState::Conflict { .. } | SaveSyncState::PlatformMismatch { .. }
    )
}

/// Where a game's uploadable archive lives. Beside the staging directory, never inside
/// it, or packing would try to include its own output.
pub fn archive_path(saves_root: &Path, game_id: i64) -> PathBuf {
    saves_root.join(format!("{game_id}.zip"))
}

/// Packs a backup directory into the archive a store holds. The whole folder, not the inner
/// zip: a restore ignores any folder without the `mapping.yaml` written beside it.
pub fn pack(staging: &Path, archive: &Path) -> std::io::Result<u64> {
    use std::io::Write;

    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(archive)?;
    let mut zip = zip::ZipWriter::new(file);
    // Ludusavi already compressed the payload; compressing it again costs time for nothing.
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let mut entries = Vec::new();
    collect_files(staging, staging, &mut entries)?;
    // Deterministic order, so the same backup produces the same bytes and the content hash
    // does not change for a save that did not.
    entries.sort();

    for relative in entries {
        let absolute = staging.join(&relative);
        zip.start_file(relative.to_string_lossy().replace('\\', "/"), options)?;
        let bytes = std::fs::read(&absolute)?;
        zip.write_all(&bytes)?;
    }

    zip.finish()?;
    Ok(std::fs::metadata(archive)?.len())
}

/// Unpacks a downloaded archive into the staging directory, replacing what was there: a
/// half-merged mixture of two machines' backups is worse than either.
pub fn unpack(archive: &Path, staging: &Path) -> std::io::Result<()> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }
    std::fs::create_dir_all(staging)?;

    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // `enclosed_name` rejects absolute paths and `..`, so a hostile archive cannot
        // write outside the staging directory.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = staging.join(relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;
    }

    Ok(())
}

fn collect_files(root: &Path, dir: &Path, into: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(root, &path, into)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            into.push(relative.to_path_buf());
        }
    }
    Ok(())
}

/// Performs the transfers the decisions call for.
pub struct SaveSync {
    store: Box<dyn SaveStore>,
    /// Holds the per-game staging directories and the packed archives beside them.
    saves_root: PathBuf,
    installation_id: Option<String>,
    device_name: Option<String>,
}

impl SaveSync {
    pub fn new(store: Box<dyn SaveStore>, saves_root: impl Into<PathBuf>) -> Self {
        Self {
            store,
            saves_root: saves_root.into(),
            installation_id: None,
            device_name: None,
        }
    }

    /// Which store this is syncing against, for the UI.
    pub fn describe(&self) -> String {
        self.store.describe()
    }

    pub fn identified_as(
        mut self,
        installation_id: Option<String>,
        device_name: Option<String>,
    ) -> Self {
        self.installation_id = installation_id;
        self.device_name = device_name;
        self
    }

    /// The newest version the store holds, or None when there are none.
    pub async fn newest_remote(&self, game_id: i64) -> Result<Option<SaveVersion>, ApiError> {
        // Every store lists newest first.
        Ok(self.store.list(game_id).await?.into_iter().next())
    }

    /// Every version of a game, newest first.
    pub async fn versions(&self, game_id: i64) -> Result<Vec<SaveVersion>, ApiError> {
        self.store.list(game_id).await
    }

    /// Packs the staged backup and uploads it. `base` is the version it was built on.
    pub async fn upload(
        &self,
        game_id: i64,
        base: Option<String>,
        platform: SavePlatform,
        ludusavi_title: Option<String>,
        force: bool,
    ) -> Result<UploadOutcome, ApiError> {
        let archive = archive_path(&self.saves_root, game_id);
        let staging = self.staging_for(game_id);
        let bytes = pack(&staging, &archive).map_err(|e| {
            tracing::error!(game_id, staging = %staging.display(), error = %e, "packing the backup failed");
            ApiError::Other(e.to_string())
        })?;

        let hash = gameyfin_api::hash_file(&archive)
            .await
            .map_err(|e| ApiError::Other(e.to_string()))?;

        // The upload is the half of a backup that was invisible in the log, so a save that
        // scanned fine and never arrived looked identical to one that was never attempted.
        tracing::info!(
            game_id,
            bytes,
            store = self.store.describe(),
            forced = force,
            "uploading a packed save"
        );

        let metadata = UploadMetadata {
            content_hash: hash,
            platform: platform.to_string(),
            installation_id: self.installation_id.clone(),
            device_name: self.device_name.clone(),
            ludusavi_title,
            base_save_id: base,
            force,
        };

        let outcome = self.store.upload(game_id, &archive, &metadata).await;
        match &outcome {
            Ok(UploadOutcome::Stored(version)) => {
                tracing::info!(game_id, version = %version.id, "save uploaded")
            }
            Ok(UploadOutcome::Unchanged) => {
                tracing::info!(game_id, "the store already holds these exact bytes")
            }
            Ok(UploadOutcome::Conflict { remote, .. }) => {
                tracing::warn!(game_id, remote = %remote.id, "the store rejected the upload as a conflict")
            }
            Err(e) => tracing::error!(game_id, error = %e, "the save could not be uploaded"),
        }
        outcome
    }

    /// Fetches a version and unpacks it, ready for Ludusavi to restore from.
    pub async fn fetch(&self, game_id: i64, save_id: &str) -> Result<PathBuf, ApiError> {
        let archive = archive_path(&self.saves_root, game_id);
        self.store.fetch(game_id, save_id, &archive).await?;

        let staging = self.staging_for(game_id);
        unpack(&archive, &staging).map_err(|e| ApiError::Other(e.to_string()))?;
        Ok(staging)
    }

    /// Where Ludusavi reads and writes this game's backup, before packing.
    pub fn staging_for(&self, game_id: i64) -> PathBuf {
        self.saves_root.join(game_id.to_string())
    }
}

/// Hash of the archive currently staged for a game, if there is one.
pub async fn staged_hash(saves_root: &Path, game_id: i64) -> SaveResult<Option<String>> {
    let archive = archive_path(saves_root, game_id);
    if !archive.exists() {
        return Ok(None);
    }
    Ok(Some(gameyfin_api::hash_file(&archive).await?))
}

#[cfg(test)]
mod tests {
    /// The staging directory has to be the one Ludusavi wrote into: packing the wrong level
    /// finds nothing and uploads an empty archive over a good backup.
    #[test]
    fn staging_is_the_directory_ludusavi_backs_up_into() {
        let root = std::path::Path::new("/games/Gameyfin/Saves");
        let sync = SaveSync::new(
            Box::new(crate::save_store::FolderStore::new("/unused", 1)),
            root,
        );

        assert_eq!(root.join("96"), sync.staging_for(96));
        // And the archive lands beside it, never inside what is being packed.
        assert_eq!(root.join("96.zip"), archive_path(root, 96));
        assert!(!archive_path(root, 96).starts_with(sync.staging_for(96)));
    }

    use super::*;

    fn remote(id: i64, platform: &str) -> SaveVersion {
        SaveVersion {
            id: id.to_string(),
            game_id: 42,
            game_title: Some("Celeste".into()),
            size_bytes: 1024,
            content_hash: "abc".into(),
            platform: platform.into(),
            installation_id: None,
            device_name: Some("desktop".into()),
            ludusavi_title: Some("Celeste".into()),
            locked: false,
            created_at: Some("2026-01-01T00:00:00Z".into()),
        }
    }

    #[test]
    fn only_a_machine_that_has_never_synced_is_asked_before_a_first_play() {
        // Its saves may predate sync, and `local_changed` cannot see what no backup captured.
        assert!(never_synced(&LocalSaveState::default()));
        assert!(!never_synced(&synced_to(7)));
        assert!(!never_synced(&LocalSaveState {
            last_backup_hash: Some("abc".into()),
            ..Default::default()
        }));
    }

    fn synced_to(id: i64) -> LocalSaveState {
        LocalSaveState {
            ludusavi_title: Some("Celeste".into()),
            custom_paths: Vec::new(),
            match_attempted: true,
            match_candidates: Vec::new(),
            last_synced_save_id: Some(id.to_string()),
            last_backup_hash: Some("abc".into()),
            last_backup_at: Some("2026-01-01T00:00:00Z".into()),
            platform: Some(SavePlatform::Windows),
            pull_offer_answered: true,
        }
    }

    #[test]
    fn nothing_anywhere_is_never_synced() {
        let state = decide(
            &LocalSaveState::default(),
            None,
            false,
            SavePlatform::Windows,
        );
        assert_eq!(SaveSyncState::NeverSynced, state);
    }

    #[test]
    fn a_local_backup_with_no_remote_is_ready_to_upload() {
        let state = decide(&synced_to(1), None, false, SavePlatform::Windows);
        assert!(matches!(state, SaveSyncState::LocalNewer { .. }));
    }

    #[test]
    fn matching_ids_and_no_local_change_is_in_sync() {
        let state = decide(
            &synced_to(5),
            Some(&remote(5, "WINDOWS")),
            false,
            SavePlatform::Windows,
        );
        assert!(matches!(state, SaveSyncState::InSync { .. }));
    }

    #[test]
    fn playing_since_the_last_sync_makes_the_local_copy_newer() {
        let state = decide(
            &synced_to(5),
            Some(&remote(5, "WINDOWS")),
            true,
            SavePlatform::Windows,
        );
        assert!(matches!(state, SaveSyncState::LocalNewer { .. }));
    }

    #[test]
    fn another_machine_uploading_makes_the_remote_newer() {
        let state = decide(
            &synced_to(5),
            Some(&remote(9, "WINDOWS")),
            false,
            SavePlatform::Windows,
        );
        match state {
            SaveSyncState::RemoteNewer { device, .. } => {
                assert_eq!(Some("desktop".into()), device)
            }
            other => panic!("expected the remote to be newer, got {other:?}"),
        }
    }

    #[test]
    fn both_sides_moving_is_a_conflict_rather_than_a_silent_overwrite() {
        let state = decide(
            &synced_to(5),
            Some(&remote(9, "WINDOWS")),
            true,
            SavePlatform::Windows,
        );
        match state {
            SaveSyncState::Conflict { remote, .. } => assert_eq!("9", remote.id),
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    #[test]
    fn a_proton_save_restores_onto_windows() {
        let state = decide(
            &synced_to(5),
            Some(&remote(5, "PROTON")),
            false,
            SavePlatform::Windows,
        );
        assert!(matches!(state, SaveSyncState::InSync { .. }));
    }

    #[test]
    fn a_windows_save_does_not_restore_onto_native_linux() {
        let state = decide(
            &synced_to(5),
            Some(&remote(5, "WINDOWS")),
            false,
            SavePlatform::Linux,
        );
        match state {
            SaveSyncState::PlatformMismatch {
                cross_os_available, ..
            } => assert!(cross_os_available),
            other => panic!("expected a platform mismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_mismatch_wine_translation_cannot_bridge_is_reported_as_such() {
        let state = decide(
            &synced_to(5),
            Some(&remote(5, "MACOS")),
            false,
            SavePlatform::Linux,
        );
        match state {
            SaveSyncState::PlatformMismatch {
                cross_os_available, ..
            } => assert!(!cross_os_available),
            other => panic!("expected a platform mismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_platform_mismatch_outranks_a_conflict() {
        // Offering "keep remote" would restore Windows paths onto Linux, so the mismatch
        // has to be settled first.
        let state = decide(
            &synced_to(5),
            Some(&remote(9, "WINDOWS")),
            true,
            SavePlatform::Linux,
        );
        assert!(matches!(state, SaveSyncState::PlatformMismatch { .. }));
    }

    #[test]
    fn an_untagged_remote_version_is_not_blocked() {
        let state = decide(
            &synced_to(5),
            Some(&remote(5, "")),
            false,
            SavePlatform::Linux,
        );
        assert!(matches!(state, SaveSyncState::InSync { .. }));
    }

    #[test]
    fn only_conflicts_and_mismatches_need_the_user() {
        assert!(needs_attention(&SaveSyncState::Conflict {
            local_at: None,
            remote: Box::new(remote(1, "WINDOWS")),
        }));
        assert!(needs_attention(&SaveSyncState::PlatformMismatch {
            local: SavePlatform::Linux,
            remote: SavePlatform::Windows,
            cross_os_available: true,
        }));
        assert!(!needs_attention(&SaveSyncState::InSync {
            last_synced_at: None
        }));
        assert!(!needs_attention(&SaveSyncState::NeverSynced));
    }

    #[test]
    fn the_state_serializes_with_a_kind_tag_for_the_ui() {
        let json = serde_json::to_value(SaveSyncState::RemoteNewer {
            remote_at: Some("2026-01-01T00:00:00Z".into()),
            device: Some("steam-deck".into()),
        })
        .unwrap();

        assert_eq!("remote-newer", json["kind"].as_str().unwrap());
        assert_eq!("steam-deck", json["device"].as_str().unwrap());
    }

    #[test]
    fn archives_are_staged_per_game() {
        let path = archive_path(Path::new("/saves"), 42);
        assert_eq!(PathBuf::from("/saves/42.zip"), path);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gameyfin-pack-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_backup_folder_round_trips_through_the_archive() {
        let root = scratch("roundtrip");
        let staging = root.join("staging");
        // What Ludusavi actually leaves behind: a game folder with its mapping alongside
        // the payload. A restore ignores a folder with no mapping.yaml, so both must survive.
        std::fs::create_dir_all(staging.join("Celeste")).unwrap();
        std::fs::write(staging.join("Celeste/mapping.yaml"), b"name: Celeste").unwrap();
        std::fs::write(staging.join("Celeste/backup-1.zip"), b"payload bytes").unwrap();

        let archive = root.join("42.zip");
        pack(&staging, &archive).unwrap();

        let restored = root.join("restored");
        unpack(&archive, &restored).unwrap();

        assert_eq!(
            b"name: Celeste".to_vec(),
            std::fs::read(restored.join("Celeste/mapping.yaml")).unwrap()
        );
        assert_eq!(
            b"payload bytes".to_vec(),
            std::fs::read(restored.join("Celeste/backup-1.zip")).unwrap()
        );
    }

    #[test]
    fn packing_the_same_backup_twice_produces_the_same_bytes() {
        // Otherwise every upload would look like changed content and defeat the hash check.
        let root = scratch("deterministic");
        let staging = root.join("staging");
        std::fs::create_dir_all(staging.join("Celeste")).unwrap();
        std::fs::write(staging.join("Celeste/mapping.yaml"), b"name: Celeste").unwrap();
        std::fs::write(staging.join("Celeste/backup-1.zip"), b"payload").unwrap();

        pack(&staging, &root.join("first.zip")).unwrap();
        pack(&staging, &root.join("second.zip")).unwrap();

        assert_eq!(
            std::fs::read(root.join("first.zip")).unwrap(),
            std::fs::read(root.join("second.zip")).unwrap()
        );
    }

    #[test]
    fn unpacking_replaces_whatever_was_staged_before() {
        let root = scratch("replace");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("mapping.yaml"), b"new").unwrap();
        let archive = root.join("42.zip");
        pack(&staging, &archive).unwrap();

        let target = root.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("stale.zip"), b"from another machine").unwrap();

        unpack(&archive, &target).unwrap();

        assert!(!target.join("stale.zip").exists());
        assert!(target.join("mapping.yaml").exists());
    }

    #[test]
    fn packing_an_empty_staging_directory_is_not_an_error() {
        let root = scratch("empty");
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();

        let archive = root.join("42.zip");
        assert!(pack(&staging, &archive).is_ok());
        assert!(archive.exists());
    }
}
