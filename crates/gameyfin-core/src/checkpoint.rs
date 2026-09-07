//! Resume state for an in-flight download.
//!
//! A large game is tens of gigabytes, so losing progress to a crash, a dropped connection
//! or a closed laptop lid is expensive. GameVault solves this by writing a small sidecar
//! next to the partial file every couple of seconds and resuming with an HTTP `Range`
//! request; this is the same idea with two additions, the expected total is recorded so a
//! truncated file is detected, and the server's `ETag` is kept so a file that changed
//! underneath us is not silently spliced together from two different versions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreResult;

/// How often progress is flushed to disk.
///
/// Two seconds bounds the worst-case loss to a couple of seconds of transfer while keeping
/// the write rate negligible even on a slow disk.
pub const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    /// Bytes already written to the partial file.
    pub received_bytes: u64,
    /// Total size the server reported when the transfer began, if it reported one.
    pub total_bytes: Option<u64>,
    /// The server's `ETag`, used to detect the file changing between attempts.
    pub etag: Option<String>,
    /// Filename parsed from `Content-Disposition`, so a resume keeps the original name.
    pub filename: Option<String>,
}

impl Checkpoint {
    pub fn new(total_bytes: Option<u64>, etag: Option<String>, filename: Option<String>) -> Self {
        Self {
            received_bytes: 0,
            total_bytes,
            etag,
            filename,
        }
    }

    /// Path of the sidecar for a given partial file.
    pub fn sidecar_path(partial: &Path) -> PathBuf {
        let mut name = partial.file_name().unwrap_or_default().to_os_string();
        name.push(".gameyfin-part");
        partial.with_file_name(name)
    }

    pub async fn load(partial: &Path) -> Option<Self> {
        let bytes = tokio::fs::read(Self::sidecar_path(partial)).await.ok()?;
        // A corrupt sidecar is not worth failing over: restarting the download is always
        // a valid fallback, so treat it as absent.
        serde_json::from_slice(&bytes).ok()
    }

    pub async fn save(&self, partial: &Path) -> CoreResult<()> {
        let json =
            serde_json::to_vec(self).map_err(|e| crate::error::CoreError::Other(e.to_string()))?;
        tokio::fs::write(Self::sidecar_path(partial), json).await?;
        Ok(())
    }

    pub async fn clear(partial: &Path) -> CoreResult<()> {
        match tokio::fs::remove_file(Self::sidecar_path(partial)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Whether a stored checkpoint can be used to continue against the current response.
    ///
    /// A changed `ETag` means the server's copy is not the one we started, so resuming
    /// would splice two different files together.
    pub fn is_resumable_against(&self, current_etag: Option<&str>) -> bool {
        if self.received_bytes == 0 {
            return false;
        }
        match (&self.etag, current_etag) {
            (Some(stored), Some(current)) => stored == current,
            // With no ETag on either side there is nothing to contradict the resume.
            (None, None) => true,
            // One side has a validator and the other does not: too weak to trust.
            _ => false,
        }
    }

    /// Reconcile a checkpoint against the partial file actually on disk.
    ///
    /// The sidecar is written after the bytes, so a crash between the two leaves the file
    /// *longer* than the checkpoint claims. Trusting the checkpoint in that case would
    /// re-request bytes already present and duplicate them. The file length is the
    /// authority; the checkpoint only carries the metadata.
    pub fn reconcile(&mut self, file_len: u64) {
        self.received_bytes = file_len;
    }

    pub fn progress(&self) -> Option<f64> {
        let total = self.total_bytes?;
        if total == 0 {
            return None;
        }
        Some((self.received_bytes as f64 / total as f64).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gameyfin-cp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn sidecar_sits_next_to_the_partial_file() {
        let p = Path::new("/downloads/Celeste.zip");
        assert_eq!(
            Checkpoint::sidecar_path(p),
            PathBuf::from("/downloads/Celeste.zip.gameyfin-part")
        );
    }

    #[test]
    fn a_fresh_checkpoint_is_not_resumable() {
        let cp = Checkpoint::new(Some(100), Some("abc".into()), None);
        assert!(!cp.is_resumable_against(Some("abc")));
    }

    #[test]
    fn matching_etag_allows_resume() {
        let mut cp = Checkpoint::new(Some(100), Some("abc".into()), None);
        cp.received_bytes = 40;
        assert!(cp.is_resumable_against(Some("abc")));
    }

    #[test]
    fn changed_etag_blocks_resume() {
        // The server's copy was replaced; splicing would corrupt the file.
        let mut cp = Checkpoint::new(Some(100), Some("abc".into()), None);
        cp.received_bytes = 40;
        assert!(!cp.is_resumable_against(Some("xyz")));
    }

    #[test]
    fn a_disappearing_etag_blocks_resume() {
        let mut cp = Checkpoint::new(Some(100), Some("abc".into()), None);
        cp.received_bytes = 40;
        assert!(!cp.is_resumable_against(None));
    }

    #[test]
    fn absent_etags_on_both_sides_still_allow_resume() {
        // Gameyfin 2.4 sends no ETag, so this is the common path today.
        let mut cp = Checkpoint::new(Some(100), None, None);
        cp.received_bytes = 40;
        assert!(cp.is_resumable_against(None));
    }

    #[test]
    fn reconcile_trusts_the_file_over_the_sidecar() {
        // Bytes are written before the sidecar, so the file can be ahead after a crash.
        let mut cp = Checkpoint::new(Some(100), None, None);
        cp.received_bytes = 40;
        cp.reconcile(57);
        assert_eq!(cp.received_bytes, 57);
    }

    #[test]
    fn progress_is_none_without_a_known_total() {
        let cp = Checkpoint::new(None, None, None);
        assert_eq!(cp.progress(), None);
    }

    #[test]
    fn progress_is_clamped() {
        let mut cp = Checkpoint::new(Some(100), None, None);
        cp.received_bytes = 150;
        assert_eq!(cp.progress(), Some(1.0));
    }

    #[tokio::test]
    async fn round_trips_through_disk() {
        let partial = temp_file("round-trip.bin");
        let mut cp = Checkpoint::new(Some(500), Some("e1".into()), Some("Celeste.zip".into()));
        cp.received_bytes = 123;
        cp.save(&partial).await.unwrap();

        let loaded = Checkpoint::load(&partial).await.expect("checkpoint");
        assert_eq!(loaded, cp);

        Checkpoint::clear(&partial).await.unwrap();
        assert!(Checkpoint::load(&partial).await.is_none());
    }

    #[tokio::test]
    async fn a_corrupt_sidecar_is_treated_as_absent() {
        let partial = temp_file("corrupt.bin");
        tokio::fs::write(Checkpoint::sidecar_path(&partial), b"not json")
            .await
            .unwrap();
        assert!(Checkpoint::load(&partial).await.is_none());
        Checkpoint::clear(&partial).await.unwrap();
    }

    #[tokio::test]
    async fn clearing_a_missing_sidecar_is_not_an_error() {
        let partial = temp_file("never-existed.bin");
        Checkpoint::clear(&partial).await.unwrap();
    }
}
