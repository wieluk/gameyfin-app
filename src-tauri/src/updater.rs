//! Keeping the app up to date. What a package can do about its own updates depends on who
//! owns the files:
//!
//! | Format          | Owner                | What we do                              |
//! |-----------------|----------------------|-----------------------------------------|
//! | AppImage        | the user             | replace it in place, signed              |
//! | MSI / NSIS      | the user's installer | run the new installer, signed            |
//! | Flatpak         | the Flatpak system   | `flatpak update` on the host             |
//! | deb / rpm       | apt / dnf            | say a version exists; the OS installs it |
//! | `cargo run`     | the developer        | nothing at all                           |
//!
//! Which one is detected from the environment (one Linux binary, four packages). The
//! **check** (one GitHub API GET) is separate from the format-gated **install** and works
//! everywhere.

use serde::{Deserialize, Serialize};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// The release feed. A draft release is not published, so this only ever sees real ones.
const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/gameyfin/gameyfin-app/releases/latest";

/// Where a user is sent when their package cannot update itself.
const RELEASES_PAGE: &str = "https://github.com/gameyfin/gameyfin-app/releases/latest";

/// How this copy of the app can be updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    /// Tauri's signed updater replaces the bundle in place. AppImage, MSI, NSIS.
    SelfInstall,
    /// `flatpak update`, run on the host because we are inside the sandbox.
    Flatpak,
    /// apt or dnf owns the files; we can only report that a release exists.
    SystemPackage,
    /// Running from a build tree. Never offer an update.
    Development,
}

impl Channel {
    /// Whether the app can start the update itself.
    pub fn can_install(self) -> bool {
        matches!(self, Channel::SelfInstall | Channel::Flatpak)
    }
}

/// Work out how this copy was installed.
///
/// Environment first, because the same Linux executable ships inside four packages.
/// `APPIMAGE` is set by the AppImage runtime and `FLATPAK_ID` by the Flatpak one, both
/// before any of our code runs, so their presence is conclusive.
pub fn detect_channel() -> Channel {
    // A debug build is a build tree by definition, whatever the surroundings look like.
    if cfg!(debug_assertions) {
        return Channel::Development;
    }
    if std::env::var_os("FLATPAK_ID").is_some() {
        return Channel::Flatpak;
    }
    if std::env::var_os("APPIMAGE").is_some() {
        return Channel::SelfInstall;
    }
    if cfg!(windows) {
        return Channel::SelfInstall;
    }
    // A release build on Linux that is neither AppImage nor Flatpak came from the deb or
    // the rpm, both of which are owned by the system package manager.
    Channel::SystemPackage
}

/// What the UI shows about updates.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    /// The newest published release, or null when the check could not be made.
    pub latest_version: Option<String>,
    /// True only when the newest release is genuinely newer than what is running.
    pub available: bool,
    pub channel: Channel,
    /// Whether this package can install the update itself.
    pub can_install: bool,
    pub release_url: String,
    /// The release notes, trimmed to something a panel can show.
    pub notes: Option<String>,
    /// Why the check failed, when it did.
    pub error: Option<String>,
}

/// The fields we read from a GitHub release.
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

/// Compare a release tag against the running version.
///
/// Tags are `v1.2.3`; the leading `v` is conventional and not part of the version. A tag
/// that is not valid semver is treated as "not newer" rather than guessed at, because the
/// failure mode of guessing is nagging every user forever about a release that does not
/// exist.
pub fn is_newer(tag: &str, current: &str) -> bool {
    let parse = |text: &str| semver::Version::parse(text.trim().trim_start_matches(['v', 'V']));
    match (parse(tag), parse(current)) {
        (Ok(latest), Ok(running)) => latest > running,
        _ => false,
    }
}

/// The version this binary was built as.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Ask GitHub what the newest release is.
///
/// Never returns an error: a failed check is a normal condition (offline, rate-limited,
/// GitHub down) and must not surface as a broken screen. The reason travels in `error`.
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
        .await
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

    // `releases/latest` already excludes both, but the fields are checked rather than
    // trusted: a pre-release offered as an update is a support burden.
    if release.draft || release.prerelease {
        return status;
    }

    status.available = is_newer(&release.tag_name, current_version());
    status.latest_version = Some(release.tag_name.trim_start_matches(['v', 'V']).to_string());
    if let Some(url) = release.html_url {
        status.release_url = url;
    }
    status.notes = release.body.map(|body| {
        // Release notes can be long; the panel shows an excerpt and links out for the rest.
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

/// Ask the Flatpak system to update us, from inside the sandbox.
///
/// `flatpak update` cannot run in here, so it is handed to the host through the sandbox
/// helper, the same escape hatch the app already uses to reach the host's Wine. No
/// elevation is involved: a `--user` installation is the user's own.
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

/// Ask GitHub what is new and report it.
#[tauri::command]
pub async fn update_status(state: tauri::State<'_, AppState>) -> CommandResult<UpdateStatus> {
    Ok(check(&state).await)
}

/// Install a waiting update, where the package format allows it.
///
/// Returns a sentence to show the user, because what happens next differs: a Flatpak
/// update lands on disk and takes effect on the next start, while a self-install replaces
/// the running bundle and needs a restart to be worth anything either.
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

/// Download and apply a signed update to an AppImage or a Windows install.
///
/// Requires the signing key to have been configured at build time. Without it the plugin
/// has no public key to verify against and refuses everything, which is reported as such
/// rather than as a mysterious failure.
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
    fn a_higher_version_is_newer() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
    }

    #[test]
    fn the_same_or_an_older_version_is_not() {
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        // A local build ahead of any release must not be told to downgrade.
        assert!(!is_newer("v0.1.0", "0.1.1"));
    }

    #[test]
    fn the_leading_v_is_optional_on_both_sides() {
        assert!(is_newer("0.2.0", "v0.1.0"));
        assert!(is_newer("V0.2.0", "0.1.0"));
    }

    #[test]
    fn a_prerelease_sorts_below_its_own_release() {
        // Semver's own rule, and the one that matters: 1.0.0-rc1 is not an update for
        // someone already running 1.0.0.
        assert!(!is_newer("v1.0.0-rc1", "1.0.0"));
        assert!(is_newer("v1.0.0", "1.0.0-rc1"));
    }

    #[test]
    fn an_unparseable_tag_is_never_treated_as_newer() {
        // Guessing here would nag every user forever about a release that does not exist.
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("v1.2", "0.1.0"));
        assert!(!is_newer("v0.2.0", "not-a-version"));
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
        // Whatever the surroundings, `cargo run` must never offer to replace itself.
        // Guarded so the assertion still means something under `cargo test --release`,
        // where the very condition it is checking no longer holds.
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
