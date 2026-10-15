//! Hex digests of downloaded files, compared against the checksums their releases publish.

use std::path::Path;

use sha2::Digest;
use tokio::io::AsyncReadExt;

/// Streams the file through `D`, so a large archive is never held in memory.
pub(crate) async fn file_hex<D: Digest>(path: &Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = D::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use sha2::Sha256;

    use super::*;

    #[tokio::test]
    async fn the_checksum_is_the_file_s_own() {
        let dir = std::env::temp_dir().join(format!("gameyfin-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("archive");
        std::fs::write(&file, b"ludusavi").unwrap();

        // From `printf ludusavi | sha256sum`, so a bug in the hashing here cannot agree
        // with itself and pass.
        let actual = file_hex::<Sha256>(&file).await.unwrap();
        assert_eq!(
            actual,
            "73b491e0ae51ddab5b56c18b8a0056e638289689fe0ef2a8b530297ec41e9ab7"
        );

        // A single changed byte must not still match, or the check protects nothing.
        std::fs::write(&file, b"ludusavj").unwrap();
        assert_ne!(file_hex::<Sha256>(&file).await.unwrap(), actual);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
