//! Resume state for an in-flight download: a sidecar next to the partial file recording
//! progress, expected total, and the server `ETag` so a changed file is never spliced.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreResult;

/// How often progress is flushed to disk; bounds worst-case loss while keeping writes cheap.
pub const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    /// Server `ETag`, used to detect the file changing between attempts.
    pub etag: Option<String>,
    /// Filename from `Content-Disposition`, so a resume keeps the original name.
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

    pub fn sidecar_path(partial: &Path) -> PathBuf {
        let mut name = partial.file_name().unwrap_or_default().to_os_string();
        name.push(".gameyfin-part");
        partial.with_file_name(name)
    }

    pub async fn load(partial: &Path) -> Option<Self> {
        let bytes = tokio::fs::read(Self::sidecar_path(partial)).await.ok()?;
        // A corrupt sidecar is treated as absent; restarting the download is a valid fallback.
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

    /// Whether a stored checkpoint can continue against the current response; a changed or
    /// mismatched `ETag` blocks the resume to avoid splicing two different files.
    pub fn is_resumable_against(&self, current_etag: Option<&str>) -> bool {
        if self.received_bytes == 0 {
            return false;
        }
        match (&self.etag, current_etag) {
            (Some(stored), Some(current)) => stored == current,
            (None, None) => true,
            // One side has a validator and the other does not: too weak to trust.
            _ => false,
        }
    }

    /// Reconcile against the partial file. The sidecar is written after the bytes, so after a
    /// crash the file length is the authority.
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
    fn resuming_needs_progress_and_an_unchanged_etag() {
        for (received, stored, server, expected, why) in [
            (0, Some("abc"), Some("abc"), false, "nothing downloaded yet"),
            (
                40,
                Some("abc"),
                Some("abc"),
                true,
                "same file, partly fetched",
            ),
            // The server's copy was replaced; splicing would corrupt the file.
            (40, Some("abc"), Some("xyz"), false, "etag changed"),
            (40, Some("abc"), None, false, "etag disappeared"),
            // Gameyfin 2.4 sends no ETag, so this is the common path today.
            (40, None, None, true, "no etag on either side"),
        ] {
            let mut cp = Checkpoint::new(Some(100), stored.map(Into::into), None);
            cp.received_bytes = received;
            assert_eq!(cp.is_resumable_against(server), expected, "{why}");
        }
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
    fn progress_needs_a_total_and_never_exceeds_one() {
        assert_eq!(Checkpoint::new(None, None, None).progress(), None);

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
