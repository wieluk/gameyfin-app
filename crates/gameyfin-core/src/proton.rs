//! Proton builds Gameyfin downloads and hands to umu by path. Managed here rather than left
//! to umu, so a first launch shows a download instead of stalling silently for minutes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Where a download unpacks before it is renamed into place; a dot name keeps it out of
/// every listing, so a half-extracted build never looks installed.
const UNPACKING: &str = ".unpacking";

/// GitHub page size. Both projects release a few times a month, so this reaches months back.
const FEED_PAGE: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ProtonFamily {
    /// Valve's Proton with umu's protonfixes. The default, and what umu itself would pick.
    UmuProton,
    /// GloriousEggroll's build, with more media codecs and game patches.
    GeProton,
}

impl ProtonFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            ProtonFamily::UmuProton => "umu-proton",
            ProtonFamily::GeProton => "ge-proton",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProtonFamily::UmuProton => "UMU-Proton",
            ProtonFamily::GeProton => "GE-Proton",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "umu-proton" => Some(ProtonFamily::UmuProton),
            "ge-proton" => Some(ProtonFamily::GeProton),
            _ => None,
        }
    }

    fn releases_api(self) -> &'static str {
        match self {
            ProtonFamily::UmuProton => {
                "https://api.github.com/repos/Open-Wine-Components/umu-proton/releases"
            }
            ProtonFamily::GeProton => {
                "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases"
            }
        }
    }

    /// The family a build belongs to, from its tag or the directory it unpacks as. The
    /// umu-proton feed still carries its `ULWGL-Proton` predecessors, which match neither.
    pub fn of_name(name: &str) -> Option<Self> {
        if name.starts_with("UMU-Proton-") {
            Some(ProtonFamily::UmuProton)
        } else if name.starts_with("GE-Proton") {
            Some(ProtonFamily::GeProton)
        } else {
            None
        }
    }

    /// Asset names a release may use, preferred first. GE-Proton added an architecture
    /// suffix once it published aarch64 builds; its older releases are x86_64 without one.
    fn asset_candidates(self, tag: &str) -> Vec<String> {
        match self {
            ProtonFamily::UmuProton => vec![format!("{tag}.tar.gz")],
            ProtonFamily::GeProton => {
                vec![format!("{tag}-x86_64.tar.gz"), format!("{tag}.tar.gz")]
            }
        }
    }
}

/// Tags that are not releases, such as umu-proton's betas.
pub fn is_prerelease_tag(tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    ["beta", "-rc"].iter().any(|marker| lower.contains(marker))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProtonRelease {
    pub family: ProtonFamily,
    pub tag: String,
    pub asset: String,
    pub url: String,
    /// The sibling `.sha512sum`. `None` when a release omits it, which is logged rather
    /// than silently accepted.
    pub sums_url: Option<String>,
    pub size_bytes: u64,
}

/// A build on disk, ready to hand to umu as `PROTONPATH`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledProton {
    /// The directory name, which is also how a game pins a build.
    pub name: String,
    pub family: Option<ProtonFamily>,
    /// The build directory, holding the `proton` script.
    pub path: PathBuf,
}

pub fn proton_root(config_dir: &Path) -> PathBuf {
    config_dir.join("proton")
}

/// Builds Gameyfin downloaded, newest first. A build counts only while its `proton` script exists.
pub fn installed(config_dir: &Path) -> Vec<InstalledProton> {
    let Ok(entries) = std::fs::read_dir(proton_root(config_dir)) else {
        return Vec::new();
    };

    let mut builds: Vec<InstalledProton> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            (!name.starts_with('.') && path.join("proton").is_file()).then(|| InstalledProton {
                family: ProtonFamily::of_name(&name),
                name,
                path,
            })
        })
        .collect();
    sort_newest_first(&mut builds);
    builds
}

