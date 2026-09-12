//! A user-installed save backup helper. The pinned copy beside the app is read-only in a
//! Flatpak, so an update lands in the config directory and takes precedence, as Wine does.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CoreError, CoreResult};

const RELEASES_API: &str = "https://api.github.com/repos/mtkennerly/ludusavi/releases";

/// Marker recording what was installed, so the version shows without running anything.
const MARKER: &str = ".gameyfin-ludusavi.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveToolRelease {
    pub version: String,
    pub asset: String,
    pub url: String,
    pub size_bytes: u64,
    /// From GitHub's asset digest: catches a corrupted transfer, not a malicious publisher.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledSaveTool {
    pub version: String,
    pub binary: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveToolStatus {
    /// None until the helper has run once and downloaded it.
    pub manifest: Option<ManifestInfo>,
    /// A user-installed copy, if there is one. Absent means the bundled build is in use.
    pub installed: Option<InstalledSaveTool>,
    /// The version shipped with the app, which is the fallback.
    pub bundled: Option<String>,
    pub latest: Option<SaveToolRelease>,
    /// Recent versions, newest first, for choosing a specific one.
    pub available: Vec<String>,
}

impl SaveToolStatus {
    /// Whether a newer version exists. Unknown when the feed could not be read: unreachable
    /// is not up to date, and saying otherwise hides a pending update.
    pub fn update_available(&self) -> bool {
        let Some(latest) = &self.latest else {
            return false;
        };
        match self.in_use() {
            Some(current) => current != latest.version,
            None => true,
        }
    }

    /// The version actually being run: an installed copy wins over the bundled one.
    pub fn in_use(&self) -> Option<&str> {
        self.installed
            .as_ref()
            .map(|i| i.version.as_str())
            .or(self.bundled.as_deref())
    }
}

pub fn save_tool_root(config_dir: &Path) -> PathBuf {
    config_dir.join("ludusavi-bin")
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "ludusavi.exe"
    } else {
        "ludusavi"
    }
}

/// The downloaded database of where games keep their saves. Separate from the helper's
/// version: it changes far more often, and an unrecognised game is usually waiting for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ManifestInfo {
    /// When it was last written, RFC 3339.
    pub updated_at: Option<String>,
    pub bytes: u64,
}

/// What the helper knows, read from the file rather than from a record of the last check:
/// a failed update must not look like a fresh one.
pub fn manifest_info(ludusavi_config_dir: &Path) -> Option<ManifestInfo> {
    let metadata = std::fs::metadata(ludusavi_config_dir.join("manifest.yaml")).ok()?;
    Some(ManifestInfo {
        updated_at: metadata.modified().ok().and_then(|time| {
            time::OffsetDateTime::from(time)
                .format(&time::format_description::well_known::Rfc3339)
                .ok()
        }),
        bytes: metadata.len(),
    })
}

/// The user-installed helper, or None when only the bundled one is present.
pub fn installed(config_dir: &Path) -> Option<InstalledSaveTool> {
    let marker = save_tool_root(config_dir).join(MARKER);
    let text = std::fs::read_to_string(marker).ok()?;
    let record: InstalledSaveTool = serde_json::from_str(&text).ok()?;
    // A half-finished install should report as absent rather than as a broken version.
    record.binary.is_file().then_some(record)
}

fn asset_for(version: &str) -> CoreResult<String> {
    let suffix = if cfg!(windows) {
        "win64.zip"
    } else if cfg!(target_os = "macos") {
        "mac.tar.gz"
    } else if cfg!(target_arch = "x86_64") {
        "linux.tar.gz"
    } else {
        return Err(CoreError::Other(
            "the save helper does not publish a build for this architecture".into(),
        ));
    };
    Ok(format!("ludusavi-{version}-{suffix}"))
}

