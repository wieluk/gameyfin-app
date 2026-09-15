//! Keeping the app up to date. What a package may do depends on who owns its files: Windows
//! replaces itself, a Flatpak asks the host, deb and rpm only report a release.

use serde::{Deserialize, Serialize};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// The release feed. A draft release is not published, so this only ever sees real ones.
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/wieluk/gameyfin-app/releases/latest";

/// Where a user is sent when their package cannot update itself.
const RELEASES_PAGE: &str = "https://github.com/wieluk/gameyfin-app/releases/latest";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Channel {
    /// Tauri's signed updater replaces the bundle in place. NSIS.
    SelfInstall,
    /// `flatpak update`, run on the host because we are inside the sandbox.
    Flatpak,
    /// apt or dnf owns the files; we can only report that a release exists.
    SystemPackage,
    /// Running from a build tree. Never offer an update.
    Development,
}

impl Channel {
    pub fn can_install(self) -> bool {
        matches!(self, Channel::SelfInstall | Channel::Flatpak)
    }
}

/// One Linux executable ships in four packages, so the environment says which.
pub fn detect_channel() -> Channel {
    // A debug build is a build tree by definition, whatever the surroundings look like.
    if cfg!(debug_assertions) {
        return Channel::Development;
    }
    if std::env::var_os("FLATPAK_ID").is_some() {
        return Channel::Flatpak;
    }
    if cfg!(windows) {
        return Channel::SelfInstall;
    }
    // Any other Linux release build is owned by apt or dnf, so it can only report a release.
    Channel::SystemPackage
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateStatus {
    pub current_version: String,
    /// The newest published release, or null when the check could not be made.
    pub latest_version: Option<String>,
    /// True only when the newest release is genuinely newer than what is running.
    pub available: bool,
    pub channel: Channel,
    pub can_install: bool,
    pub release_url: String,
    /// The release notes, trimmed to something a panel can show.
    pub notes: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateOutcome {
    pub message: String,
    /// True when the new version is on disk and only a restart stands between it and the user.
    pub restart_needed: bool,
}

impl UpdateOutcome {
    fn installed() -> Self {
        Self {
            message: "Updated. Restart Gameyfin to run the new version.".to_string(),
            restart_needed: true,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

/// Non-semver counts as "not newer", or a bad tag would nag forever.
pub fn is_newer(tag: &str, current: &str) -> bool {
    let parse = |text: &str| semver::Version::parse(text.trim().trim_start_matches(['v', 'V']));
    match (parse(tag), parse(current)) {
        (Ok(latest), Ok(running)) => latest > running,
        _ => false,
    }
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Never fails: offline and rate-limited are normal, so the reason travels in `error`.
pub async fn check(state: &AppState) -> UpdateStatus {
    let channel = detect_channel();
    let mut status = UpdateStatus {
        current_version: current_version().to_string(),
        latest_version: None,
        available: false,
        channel,
        can_install: channel.can_install(),
        release_url: RELEASES_PAGE.to_string(),
        notes: None,
        error: None,
    };

    // Nothing published can be newer than a build from the working tree, and telling a
    // developer to update to the version they are editing is noise.
    if channel == Channel::Development {
        return status;
    }

    let request = state
        .http()
        .get(LATEST_RELEASE_URL)
        // The GitHub API rejects a request without a User-Agent outright.
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");

    let release: Release = match request.send().await {
        Ok(response) if response.status().is_success() => match response.json().await {
            Ok(release) => release,
            Err(e) => {
                status.error = Some(format!("could not read the release feed: {e}"));
                return status;
            }
        },
        Ok(response) => {
            status.error = Some(format!(
                "the release feed answered HTTP {}",
                response.status()
            ));
            return status;
        }
        Err(e) => {
            tracing::debug!("update check failed: {e}");
            status.error = Some("could not reach GitHub to check for updates".to_string());
            return status;
        }
    };

    // `releases/latest` excludes both already, but a pre-release offered as an update is
    // a support burden, so the fields are checked rather than trusted.
    if release.draft || release.prerelease {
        return status;
    }

    status.available = is_newer(&release.tag_name, current_version());
    status.latest_version = Some(release.tag_name.trim_start_matches(['v', 'V']).to_string());
    if let Some(url) = release.html_url {
        status.release_url = url;
    }
    status.notes = release.body.map(|body| {
        let trimmed = body.trim();
        if trimmed.chars().count() > 800 {
            let cut: String = trimmed.chars().take(800).collect();
            format!("{cut}…")
        } else {
            trimmed.to_string()
        }
    });

    if status.available {
        tracing::info!(
            latest = ?status.latest_version,
            current = current_version(),
            ?channel,
            "an update is available"
        );
    }
    status
}

/// The repository a Flatpak install must track for `flatpak update` to find releases.
const FLATPAK_REPO_FILE: &str =
    "https://wieluk.github.io/gameyfin-app/repo/gameyfin-app.flatpakrepo";

/// Runs `flatpak` on the host, since it cannot run inside the sandbox. Returns trimmed stdout.
async fn host_flatpak(args: &[&str]) -> CommandResult<String> {
    let output = tokio::process::Command::new("flatpak-spawn")
        .arg("--host")
        .arg("flatpak")
        .args(args)
        .output()
        .await
        .map_err(|e| {
            CommandError::Message(format!("could not ask the system to run flatpak: {e}"))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CommandError::Message(if stderr.is_empty() {
            stdout
        } else {
            stderr
        }));
    }
    Ok(stdout)
}

/// The URL of `origin` in `flatpak remotes --columns=name,url` output, if it is fetched over
/// the network. Bundles and local builds get a remote with no URL or a `file://` one.
fn network_remote_url<'a>(remotes: &'a str, origin: &str) -> Option<&'a str> {
    remotes
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            Some((columns.next()?, columns.next()?))
        })
        .filter(|(name, _)| *name == origin)
        .map(|(_, url)| url)
        .find(|url| url.starts_with("https://") || url.starts_with("http://"))
}

fn flatpak_app_id() -> String {
    std::env::var("FLATPAK_ID").unwrap_or_else(|_| "org.gameyfin.gameyfin-app".into())
}

async fn flatpak_update() -> CommandResult<UpdateOutcome> {
    let app_id = flatpak_app_id();
    let fail = |step: &str, e: CommandError| {
        CommandError::Message(format!(
            "{step}: {e}. Run `flatpak update {app_id}` yourself."
        ))
    };

    // Without a tracked repository `flatpak update` succeeds with nothing to do, forever.
    let origin = host_flatpak(&["info", "--show-origin", &app_id])
        .await
        .map_err(|e| fail("could not read how Gameyfin was installed", e))?;
    let remotes = host_flatpak(&["remotes", "--columns=name,url"])
        .await
        .map_err(|e| fail("could not list the Flatpak repositories", e))?;
    if network_remote_url(&remotes, &origin).is_none() {
        tracing::info!(%origin, "flatpak install tracks no online repository");
        return Err(CommandError::Message(format!(
            "This Flatpak was installed from a file or a local build, so it has no repository \
             to update from. Add the repository and reinstall once: \
             flatpak remote-add --user --if-not-exists gameyfin-app {FLATPAK_REPO_FILE} && \
             flatpak install --user --reinstall gameyfin-app {app_id}"
        )));
    }

    let before = host_flatpak(&["info", "--show-commit", &app_id]).await.ok();
    let log = host_flatpak(&["update", "-y", "--noninteractive", &app_id])
        .await
        .map_err(|e| fail("the update did not complete", e))?;
    tracing::info!("flatpak update finished: {log}");
    let after = host_flatpak(&["info", "--show-commit", &app_id]).await.ok();

    // GitHub announces a release before the Flatpak repository has it, so say so honestly.
    if before.is_some() && before == after {
        return Err(CommandError::Message(
            "The Flatpak repository has no newer build yet. Try again in a few minutes.".into(),
        ));
    }
    Ok(UpdateOutcome::installed())
}

#[tauri::command]
pub async fn update_status(state: tauri::State<'_, AppState>) -> CommandResult<UpdateStatus> {
    Ok(check(&state).await)
}

/// Install a waiting update where the format allows it. Returns a sentence for the user,
/// since what happens next differs by format.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> CommandResult<UpdateOutcome> {
    match detect_channel() {
        Channel::Flatpak => flatpak_update().await,
        Channel::SelfInstall => self_install(&app).await,
        Channel::SystemPackage => Err(CommandError::Message(
            "This copy of Gameyfin was installed by your system's package manager, so it \
             updates with the rest of your system. Update it there, or download the new \
             package from the releases page."
                .into(),
        )),
        Channel::Development => Err(CommandError::Message(
            "This is a development build; there is nothing to update it to.".into(),
        )),
    }
}

/// Applies a signed update. Without a signing key at build time the plugin refuses
/// everything, which is reported as such.
async fn self_install(app: &tauri::AppHandle) -> CommandResult<UpdateOutcome> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| {
        CommandError::Message(format!(
            "the updater is not available in this build: {e}. \
             Download the new version from the releases page instead."
        ))
    })?;

    let update = updater
        .check()
        .await
        .map_err(|e| CommandError::Message(format!("could not check for an update: {e}")))?;

    let Some(update) = update else {
        return Ok(UpdateOutcome {
            message: "Gameyfin is already up to date.".to_string(),
            restart_needed: false,
        });
    };

    tracing::info!(version = %update.version, "installing an update");
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| CommandError::Message(format!("the update could not be installed: {e}")))?;

    Ok(UpdateOutcome::installed())
}

