//! A Wine runtime the app downloads and owns. One runtime and version across every package
//! format, with no `flatpak-spawn` boundary for settings to fail to cross, unlike the
//! host's Wine (absent in a Flatpak sandbox, an rpm-layer-and-reboot on atomic distros).
//!
//! The default is the WoW64 build: it translates 32-bit Windows calls inside a 64-bit
//! process and needs no 32-bit host libraries, so one tarball behaves the same everywhere.
//! [`WineVariant`] lets a user switch to the classic build for the minority of 32-bit
//! programs that still misbehave under WoW64.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CoreError, CoreResult};

/// Where the builds come from: Kron4ek's self-contained Wine-Staging builds, as used by
/// Lutris and Bottles, published per release with checksums.
const RELEASES_API: &str = "https://api.github.com/repos/Kron4ek/Wine-Builds/releases/latest";

/// Marker recording what is installed, so the version can be shown without unpacking.
const MARKER: &str = ".gameyfin-wine.json";

/// Directory the extracted build lives in, inside the Wine root.
const CURRENT: &str = "current";

/// Which build to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WineVariant {
    /// Wine-Staging with new WoW64. Needs no 32-bit host libraries, so it works the same
    /// in every package format. The default, and the only variant that meets that bar.
    #[default]
    StagingWow64,
    /// Wine-Staging with classic WoW64. Better tested for 32-bit programs, but needs
    /// 32-bit host libraries, which a Flatpak sandbox does not have unless the i386
    /// compatibility extension is installed alongside it.
    Staging,
}

impl WineVariant {
    /// The part of the asset filename that identifies this build.
    fn asset_suffix(self) -> &'static str {
        match self {
            WineVariant::StagingWow64 => "staging-amd64-wow64",
            WineVariant::Staging => "staging-amd64",
        }
    }

    /// What to show the user.
    pub fn label(self) -> &'static str {
        match self {
            WineVariant::StagingWow64 => "Wine-Staging (WoW64)",
            WineVariant::Staging => "Wine-Staging (32-bit libraries)",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WineVariant::StagingWow64 => "staging-wow64",
            WineVariant::Staging => "staging",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "staging" => WineVariant::Staging,
            _ => WineVariant::StagingWow64,
        }
    }
}

/// A build available to download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WineRelease {
    pub version: String,
    pub variant: WineVariant,
    pub asset: String,
    pub url: String,
    pub size_bytes: u64,
    /// From the release's `sha256sums.txt`. `None` when that file is absent, which is
    /// reported rather than silently accepted.
    pub sha256: Option<String>,
}

/// A build already on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledWine {
    pub version: String,
    pub variant: WineVariant,
    /// The `wine` binary itself, ready to run.
    pub binary: PathBuf,
}

/// Everything the settings screen needs to describe the Wine situation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WineStatus {
    pub installed: Option<InstalledWine>,
    /// `None` when the release feed could not be reached; the UI says so rather than
    /// claiming the installed build is current.
    pub latest: Option<WineRelease>,
}

impl WineStatus {
    /// Whether a newer build than the installed one is available.
    ///
    /// A variant change counts: switching to the classic build is an update to perform,
    /// not a version comparison.
    pub fn update_available(&self) -> bool {
        match (&self.installed, &self.latest) {
            (Some(installed), Some(latest)) => {
                installed.version != latest.version || installed.variant != latest.variant
            }
            _ => false,
        }
    }
}

/// Root of everything this module owns.
pub fn wine_root(config_dir: &Path) -> PathBuf {
    config_dir.join("wine")
}

/// The build currently installed, if any.
///
/// The marker is trusted only as far as the binary it names: a half-extracted or manually
/// deleted install must report as absent, not as a Wine that fails on first use.
pub fn installed(config_dir: &Path) -> Option<InstalledWine> {
    let root = wine_root(config_dir);
    let marker: InstalledWine =
        serde_json::from_str(&std::fs::read_to_string(root.join(MARKER)).ok()?).ok()?;
    marker.binary.is_file().then_some(marker)
}

/// The `wine` binary inside an extracted build.
fn binary_in(dir: &Path) -> PathBuf {
    dir.join("bin").join("wine")
}

