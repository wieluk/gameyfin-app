//! Gameyfin's own Wine, the fallback for what Proton cannot run on this system.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CoreError, CoreResult};

/// Kron4ek's self-contained Wine-Staging builds, as Lutris and Bottles use. Listed rather than
/// `latest`, which is almost always a development release.
const RELEASES_LIST_API: &str = "https://api.github.com/repos/Kron4ek/Wine-Builds/releases";

/// GitHub's page limit. Wine ships about 25 releases a year, so this reaches two stables back.
const FEED_PAGE: usize = 100;

/// Wine-Staging with new WoW64, which needs no 32-bit libraries on the host.
const ASSET_SUFFIX: &str = "staging-amd64-wow64";

/// Marker recording what is installed, so the version is known without unpacking.
const MARKER: &str = ".gameyfin-wine.json";

/// Directory the extracted build lives in, inside the Wine root.
const CURRENT: &str = "current";

/// The feed also carries Kron4ek's `proton-*` builds, which are not Wine's own releases.
fn is_wine_tag(tag: &str) -> bool {
    !tag.starts_with("proton-")
}

/// Whether a version is a stable release: `x.0` and its maintenance releases `x.0.y`.
fn is_stable(version: &str) -> bool {
    let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let mut parts = version.split('.');
    parts.next().is_some_and(numeric) && parts.next() == Some("0") && parts.all(numeric)
}

fn asset_name(version: &str) -> String {
    format!("wine-{version}-{ASSET_SUFFIX}.tar.xz")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WineRelease {
    pub version: String,
    pub asset: String,
    pub url: String,
    /// From the release's `sha256sums.txt`, `None` when that file is absent.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledWine {
    pub version: String,
    pub binary: PathBuf,
}

pub fn wine_root(config_dir: &Path) -> PathBuf {
    config_dir.join("wine")
}

/// The installed build, trusting the marker only as far as the binary it names.
pub fn installed(config_dir: &Path) -> Option<InstalledWine> {
    let root = wine_root(config_dir);
    let marker: InstalledWine =
        serde_json::from_str(&std::fs::read_to_string(root.join(MARKER)).ok()?).ok()?;
    marker.binary.is_file().then_some(marker)
}

/// The `wine` binary inside an extracted build. `files/bin` is Proton's layout, probed so a
/// build that unpacks that way is not mistaken for one with no binary.
fn binary_in(dir: &Path) -> PathBuf {
    let nested = dir.join("files").join("bin").join("wine");
    if nested.is_file() {
        return nested;
    }
    dir.join("bin").join("wine")
}

/// The newest stable release that publishes the WoW64 build.
pub async fn latest_release(http: &reqwest::Client) -> CoreResult<WineRelease> {
    let feed: Vec<serde_json::Value> = http
        .get(format!("{RELEASES_LIST_API}?per_page={FEED_PAGE}"))
        .header(reqwest::header::USER_AGENT, "Gameyfin-App")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let version = feed
        .iter()
        .find_map(|entry| {
            let version = entry.get("tag_name")?.as_str()?;
            let wanted = asset_name(version);
            let has_build = entry.get("assets")?.as_array()?.iter().any(|a| {
                a.get("name").and_then(serde_json::Value::as_str) == Some(wanted.as_str())
            });
            (is_wine_tag(version) && is_stable(version) && has_build).then(|| version.to_string())
        })
        .ok_or_else(|| {
            CoreError::Other("no recent stable Wine release has a WoW64 build".into())
        })?;
    release_for(http, &version).await
}

async fn release_for(http: &reqwest::Client, version: &str) -> CoreResult<WineRelease> {
    // GitHub answers 403 without a user agent, which reads as a permissions problem.
    let response: serde_json::Value = http
        .get(format!("{RELEASES_LIST_API}/tags/{version}"))
        .header(reqwest::header::USER_AGENT, "Gameyfin-App")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let assets = response
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CoreError::Other("the Wine release feed listed no builds".into()))?;
    let named = |name: &str| {
        assets
            .iter()
            .find(|a| a.get("name").and_then(serde_json::Value::as_str) == Some(name))
    };

    let wanted = asset_name(version);
    let url = named(&wanted)
        .and_then(|a| a.get("browser_download_url"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CoreError::Other(format!("Wine {version} has no WoW64 build")))?
        .to_string();

    let mut sha256 = None;
    if let Some(sums_url) = named("sha256sums.txt")
        .and_then(|a| a.get("browser_download_url"))
        .and_then(serde_json::Value::as_str)
    {
        let fetched = http
            .get(sums_url)
            .header(reqwest::header::USER_AGENT, "Gameyfin-App")
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
        version: version.to_string(),
        asset: wanted,
        url,
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

/// Download, verify and unpack a build, swapped into place only on success.
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
            // A kept file would be resumed onto by the next attempt.
            let _ = tokio::fs::remove_file(&archive).await;
            return Err(CoreError::Other(
                "the downloaded Wine build did not match its checksum, so it was discarded".into(),
            ));
        }
    }

    let staging = root.join(".unpacking");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await?;

    let (from, into) = (archive.clone(), staging.clone());
    let unpacked = tokio::task::spawn_blocking(move || unpack(&from, &into))
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

/// Extract a `.tar.xz` and return its single top-level directory.
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
    // Guessing which of several directories is Wine would install something arbitrary.
    if entries.next().is_some() {
        return Err(CoreError::Other(
            "the Wine build had an unexpected layout".into(),
        ));
    }
    Ok(first)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_stable_wine_releases_are_used() {
        for stable in ["11.0", "10.0", "8.0.2"] {
            assert!(is_stable(stable), "{stable}");
        }
        for development in ["11.17", "11.1", "11.0-rc3", "proton-11.0-2", "", "11"] {
            assert!(!is_stable(development), "{development}");
        }
        assert!(!is_wine_tag("proton-11.0-2"));
        assert!(is_wine_tag("11.0"));
    }

    #[test]
    fn a_checksum_is_matched_to_its_own_file() {
        let sums = "aaa  wine-11.0-staging-amd64.tar.xz\n\
                    bbb  wine-11.0-staging-amd64-wow64.tar.xz\n";
        // The classic build's name is a prefix of the WoW64 one.
        assert_eq!(sha_for(sums, &asset_name("11.0")), Some("bbb".into()));
        assert_eq!(
            sha_for(sums, "wine-11.0-staging-amd64.tar.xz"),
            Some("aaa".into())
        );
        assert_eq!(sha_for(sums, "not-here.tar.xz"), None);
        assert_eq!(
            sha_for("ccc *wine.tar.xz", "wine.tar.xz"),
            Some("ccc".into())
        );
    }

    #[test]
    fn a_missing_binary_means_nothing_is_installed() {
        let dir = std::env::temp_dir().join(format!("gameyfin-wine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = wine_root(&dir);
        std::fs::create_dir_all(&root).unwrap();

        let record = InstalledWine {
            version: "11.0".into(),
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