/// The build a game runs. A pin to a replaced build follows the newest of its family, and no
/// pin means the newest UMU-Proton.
pub fn pick(all: &[InstalledProton], wanted: Option<&str>) -> Option<InstalledProton> {
    let newest_of = |family| all.iter().find(|b| b.family == Some(family));
    wanted
        .and_then(|name| {
            all.iter()
                .find(|b| b.name == name)
                .or_else(|| ProtonFamily::of_name(name).and_then(newest_of))
        })
        .or_else(|| newest_of(ProtonFamily::UmuProton))
        .or_else(|| all.first())
        .cloned()
}

fn sort_newest_first(builds: &mut [InstalledProton]) {
    builds.sort_by_key(|b| std::cmp::Reverse(version_key(&b.name)));
}

/// Digit groups of a build name, so `GE-Proton11-6` sorts above `GE-Proton10-34`. The
/// architecture suffix is dropped first, or its `86` and `64` would count as versions.
fn version_key(name: &str) -> Vec<u64> {
    name.trim_end_matches("-x86_64")
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Releases that publish an x86_64 build, newest first.
pub async fn releases(
    http: &reqwest::Client,
    family: ProtonFamily,
) -> CoreResult<Vec<ProtonRelease>> {
    // GitHub answers 403 without a user agent, which reads as a permissions problem.
    let feed: Vec<serde_json::Value> = http
        .get(format!("{}?per_page={FEED_PAGE}", family.releases_api()))
        .header(reqwest::header::USER_AGENT, "Gameyfin-App")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(feed
        .iter()
        .filter_map(|entry| release_from(family, entry))
        .collect())
}

/// The newest release that is not a beta.
pub async fn latest_release(
    http: &reqwest::Client,
    family: ProtonFamily,
) -> CoreResult<ProtonRelease> {
    releases(http, family)
        .await?
        .into_iter()
        .find(|r| !is_prerelease_tag(&r.tag))
        .ok_or_else(|| CoreError::Other(format!("no {} release was found", family.label())))
}

/// One feed entry, if it is a release of this family with an x86_64 build.
fn release_from(family: ProtonFamily, entry: &serde_json::Value) -> Option<ProtonRelease> {
    if entry.get("prerelease").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }
    let tag = entry.get("tag_name")?.as_str()?;
    if ProtonFamily::of_name(tag) != Some(family) {
        return None;
    }

    let assets = entry.get("assets")?.as_array()?;
    let named = |wanted: &str| {
        assets
            .iter()
            .find(|a| a.get("name").and_then(serde_json::Value::as_str) == Some(wanted))
    };

    // Matched on exact names, so an aarch64-only release is skipped rather than downloaded.
    let (asset_name, asset) = family
        .asset_candidates(tag)
        .into_iter()
        .find_map(|wanted| named(&wanted).map(|asset| (wanted, asset)))?;

    let sums_name = format!("{}.sha512sum", asset_name.trim_end_matches(".tar.gz"));
    let url_of = |a: &serde_json::Value| {
        a.get("browser_download_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };

    Some(ProtonRelease {
        family,
        tag: tag.to_string(),
        url: url_of(asset)?,
        sums_url: named(&sums_name).and_then(url_of),
        size_bytes: asset
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        asset: asset_name,
    })
}

/// Download, verify and unpack a build, replacing the older build of its family.
pub async fn install<F>(
    config_dir: &Path,
    http: &reqwest::Client,
    release: &ProtonRelease,
    downloader: &crate::download::Downloader,
    on_progress: F,
) -> CoreResult<InstalledProton>
where
    F: FnMut(crate::download::Progress) + Send,
{
    let root = proton_root(config_dir);
    std::fs::create_dir_all(&root)?;

    let archive = root.join(&release.asset);
    downloader
        .download(&release.url, &archive, |r| r, on_progress)
        .await?;

    match &release.sums_url {
        Some(url) => {
            let sums = http
                .get(url)
                .header(reqwest::header::USER_AGENT, "Gameyfin-App")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let expected = crate::wine::sha_for(&sums, &release.asset).ok_or_else(|| {
                CoreError::Other(format!("the checksum file does not name {}", release.asset))
            })?;
            let actual = sha512_of(&archive).await?;
            if !actual.eq_ignore_ascii_case(&expected) {
                // A kept partial would be resumed onto by the next attempt.
                let _ = std::fs::remove_file(&archive);
                return Err(CoreError::Other(format!(
                    "the downloaded {} did not match its checksum, so it was discarded",
                    release.tag
                )));
            }
        }
        None => tracing::warn!(tag = %release.tag, "no checksum published; not verified"),
    }

    let staging = root.join(UNPACKING);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let unpack_from = archive.clone();
    let unpack_to = staging.clone();
    let build = tokio::task::spawn_blocking(move || {
        crate::extract::extract(&unpack_from, &unpack_to, |_| {})?;
        find_build(&unpack_to)
            .ok_or_else(|| CoreError::Other("the archive held no proton script".into()))
    })
    .await
    .map_err(|e| CoreError::Other(format!("unpacking did not finish: {e}")))??;

    let name = build
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| release.tag.clone());
    let destination = root.join(&name);
    let _ = std::fs::remove_dir_all(&destination);
    std::fs::rename(&build, &destination)?;
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);

    // One build per family: games pinned to the old one follow it to the new one.
    for old in installed(config_dir)
        .into_iter()
        .filter(|b| b.family == Some(release.family) && b.name != name)
    {
        let _ = std::fs::remove_dir_all(&old.path);
    }

    Ok(InstalledProton {
        family: Some(release.family),
        name,
        path: destination,
    })
}