/// Runs on the host. Waits for this sandbox to close first, or the new copy would hand itself
/// to the old one through the single-instance name and exit.
const FLATPAK_RELAUNCH: &str = r#"i=0
while [ "$i" -lt 50 ] && flatpak ps --columns=application | grep -qx "$1"; do
  sleep 0.2
  i=$((i + 1))
done
exec flatpak run "$1""#;

/// Starts the updated copy and quits this one. Async so Tauri's restart runs the exit
/// handlers that release the single-instance lock.
#[tauri::command]
pub async fn restart_app(app: tauri::AppHandle) -> CommandResult<()> {
    if detect_channel() != Channel::Flatpak {
        app.restart();
    }

    // Restarting inside this sandbox would run the old deploy again.
    std::process::Command::new("flatpak-spawn")
        .args([
            "--host",
            "sh",
            "-c",
            FLATPAK_RELAUNCH,
            "sh",
            &flatpak_app_id(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| CommandError::Message(format!("could not start the new version: {e}")))?;
    crate::tray::quit_now(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_compares_semver_and_refuses_to_guess() {
        for (candidate, current, expected, why) in [
            ("v0.2.0", "0.1.0", true, "minor bump"),
            ("0.1.1", "0.1.0", true, "patch bump"),
            ("v1.0.0", "0.9.9", true, "major bump"),
            ("v0.1.0", "0.1.0", false, "same version"),
            ("v0.1.0", "0.2.0", false, "older than current"),
            // A local build ahead of any release must not be told to downgrade.
            ("v0.1.0", "0.1.1", false, "behind a local build"),
            (
                "0.2.0",
                "v0.1.0",
                true,
                "leading v is optional on the current side",
            ),
            ("V0.2.0", "0.1.0", true, "and its case does not matter"),
            // Semver's own rule: 1.0.0-rc1 is not an update for someone on 1.0.0.
            ("v1.0.0-rc1", "1.0.0", false, "prerelease below its release"),
            ("v1.0.0", "1.0.0-rc1", true, "release above its prerelease"),
            // Guessing here would nag every user forever about a nonexistent release.
            ("nightly", "0.1.0", false, "unparseable candidate"),
            ("", "0.1.0", false, "empty candidate"),
            ("v1.2", "0.1.0", false, "not a full version"),
            ("v0.2.0", "not-a-version", false, "unparseable current"),
        ] {
            assert_eq!(is_newer(candidate, current), expected, "{why}");
        }
    }

    #[test]
    fn only_the_formats_that_own_their_files_can_install() {
        assert!(Channel::SelfInstall.can_install());
        assert!(Channel::Flatpak.can_install());
        // apt and dnf own theirs, and a developer's build tree is nobody's business.
        assert!(!Channel::SystemPackage.can_install());
        assert!(!Channel::Development.can_install());
    }

    #[test]
    fn only_an_online_origin_can_be_updated_from() {
        let remotes = "flathub\thttps://dl.flathub.org/repo/\n\
                       gameyfin-app\thttps://wieluk.github.io/gameyfin-app/repo/\n\
                       gameyfin_app-origin\tfile:///home/me/gameyfin-app/.flatpak-builder/repo\n\
                       gameyfin1-origin\n";
        assert_eq!(
            network_remote_url(remotes, "gameyfin-app"),
            Some("https://wieluk.github.io/gameyfin-app/repo/")
        );
        // `flatpak-builder --install` leaves a local origin, a bundle without a URL leaves none.
        assert_eq!(network_remote_url(remotes, "gameyfin_app-origin"), None);
        assert_eq!(network_remote_url(remotes, "gameyfin1-origin"), None);
        assert_eq!(network_remote_url(remotes, "missing"), None);
        // A prefix of a remote name is not that remote.
        assert_eq!(network_remote_url(remotes, "gameyfin"), None);
    }

    #[test]
    fn a_debug_build_is_always_development() {
        // `cargo run` must never offer to replace itself. Guarded, since `--release` tests have no
        // debug assertions.
        if cfg!(debug_assertions) {
            assert_eq!(detect_channel(), Channel::Development);
        } else {
            assert_ne!(detect_channel(), Channel::Development);
        }
    }

    #[test]
    fn the_current_version_is_real_semver() {
        assert!(
            semver::Version::parse(current_version()).is_ok(),
            "the crate version {} must parse, or every comparison silently says 'no'",
            current_version()
        );
    }
}
