//! Deciding what to do with a game's saves, and doing it.
//!
//! The decisions are pure functions over local and remote state so they can be tested
//! without a server or the Ludusavi binary; [`SaveSync`] performs the transfers.

use std::path::{Path, PathBuf};

use gameyfin_api::saves::{SaveVersion, UploadMetadata, UploadOutcome};
use gameyfin_api::{ApiError, GameyfinClient};
use gameyfin_saves::{SavePlatform, SaveResult};
use serde::{Deserialize, Serialize};

/// What this machine knows about a game's saves, as recorded after the last sync.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalSaveState {
    /// Ludusavi title this game resolved to. Absent means it was never matched.
    pub ludusavi_title: Option<String>,
    /// The server version this machine last restored from or uploaded.
    pub last_synced_save_id: Option<i64>,
    /// Content hash of the last backup taken here.
    pub last_backup_hash: Option<String>,
    pub last_backup_at: Option<String>,
    pub platform: Option<SavePlatform>,
}

/// What the UI shows for one game. Mirrored by hand in `src/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SaveSyncState {
    /// The server has save sync turned off.
    Unsupported,
    /// Ludusavi has no manifest entry for this game, so there is nothing to back up yet.
    Unmatched {
        candidates: Vec<String>,
    },
    NeverSynced,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictChoice {
    /// Upload the local backup, keeping the remote version in the history.
    KeepLocal,
    /// Restore the remote version, discarding the local backup.
    KeepRemote,
    /// Upload, and keep both. Identical to KeepLocal on the server, named for what the
    /// user is choosing rather than for what the request does.
    KeepBoth,
}

/// Decides what a game's saves need, given what is here and what the server holds.
///
/// `local_changed` is whether a fresh backup differs from the one last synced, which the
/// caller establishes by hashing. Kept separate so this stays pure.
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

    let remote_is_ours = local.last_synced_save_id == Some(remote.id);

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

/// Whether Ludusavi's Wine translation could plausibly bridge these two.
///
/// It maps between Windows paths and a Wine prefix, so it only helps when exactly one
/// side is native Windows and the other is a Linux-hosted prefix.
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

/// Where a game's backup archive is staged before upload, and after download.
pub fn archive_path(staging: &Path, game_id: i64) -> PathBuf {
    staging.join(format!("{game_id}.zip"))
}

/// Performs the transfers the decisions call for.
pub struct SaveSync<'a> {
    client: &'a GameyfinClient,
    staging: PathBuf,
    installation_id: Option<String>,
    device_name: Option<String>,
}

impl<'a> SaveSync<'a> {
    pub fn new(client: &'a GameyfinClient, staging: impl Into<PathBuf>) -> Self {
        Self {
            client,
            staging: staging.into(),
            installation_id: None,
            device_name: None,
        }
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

    /// The newest version the server holds, or None when there are none.
    pub async fn newest_remote(&self, game_id: i64) -> Result<Option<SaveVersion>, ApiError> {
        // The endpoint returns newest first.
        Ok(self.client.list_saves(game_id).await?.into_iter().next())
    }

    /// Uploads a staged archive. `base` is the version it was built on top of.
    pub async fn upload(
        &self,
        game_id: i64,
        base: Option<i64>,
        platform: SavePlatform,
        ludusavi_title: Option<String>,
        force: bool,
    ) -> Result<UploadOutcome, ApiError> {
        let archive = archive_path(&self.staging, game_id);
        let hash = gameyfin_api::hash_file(&archive)
            .await
            .map_err(|e| ApiError::Other(e.to_string()))?;

        let metadata = UploadMetadata {
            content_hash: hash,
            platform: platform.to_string(),
            installation_id: self.installation_id.clone(),
            device_name: self.device_name.clone(),
            ludusavi_title,
            base_save_id: base,
            force,
        };

        self.client.upload_save(game_id, &archive, &metadata).await
    }

    /// Fetches a version into staging, ready for Ludusavi to restore from.
    pub async fn fetch(&self, game_id: i64, save_id: i64) -> Result<PathBuf, ApiError> {
        let archive = archive_path(&self.staging, game_id);
        self.client
            .download_save(game_id, save_id, &archive)
            .await?;
        Ok(archive)
    }

    pub fn staging(&self) -> &Path {
        &self.staging
    }
}

/// Hash of the archive currently staged for a game, if there is one.
pub async fn staged_hash(staging: &Path, game_id: i64) -> SaveResult<Option<String>> {
    let archive = archive_path(staging, game_id);
    if !archive.exists() {
        return Ok(None);
    }
    Ok(Some(gameyfin_api::hash_file(&archive).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(id: i64, platform: &str) -> SaveVersion {
        SaveVersion {
            id,
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

    fn synced_to(id: i64) -> LocalSaveState {
        LocalSaveState {
            ludusavi_title: Some("Celeste".into()),
            last_synced_save_id: Some(id),
            last_backup_hash: Some("abc".into()),
            last_backup_at: Some("2026-01-01T00:00:00Z".into()),
            platform: Some(SavePlatform::Windows),
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
            SaveSyncState::Conflict { remote, .. } => assert_eq!(9, remote.id),
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
        let path = archive_path(Path::new("/staging"), 42);
        assert_eq!(PathBuf::from("/staging/42.zip"), path);
    }
}