/// The unpacked build: the directory holding the `proton` script, at the top or one down.
fn find_build(root: &Path) -> Option<PathBuf> {
    if root.join("proton").is_file() {
        return Some(root.to_path_buf());
    }
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("proton").is_file())
}

async fn sha512_of(path: &Path) -> CoreResult<String> {
    use sha2::{Digest, Sha512};
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha512::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Delete one managed build. Prefixes that used it switch to the default on next launch.
pub async fn remove(config_dir: &Path, name: &str) -> CoreResult<()> {
    // Only a plain directory name: a crafted value must not reach outside the Proton root.
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return Err(CoreError::Other(format!("{name:?} is not a Proton build")));
    }
    let path = proton_root(config_dir).join(name);
    if path.is_dir() {
        tokio::fs::remove_dir_all(&path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-proton-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(tag: &str, assets: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "prerelease": false,
            "assets": assets.iter().map(|name| serde_json::json!({
                "name": name,
                "browser_download_url": format!("https://example.test/{name}"),
                "size": 1024,
            })).collect::<Vec<_>>(),
        })
    }

    fn build(name: &str) -> InstalledProton {
        InstalledProton {
            name: name.to_string(),
            family: ProtonFamily::of_name(name),
            path: PathBuf::from("/builds").join(name),
        }
    }

    #[test]
    fn a_ge_release_takes_the_x86_64_build_and_never_the_aarch64_one() {
        let release = release_from(
            ProtonFamily::GeProton,
            &entry(
                "GE-Proton11-6",
                &[
                    "GE-Proton11-6-aarch64.sha512sum",
                    "GE-Proton11-6-aarch64.tar.gz",
                    "GE-Proton11-6-x86_64.sha512sum",
                    "GE-Proton11-6-x86_64.tar.gz",
                ],
            ),
        )
        .unwrap();
        assert_eq!(release.asset, "GE-Proton11-6-x86_64.tar.gz");
        assert_eq!(
            release.sums_url.as_deref(),
            Some("https://example.test/GE-Proton11-6-x86_64.sha512sum")
        );
    }

    #[test]
    fn an_older_ge_release_without_an_architecture_suffix_is_still_found() {
        let release = release_from(
            ProtonFamily::GeProton,
            &entry(
                "GE-Proton10-34",
                &["GE-Proton10-34.sha512sum", "GE-Proton10-34.tar.gz"],
            ),
        )
        .unwrap();
        assert_eq!(release.asset, "GE-Proton10-34.tar.gz");
        assert!(release
            .sums_url
            .unwrap()
            .ends_with("GE-Proton10-34.sha512sum"));
    }

    #[test]
    fn a_release_that_only_publishes_aarch64_is_skipped() {
        let aarch64_only = entry(
            "GE-Proton12-1",
            &[
                "GE-Proton12-1-aarch64.tar.gz",
                "GE-Proton12-1-aarch64.sha512sum",
            ],
        );
        assert_eq!(release_from(ProtonFamily::GeProton, &aarch64_only), None);
    }

    #[test]
    fn the_umu_feed_drops_its_ulwgl_predecessors_and_marks_betas() {
        let ulwgl = entry("ULWGL-Proton-8.0-5-3", &["ULWGL-Proton-8.0-5-3.tar.gz"]);
        assert_eq!(release_from(ProtonFamily::UmuProton, &ulwgl), None);

        let current = entry(
            "UMU-Proton-10.0-4",
            &["UMU-Proton-10.0-4.sha512sum", "UMU-Proton-10.0-4.tar.gz"],
        );
        assert!(release_from(ProtonFamily::UmuProton, &current).is_some());
        // A GE tag in the umu feed would be a family mix-up, not a release.
        assert_eq!(release_from(ProtonFamily::GeProton, &current), None);

        assert!(is_prerelease_tag("UMU-Proton-9.0-beta16-2"));
        assert!(!is_prerelease_tag("UMU-Proton-9.0-4e"));
    }

    #[test]
    fn builds_sort_newest_first_across_numbering_and_suffix_changes() {
        let mut builds = vec![
            build("GE-Proton9-27"),
            build("GE-Proton11-6-x86_64"),
            build("GE-Proton10-34"),
            build("GE-Proton11-3"),
        ];
        sort_newest_first(&mut builds);
        let names: Vec<&str> = builds.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "GE-Proton11-6-x86_64",
                "GE-Proton11-3",
                "GE-Proton10-34",
                "GE-Proton9-27"
            ]
        );

        assert!(version_key("UMU-Proton-10.0-4") > version_key("UMU-Proton-9.0-4e"));
    }

    #[test]
    fn only_directories_holding_a_proton_script_count_as_installed() {
        let dir = scratch("installed");
        let root = proton_root(&dir);
        for name in ["UMU-Proton-10.0-4", "GE-Proton11-6-x86_64", UNPACKING] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("proton"), "#!/usr/bin/env python3").unwrap();
        }
        // Interrupted before the script landed.
        std::fs::create_dir_all(root.join("UMU-Proton-9.0-3")).unwrap();

        let mut names: Vec<String> = installed(&dir).into_iter().map(|b| b.name).collect();
        names.sort();
        assert_eq!(names, ["GE-Proton11-6-x86_64", "UMU-Proton-10.0-4"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_default_is_the_newest_umu_proton_even_beside_a_newer_ge() {
        let all = vec![
            build("GE-Proton11-6-x86_64"),
            build("UMU-Proton-10.0-4"),
            build("UMU-Proton-9.0-3"),
        ];
        assert_eq!(pick(&all, None).unwrap().name, "UMU-Proton-10.0-4");
    }

    #[test]
    fn a_pin_to_a_replaced_build_follows_its_family() {
        let all = vec![build("UMU-Proton-10.0-4"), build("GE-Proton11-6-x86_64")];
        assert_eq!(
            pick(&all, Some("GE-Proton11-6-x86_64")).unwrap().name,
            "GE-Proton11-6-x86_64"
        );
        assert_eq!(
            pick(&all, Some("GE-Proton10-1")).unwrap().name,
            "GE-Proton11-6-x86_64"
        );
        let umu_only = vec![build("UMU-Proton-10.0-4")];
        assert_eq!(
            pick(&umu_only, Some("GE-Proton10-1")).unwrap().name,
            "UMU-Proton-10.0-4"
        );
    }

    #[test]
    fn the_unpacked_build_is_found_under_its_own_top_directory() {
        let dir = scratch("find");
        let nested = dir.join("UMU-Proton-10.0-4");
        std::fs::create_dir_all(nested.join("files")).unwrap();
        std::fs::write(nested.join("proton"), "").unwrap();
        assert_eq!(find_build(&dir), Some(nested));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_build_name_that_would_escape_the_proton_root_is_refused() {
        let dir = scratch("remove");
        for name in ["../victim", "", ".unpacking", "a/b"] {
            assert!(remove(&dir, name).await.is_err(), "{name:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
