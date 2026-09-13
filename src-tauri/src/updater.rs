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

/// `flatpak update` cannot run inside the sandbox, so it is handed to the host.
async fn flatpak_update() -> CommandResult<String> {
    let app_id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| "org.gameyfin.Gameyfin".into());

    let output = tokio::process::Command::new("flatpak-spawn")
        .args(["--host", "flatpak", "update", "-y", &app_id])
        .output()
        .await
        .map_err(|e| {
            CommandError::Message(format!(
                "could not ask the system to update Gameyfin: {e}. \
                 Run `flatpak update {app_id}` yourself."
            ))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        return Err(CommandError::Message(format!(
            "the update did not complete: {}",
            if stderr.is_empty() { stdout } else { stderr }
        )));
    }

    tracing::info!("flatpak update finished: {stdout}");
    Ok("Updated. Restart Gameyfin to run the new version.".to_string())
}

#[tauri::command]
pub async fn update_status(state: tauri::State<'_, AppState>) -> CommandResult<UpdateStatus> {
    Ok(check(&state).await)
}

/// Install a waiting update where the format allows it. Returns a sentence for the user,
/// since what happens next differs by format.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> CommandResult<String> {
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
async fn self_install(app: &tauri::AppHandle) -> CommandResult<String> {
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
        return Ok("Gameyfin is already up to date.".to_string());
    };

    tracing::info!(version = %update.version, "installing an update");
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| CommandError::Message(format!("the update could not be installed: {e}")))?;

    Ok("Updated. Restart Gameyfin to run the new version.".to_string())
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