pub async fn releases(http: &reqwest::Client, limit: usize) -> CoreResult<Vec<SaveToolRelease>> {
    // GitHub answers 403 without a user agent, which reads as a permissions problem.
    let feed: Vec<serde_json::Value> = http
        .get(format!("{RELEASES_API}?per_page={limit}"))
        .header(reqwest::header::USER_AGENT, "Gameyfin-Desktop")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mut found = Vec::new();
    for entry in feed {
        // Pre-releases are opt-in upstream and not something to offer by default.
        if entry.get("prerelease").and_then(serde_json::Value::as_bool) == Some(true) {
            continue;
        }
        let Some(version) = entry
            .get("tag_name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(wanted) = asset_for(&version) else {
            continue;
        };
        let Some(assets) = entry.get("assets").and_then(serde_json::Value::as_array) else {
            continue;
        };
        let Some(asset) = assets
            .iter()
            .find(|a| a.get("name").and_then(serde_json::Value::as_str) == Some(wanted.as_str()))
        else {
            continue;
        };

        found.push(SaveToolRelease {
            version,
            asset: wanted,
            url: asset
                .get("browser_download_url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            size_bytes: asset
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            sha256: asset
                .get("digest")
                .and_then(serde_json::Value::as_str)
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(str::to_string),
        });
    }

    if found.is_empty() {
        return Err(CoreError::Other(
            "no save helper build was published for this system".into(),
        ));
    }
    Ok(found)
}

/// Downloads and installs one release, replacing whatever was installed before.
pub async fn install<F>(
    config_dir: &Path,
    release: &SaveToolRelease,
    downloader: &crate::download::Downloader,
    on_progress: F,
) -> CoreResult<InstalledSaveTool>
where
    F: FnMut(crate::download::Progress) + Send,
{
    let root = save_tool_root(config_dir);
    tokio::fs::create_dir_all(&root).await?;
    let archive = root.join(&release.asset);

    downloader
        .download(&release.url, &archive, |req| req, on_progress)
        .await?;

    if let Some(expected) = &release.sha256 {
        let actual = sha256_of(&archive)?;
        if !actual.eq_ignore_ascii_case(expected) {
            // Left behind, a partial file would be resumed rather than refetched.
            let _ = tokio::fs::remove_file(&archive).await;
            return Err(CoreError::Other(
                "the downloaded save helper did not match its checksum".into(),
            ));
        }
    }

    let staging = root.join(".unpacking");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await?;

    // Decompression is CPU-bound and would otherwise stall the async runtime.
    let archive_for_unpack = archive.clone();
    let staging_for_unpack = staging.clone();
    tokio::task::spawn_blocking(move || unpack(&archive_for_unpack, &staging_for_unpack))
        .await
        .map_err(|e| CoreError::Other(format!("unpacking the save helper panicked: {e}")))??;

    let unpacked = find_binary(&staging)?;
    let target = root.join(binary_name());
    let _ = tokio::fs::remove_file(&target).await;
    tokio::fs::rename(&unpacked, &target).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = tokio::fs::metadata(&target).await?.permissions();
        permissions.set_mode(0o755);
        tokio::fs::set_permissions(&target, permissions).await?;
    }

    let _ = tokio::fs::remove_dir_all(&staging).await;
    let _ = tokio::fs::remove_file(&archive).await;

    let record = InstalledSaveTool {
        version: release.version.clone(),
        binary: target,
    };
    tokio::fs::write(
        root.join(MARKER),
        serde_json::to_vec_pretty(&record)
            .map_err(|e| CoreError::Other(format!("could not record the version: {e}")))?,
    )
    .await?;

    Ok(record)
}

/// Removes a user-installed helper, falling back to the bundled one.
pub async fn remove(config_dir: &Path) -> CoreResult<()> {
    let root = save_tool_root(config_dir);
    if root.exists() {
        tokio::fs::remove_dir_all(&root).await?;
    }
    Ok(())
}

fn unpack(archive: &Path, into: &Path) -> CoreResult<()> {
    let file = std::fs::File::open(archive)?;

    if archive.extension().and_then(|e| e.to_str()) == Some("zip") {
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| CoreError::Other(format!("the download is not a zip: {e}")))?;
        zip.extract(into)
            .map_err(|e| CoreError::Other(format!("could not unpack the save helper: {e}")))?;
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        tar.unpack(into)?;
    }
    Ok(())
}