/// Look up the latest published build for a variant.
pub async fn latest_release(
    http: &reqwest::Client,
    variant: WineVariant,
) -> CoreResult<WineRelease> {
    // GitHub rejects API requests without a user agent, with a 403 that looks like a
    // permissions problem rather than a missing header.
    let response: serde_json::Value = http
        .get(RELEASES_API)
        .header(reqwest::header::USER_AGENT, "Gameyfin-Desktop")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let version = response
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CoreError::Other("the Wine release feed had no version".into()))?
        .to_string();

    let assets = response
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CoreError::Other("the Wine release feed listed no builds".into()))?;

    // Matched on the exact filename rather than a substring: `staging-amd64` is a prefix
    // of `staging-amd64-wow64`, so a contains-check would happily pick the wrong build.
    let wanted = format!("wine-{version}-{}.tar.xz", variant.asset_suffix());
    let asset = assets
        .iter()
        .find(|a| a.get("name").and_then(serde_json::Value::as_str) == Some(wanted.as_str()))
        .ok_or_else(|| {
            CoreError::Other(format!(
                "Wine {version} does not publish a {} build",
                variant.label()
            ))
        })?;

    let url = asset
        .get("browser_download_url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CoreError::Other("the Wine build had no download link".into()))?
        .to_string();
    let size_bytes = asset
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    // Checksums live in a sibling asset, one `<sha256>  <filename>` per line.
    let sums_url = assets
        .iter()
        .find(|a| a.get("name").and_then(serde_json::Value::as_str) == Some("sha256sums.txt"))
        .and_then(|a| a.get("browser_download_url"))
        .and_then(serde_json::Value::as_str);

    // A missing or unreadable checksum file is reported as `None` rather than failing the
    // lookup, so a release that omits it can still be installed over HTTPS. `install`
    // verifies whenever a sum is present and skips verification when it is not, and
    // `install_wine` logs which of the two happened.
    let mut sha256 = None;
    if let Some(url) = sums_url {
        let fetched = http
            .get(url)
            .header(reqwest::header::USER_AGENT, "Gameyfin-Desktop")
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        if let Ok(response) = fetched {
            if let Ok(text) = response.text().await {
                sha256 = sha_for(&text, &wanted);
            }
        }
    }

    Ok(WineRelease {
        version,
        variant,
        asset: wanted,
        url,
        size_bytes,
        sha256,
    })
}

/// Pull one file's checksum out of a `sha256sums.txt`.
pub fn sha_for(sums: &str, filename: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let sum = parts.next()?;
        // The name may be prefixed with `*` for binary mode.
        let name = parts.next()?.trim_start_matches('*');
        (name == filename).then(|| sum.to_ascii_lowercase())
    })
}

/// Download, verify and unpack a build. Extraction is swapped into place only on success,
/// so an interrupted update leaves the previous Wine working.
pub async fn install<F>(
    config_dir: &Path,
    release: &WineRelease,
    downloader: &crate::download::Downloader,
    on_progress: F,
) -> CoreResult<InstalledWine>
where
    F: FnMut(crate::download::Progress) + Send,
{
    let root = wine_root(config_dir);
    tokio::fs::create_dir_all(&root).await?;

    let archive = root.join(&release.asset);
    downloader
        .download(&release.url, &archive, |r| r, on_progress)
        .await?;

    if let Some(expected) = &release.sha256 {
        let actual = sha256_of(&archive).await?;
        if !actual.eq_ignore_ascii_case(expected) {
            // Removed rather than kept: a resumable downloader would otherwise treat the
            // corrupt file as a partial transfer and resume onto the end of it forever.
            let _ = tokio::fs::remove_file(&archive).await;
            return Err(CoreError::Other(
                "the downloaded Wine build did not match its checksum, so it was discarded".into(),
            ));
        }
    }

    let staging = root.join(".unpacking");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await?;

    let archive_for_task = archive.clone();
    let staging_for_task = staging.clone();
    // Decompression is CPU-bound and blocking; on the async runtime it would stall every
    // other task for the duration.
    let unpacked =
        tokio::task::spawn_blocking(move || unpack(&archive_for_task, &staging_for_task))
            .await
            .map_err(|e| CoreError::Other(format!("could not unpack Wine: {e}")))??;

    let destination = root.join(CURRENT);
    let _ = tokio::fs::remove_dir_all(&destination).await;
    tokio::fs::rename(&unpacked, &destination).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let _ = tokio::fs::remove_file(&archive).await;

    let binary = binary_in(&destination);
    if !binary.is_file() {
        return Err(CoreError::Other(
            "the Wine build unpacked without a wine binary in it".into(),
        ));
    }

    let record = InstalledWine {
        version: release.version.clone(),
        variant: release.variant,
        binary,
    };
    tokio::fs::write(
        root.join(MARKER),
        serde_json::to_string_pretty(&record)
            .map_err(|e| CoreError::Other(format!("could not record the Wine version: {e}")))?,
    )
    .await?;

    Ok(record)
}

/// Extract a `.tar.xz`, returning the build's own single top-level directory inside it so
/// the caller can rename it into place rather than nesting it a level deeper.
fn unpack(archive: &Path, into: &Path) -> CoreResult<PathBuf> {
    let file = std::fs::File::open(archive)?;
    let decompressed = liblzma::read::XzDecoder::new(std::io::BufReader::new(file));
    tar::Archive::new(decompressed).unpack(into)?;

    let mut entries = std::fs::read_dir(into)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir());

    let first = entries
        .next()
        .ok_or_else(|| CoreError::Other("the Wine build unpacked to nothing".into()))?;

    // More than one top-level directory means the layout is not what this expects, and
    // guessing which one is Wine would install something arbitrary.
    if entries.next().is_some() {
        return Err(CoreError::Other(
            "the Wine build had an unexpected layout".into(),
        ));
    }
    Ok(first)
}

