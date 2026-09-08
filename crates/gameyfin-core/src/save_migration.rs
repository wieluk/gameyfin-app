//! Copying saves from one store to another, for a user changing where saves live.
//!
//! Nothing is ever deleted from the source: a migration that half-succeeds must leave the
//! user with everything they started with.

use std::path::Path;

use serde::Serialize;

use crate::save_store::{SaveStore, StoreResult};
use gameyfin_api::saves::{UploadMetadata, UploadOutcome};
use gameyfin_api::ApiError;

/// Enough problems to see the pattern, not so many that one broken store floods the UI.
const MAX_PROBLEMS: usize = 20;

/// What a finished migration did.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSummary {
    /// Games the source held anything for.
    pub games: usize,
    pub copied: usize,
    /// Already at the destination, byte for byte.
    pub skipped: usize,
    pub failed: usize,
    pub bytes: u64,
    pub problems: Vec<String>,
}

impl MigrationSummary {
    fn note(&mut self, problem: String) {
        self.failed += 1;
        if self.problems.len() < MAX_PROBLEMS {
            self.problems.push(problem);
        }
    }
}

/// Copy every game's saves from `source` to `destination`.
///
/// Only the newest version of each game is copied unless `all_versions` is set: a full
/// history is usually not what someone switching backends wants to pay for.
pub async fn migrate<F>(
    source: &dyn SaveStore,
    destination: &dyn SaveStore,
    scratch: &Path,
    all_versions: bool,
    mut on_progress: F,
) -> StoreResult<MigrationSummary>
where
    F: FnMut(usize, usize),
{
    let games = source.games().await?;
    let mut summary = MigrationSummary {
        games: games.len(),
        ..Default::default()
    };

    tokio::fs::create_dir_all(scratch)
        .await
        .map_err(|e| ApiError::Other(format!("could not prepare a working folder: {e}")))?;

    for (index, game_id) in games.iter().copied().enumerate() {
        on_progress(index, games.len());

        let versions = match source.list(game_id).await {
            Ok(versions) => versions,
            Err(e) => {
                summary.note(format!("Could not read game {game_id}: {e}"));
                continue;
            }
        };

        // Without the destination's list every rerun would copy everything again.
        let present: Vec<String> = match destination.list(game_id).await {
            Ok(existing) => existing.into_iter().map(|v| v.content_hash).collect(),
            Err(e) => {
                summary.note(format!("Could not read the destination for {game_id}: {e}"));
                continue;
            }
        };

        // Oldest first, so the destination ends up ordered the way the source was, and its
        // own retention limit keeps the newest rather than the first to arrive.
        let chosen: Vec<_> = if all_versions {
            versions.into_iter().rev().collect()
        } else {
            versions.into_iter().take(1).collect()
        };

        for version in chosen {
            if present.contains(&version.content_hash) {
                summary.skipped += 1;
                continue;
            }

            let archive = scratch.join(format!("{game_id}-{}.zip", version.id));
            if let Err(e) = source.fetch(game_id, &version.id, &archive).await {
                summary.note(format!(
                    "Could not download {} of {game_id}: {e}",
                    version.id
                ));
                continue;
            }

            let metadata = UploadMetadata {
                content_hash: version.content_hash.clone(),
                platform: version.platform.clone(),
                installation_id: version.installation_id.clone(),
                device_name: version.device_name.clone(),
                ludusavi_title: version.ludusavi_title.clone(),
                // A migration is a deliberate copy, not a concurrent edit: conflict
                // detection would reject every version after the first.
                base_save_id: None,
                force: true,
            };

            match destination.upload(game_id, &archive, &metadata).await {
                Ok(UploadOutcome::Stored(stored)) => {
                    summary.copied += 1;
                    summary.bytes += version.size_bytes;
                    if version.locked {
                        let _ = destination.set_locked(game_id, &stored.id, true).await;
                    }
                }
                Ok(UploadOutcome::Unchanged) => summary.skipped += 1,
                Ok(UploadOutcome::Conflict { .. }) => {
                    summary.note(format!(
                        "The destination rejected {} of {game_id}",
                        version.id
                    ));
                }
                Err(e) => {
                    summary.note(format!("Could not upload {} of {game_id}: {e}", version.id))
                }
            }

            let _ = tokio::fs::remove_file(&archive).await;
        }
    }

    on_progress(games.len(), games.len());
    let _ = tokio::fs::remove_dir_all(scratch).await;
    Ok(summary)
}
