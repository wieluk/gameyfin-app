//! On-disk cache for game artwork (covers never change once the server assigns them an id).
//! Prunes itself by last use so it does not grow forever.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How large the cache may grow before the least recently used entries are dropped.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// A cached image's file name.
///
/// Derived from the server path so it is stable across restarts, and sanitised so it
/// cannot escape the cache directory.
fn entry_name(path: &str) -> String {
    let cleaned: String = path
        .trim_start_matches('/')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    cleaned
}

pub struct ImageCache {
    dir: PathBuf,
}

impl ImageCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn entry_path(&self, path: &str) -> PathBuf {
        self.dir.join(entry_name(path))
    }

    /// Read a cached image, if present.
    ///
    /// Reading also refreshes the entry's modification time, which is what the pruner
    /// uses to decide what is still wanted.
    pub async fn get(&self, path: &str) -> Option<(Vec<u8>, String)> {
        let file = self.entry_path(path);
        let bytes = tokio::fs::read(&file).await.ok()?;
        if bytes.is_empty() {
            return None;
        }

        // Best-effort touch; a cache that cannot record a hit is still a usable cache.
        let _ = filetime_touch(&file);

        let content_type = tokio::fs::read_to_string(file.with_extension("type"))
            .await
            .unwrap_or_else(|_| "image/jpeg".to_string());
        Some((bytes, content_type))
    }

    /// Store an image.
    pub async fn put(&self, path: &str, bytes: &[u8], content_type: &str) {
        if bytes.is_empty() {
            return;
        }
        if let Err(e) = tokio::fs::create_dir_all(&self.dir).await {
            tracing::debug!(error = %e, "could not create the image cache directory");
            return;
        }

        let file = self.entry_path(path);
        if let Err(e) = tokio::fs::write(&file, bytes).await {
            tracing::debug!(error = %e, "could not cache an image");
            return;
        }
        let _ = tokio::fs::write(file.with_extension("type"), content_type).await;
    }

    /// Total size of the cache.
    pub fn size(&self) -> u64 {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        entries
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum()
    }

    /// Drop the least recently used entries until the cache fits in `MAX_BYTES`.
    pub fn prune(&self) -> u64 {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };

        let mut files: Vec<(PathBuf, u64, SystemTime)> = entries
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                if !meta.is_file() {
                    return None;
                }
                let path = e.path();
                // Sidecars are removed with the image they belong to.
                if path.extension().is_some_and(|x| x == "type") {
                    return None;
                }
                Some((path, meta.len(), meta.modified().ok()?))
            })
            .collect();

        let total: u64 = files.iter().map(|(_, size, _)| size).sum();
        if total <= MAX_BYTES {
            return 0;
        }

        // Oldest first, so the least recently viewed artwork goes.
        files.sort_by_key(|(_, _, modified)| *modified);

        let mut freed = 0;
        for (path, size, _) in files {
            if total - freed <= MAX_BYTES {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                let _ = std::fs::remove_file(path.with_extension("type"));
                freed += size;
            }
        }

        if freed > 0 {
            tracing::info!(freed, "pruned the image cache");
        }
        freed
    }

    /// Remove everything.
    pub async fn clear(&self) -> std::io::Result<()> {
        match tokio::fs::remove_dir_all(&self.dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// Update a file's modification time to now.
fn filetime_touch(path: &Path) -> std::io::Result<()> {
    // Rewriting the file's own length through `set_len` is the portable way to bump mtime
    // without a dependency, and is a no-op on the contents.
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    let len = file.metadata()?.len();
    file.set_len(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-imgcache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn entry_names_cannot_escape_the_cache_directory() {
        let name = entry_name("/images/../../etc/passwd");
        assert!(!name.contains('/'));
        assert!(!name.contains(".."));
    }

    #[test]
    fn entry_names_are_stable_for_a_path() {
        assert_eq!(entry_name("/images/cover/7"), entry_name("images/cover/7"));
    }

    #[tokio::test]
    async fn stores_and_returns_an_image() {
        let dir = scratch("roundtrip");
        let cache = ImageCache::new(&dir);

        assert!(cache.get("/images/cover/7").await.is_none());
        cache.put("/images/cover/7", b"bytes", "image/png").await;

        let (bytes, content_type) = cache.get("/images/cover/7").await.unwrap();
        assert_eq!(bytes, b"bytes");
        assert_eq!(content_type, "image/png");

        cache.clear().await.unwrap();
    }

    #[tokio::test]
    async fn an_empty_body_is_never_cached() {
        // Caching an empty response would serve a broken image forever.
        let dir = scratch("empty");
        let cache = ImageCache::new(&dir);
        cache.put("/images/cover/1", b"", "image/png").await;
        assert!(cache.get("/images/cover/1").await.is_none());
        cache.clear().await.unwrap();
    }

    #[tokio::test]
    async fn pruning_leaves_a_small_cache_alone() {
        let dir = scratch("small");
        let cache = ImageCache::new(&dir);
        cache.put("/images/cover/1", b"small", "image/png").await;

        assert_eq!(cache.prune(), 0);
        assert!(cache.get("/images/cover/1").await.is_some());

        cache.clear().await.unwrap();
    }

    #[tokio::test]
    async fn size_counts_stored_images() {
        let dir = scratch("size");
        let cache = ImageCache::new(&dir);
        cache.put("/images/cover/1", &[0u8; 100], "image/png").await;
        assert!(cache.size() >= 100);
        cache.clear().await.unwrap();
    }

    #[tokio::test]
    async fn clearing_a_missing_cache_is_not_an_error() {
        ImageCache::new(scratch("missing")).clear().await.unwrap();
    }
}