/// SHA-256 of a file, read in chunks so a 100 MB archive is not held in memory.
async fn sha256_of(path: &Path) -> CoreResult<String> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Delete the downloaded Wine entirely.
pub async fn remove(config_dir: &Path) -> CoreResult<()> {
    let root = wine_root(config_dir);
    if root.exists() {
        tokio::fs::remove_dir_all(&root).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wow64_build_is_the_default() {
        // It is the only variant that needs no 32-bit host libraries, and therefore the
        // only one that behaves the same in a Flatpak as in a deb.
        assert_eq!(WineVariant::default(), WineVariant::StagingWow64);
        assert_eq!(WineVariant::default().asset_suffix(), "staging-amd64-wow64");
    }

    #[test]
    fn variants_round_trip_through_their_string_form() {
        for variant in [WineVariant::StagingWow64, WineVariant::Staging] {
            assert_eq!(WineVariant::parse(variant.as_str()), variant);
        }
        // Anything unrecognised falls back to the safe default rather than failing to
        // load the whole settings file.
        assert_eq!(WineVariant::parse("nonsense"), WineVariant::StagingWow64);
    }

    #[test]
    fn a_checksum_is_matched_to_its_own_file() {
        let sums = "aaa  wine-11.17-staging-amd64.tar.xz\n\
                    bbb  wine-11.17-staging-amd64-wow64.tar.xz\n";

        // The classic build's name is a prefix of the WoW64 one, so a sloppy match here
        // verifies the download against the wrong checksum and rejects a good file.
        assert_eq!(
            sha_for(sums, "wine-11.17-staging-amd64-wow64.tar.xz"),
            Some("bbb".into())
        );
        assert_eq!(
            sha_for(sums, "wine-11.17-staging-amd64.tar.xz"),
            Some("aaa".into())
        );
        assert_eq!(sha_for(sums, "not-here.tar.xz"), None);
    }

    #[test]
    fn a_binary_mode_checksum_line_is_understood() {
        assert_eq!(
            sha_for("ccc *wine.tar.xz", "wine.tar.xz"),
            Some("ccc".into())
        );
    }

    #[test]
    fn an_update_is_offered_for_a_new_version_or_a_changed_variant() {
        let release = |version: &str, variant| WineRelease {
            version: version.into(),
            variant,
            asset: "a".into(),
            url: "u".into(),
            size_bytes: 1,
            sha256: None,
        };
        let installed = |version: &str, variant| InstalledWine {
            version: version.into(),
            variant,
            binary: PathBuf::from("/wine"),
        };

        let same = WineStatus {
            installed: Some(installed("11.17", WineVariant::StagingWow64)),
            latest: Some(release("11.17", WineVariant::StagingWow64)),
        };
        assert!(!same.update_available());

        let newer = WineStatus {
            installed: Some(installed("11.16", WineVariant::StagingWow64)),
            latest: Some(release("11.17", WineVariant::StagingWow64)),
        };
        assert!(newer.update_available());

        // Switching variant is an install to perform even at the same version.
        let switched = WineStatus {
            installed: Some(installed("11.17", WineVariant::StagingWow64)),
            latest: Some(release("11.17", WineVariant::Staging)),
        };
        assert!(switched.update_available());

        // An unreachable feed must never read as "up to date".
        let offline = WineStatus {
            installed: Some(installed("11.17", WineVariant::StagingWow64)),
            latest: None,
        };
        assert!(!offline.update_available());
    }

    #[test]
    fn a_missing_binary_means_nothing_is_installed() {
        let dir = std::env::temp_dir().join(format!("gameyfin-wine-{}", std::process::id()));
        // Cleared first: a previous run that panicked leaves this behind, and the
        // leftover state is exactly what these assertions are checking for.
        let _ = std::fs::remove_dir_all(&dir);
        let root = wine_root(&dir);
        std::fs::create_dir_all(&root).unwrap();

        // A marker pointing at a binary that is not there: a deleted or half-extracted
        // install must report absent, not hand back a Wine that fails on first use.
        let record = InstalledWine {
            version: "11.17".into(),
            variant: WineVariant::StagingWow64,
            binary: root.join("current/bin/wine"),
        };
        std::fs::write(root.join(MARKER), serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(installed(&dir), None);

        std::fs::create_dir_all(root.join("current/bin")).unwrap();
        std::fs::write(root.join("current/bin/wine"), b"#!/bin/sh\n").unwrap();
        assert_eq!(installed(&dir), Some(record));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