/// The helper executable in the unpacked tree, searched rather than assumed: the layout is
/// upstream's to change, and a wrong guess surfaces much later.
fn find_binary(root: &Path) -> CoreResult<PathBuf> {
    let wanted = binary_name();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(wanted) {
                return Ok(path);
            }
        }
    }
    Err(CoreError::Other(
        "the download did not contain the save helper".into(),
    ))
}

fn sha256_of(path: &Path) -> CoreResult<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
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
    use super::*;

    fn release(version: &str) -> SaveToolRelease {
        SaveToolRelease {
            version: version.into(),
            asset: "x".into(),
            url: "https://example.invalid/x".into(),
            size_bytes: 1,
            sha256: None,
        }
    }

    fn status(
        installed: Option<&str>,
        bundled: Option<&str>,
        latest: Option<&str>,
    ) -> SaveToolStatus {
        SaveToolStatus {
            manifest: None,
            installed: installed.map(|v| InstalledSaveTool {
                version: v.into(),
                binary: PathBuf::from("/tmp/ludusavi"),
            }),
            bundled: bundled.map(str::to_string),
            latest: latest.map(release),
            available: Vec::new(),
        }
    }

    #[test]
    fn the_installed_copy_runs_when_there_is_one() {
        assert_eq!(
            Some("v0.31.0"),
            status(Some("v0.31.0"), Some("v0.30.0"), None).in_use()
        );
        assert_eq!(
            Some("v0.30.0"),
            status(None, Some("v0.30.0"), None).in_use()
        );
    }

    #[test]
    fn an_update_is_offered_only_when_a_newer_release_is_known() {
        for (installed, bundled, latest, expected, why) in [
            (
                None,
                Some("v0.30.0"),
                Some("v0.31.0"),
                true,
                "bundled is behind",
            ),
            (
                Some("v0.30.0"),
                Some("v0.30.0"),
                Some("v0.31.0"),
                true,
                "installed is behind",
            ),
            (
                Some("v0.31.0"),
                Some("v0.30.0"),
                Some("v0.31.0"),
                false,
                "already running the newest",
            ),
            (
                None,
                Some("v0.31.0"),
                Some("v0.31.0"),
                false,
                "bundled is newest",
            ),
            // Offline is not the same as current; claiming otherwise hides an update.
            (
                Some("v0.30.0"),
                None,
                None,
                false,
                "no feed to compare against",
            ),
        ] {
            assert_eq!(
                status(installed, bundled, latest).update_available(),
                expected,
                "{why}"
            );
        }
    }

    #[test]
    fn the_asset_name_matches_what_upstream_publishes() {
        let name = asset_for("v0.31.0").unwrap();
        assert!(name.starts_with("ludusavi-v0.31.0-"), "{name}");
        if cfg!(windows) {
            assert!(name.ends_with("win64.zip"), "{name}");
        } else if cfg!(target_os = "macos") {
            assert!(name.ends_with("mac.tar.gz"), "{name}");
        } else {
            assert!(name.ends_with("linux.tar.gz"), "{name}");
        }
    }

    #[test]
    fn the_checksum_the_install_compares_is_the_file_s_own() {
        let dir = std::env::temp_dir().join(format!("gameyfin-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("archive");
        std::fs::write(&file, b"ludusavi").unwrap();

        // From `printf ludusavi | sha256sum`, so a bug in the hashing here cannot agree
        // with itself and pass.
        let actual = sha256_of(&file).unwrap();
        assert_eq!(
            actual,
            "73b491e0ae51ddab5b56c18b8a0056e638289689fe0ef2a8b530297ec41e9ab7"
        );

        // A single changed byte must not still match, or the check protects nothing.
        std::fs::write(&file, b"ludusavj").unwrap();
        assert_ne!(sha256_of(&file).unwrap(), actual);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_half_finished_install_reports_as_absent() {
        let dir = std::env::temp_dir().join(format!("gameyfin-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(save_tool_root(&dir)).unwrap();
        // A marker naming a binary that was never unpacked.
        std::fs::write(
            save_tool_root(&dir).join(MARKER),
            serde_json::to_string(&InstalledSaveTool {
                version: "v0.31.0".into(),
                binary: dir.join("nothing-here"),
            })
            .unwrap(),
        )
        .unwrap();

        assert!(installed(&dir).is_none());
    }
}
