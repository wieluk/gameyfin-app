//! Persisted client settings, JSON in the platform config directory. Session cookies live
//! here too (owner-only on Unix); moving them to the OS keyring is the intended next step.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Gameyfin server, without a trailing slash.
    pub server_url: Option<String>,
    /// Where games are downloaded and installed. The default of [`Self::library_roots`].
    pub library_root: Option<String>,
    /// Additional places games may go.
    ///
    /// A games library outgrows one drive sooner than almost anything else a person
    /// stores, so the folder is a list rather than a single path. The primary stays a
    /// field of its own: it is the default, and keeping it separate means a settings file
    /// written before this existed still names the same folder it always did.
    #[serde(default)]
    pub extra_library_roots: Vec<String>,
    /// Harvested session cookies, if the user has signed in.
    pub cookies: HashMap<String, String>,
    /// Username of the last signed-in account, shown while reconnecting.
    pub username: Option<String>,
    /// How much detail to write to the log.
    pub log_level: LogLevel,
    /// Download speed cap in KiB/s. Zero means unlimited.
    ///
    /// Applied client-side: Gameyfin does not offer a server-side throttle, so the app
    /// reads more slowly and lets TCP backpressure do the rest.
    #[serde(default)]
    pub download_limit_kib: u32,
    /// Address-space cap applied to Windows installers, in MiB. Zero disables it.
    ///
    /// Defaults on because the bug it avoids, an integer overflow in FreeArc's
    /// `unarc.dll`, which repack installers use, hangs the install with no error at all,
    /// and the cap costs nothing for installers that do not need that much memory.
    #[serde(default = "default_installer_memory_limit")]
    pub installer_memory_limit_mb: u32,
    /// Which Wine build to download and keep up to date.
    ///
    /// A setting rather than a constant because the default, new WoW64, is younger than
    /// the classic path: a user whose installer misbehaves under it can switch without
    /// waiting for a release. Switching needs the 32-bit host libraries the classic build
    /// links against, which a Flatpak sandbox does not have on its own.
    #[serde(default)]
    pub wine_variant: gameyfin_core::wine::WineVariant,
    /// Which download provider to ask the server for, by plugin key.
    ///
    /// None means the server's highest-priority one. Mirrored to the server's
    /// `preferred-download-method` so the web UI and this client agree.
    #[serde(default)]
    pub download_provider: Option<String>,

    /// The user turned down the offer to download Wine and asked not to be offered again.
    ///
    /// Separate from having Wine: a native-only library never needs it, and asking at every
    /// start is the wrong way to find that out.
    #[serde(default)]
    pub wine_prompt_dismissed: bool,

    /// Tell the desktop when a download or an install finishes.
    #[serde(default = "on")]
    pub notify_transfers: bool,
    /// Tell the desktop when something fails.
    ///
    /// Separate from the above because a failure is worth interrupting for even when
    /// somebody has turned the routine chatter off.
    #[serde(default = "on")]
    pub notify_failures: bool,
    /// Tell the desktop when a new version of Gameyfin is released.
    #[serde(default = "on")]
    pub notify_updates: bool,

    /// Closing the window hides it to the tray rather than quitting.
    ///
    /// On by default because the window is not the app: a download runs in this process,
    /// and closing the window during one used to abandon it.
    #[serde(default = "on")]
    pub close_to_tray: bool,
    /// Start hidden, with only the tray icon showing.
    #[serde(default)]
    pub start_minimized: bool,

    /// Extract and install as soon as a download finishes.
    ///
    /// Off by default: a finished download is an archive, and what to do with it is a
    /// decision with consequences (disk space, a setup wizard, an overwritten install).
    /// This is for people who have made that decision once and do not want to be asked.
    #[serde(default)]
    pub auto_install: bool,

    /// Look up each game's umu id so per-title Proton fixes apply.
    #[serde(default = "on")]
    pub umu_fixes: bool,

    /// Read connected controllers.
    #[serde(default = "on")]
    pub gamepad_enabled: bool,
    /// How far a stick must move before it counts, 0.0 to 1.0.
    ///
    /// A worn stick rests off-centre, and without a dead zone that reads as the user
    /// holding a direction forever.
    #[serde(default = "default_deadzone")]
    pub gamepad_deadzone: f64,
    /// Switch to the large-format layout when a controller is connected.
    #[serde(default = "on")]
    pub couch_mode_auto: bool,

    /// Check for a new release at startup.
    #[serde(default = "on")]
    pub check_for_updates: bool,

    /// Tried automatically when an archive turns out to be encrypted.
    ///
    /// Stored in the same owner-only file as the session cookies. It is a convenience for
    /// a library that uses one password throughout, not a secret store.
    #[serde(default)]
    pub extraction_password: Option<String>,

    /// Executable names never offered as a launch candidate.
    ///
    /// Detection is a heuristic over a directory that may hold hundreds of binaries, and
    /// the redistributables and crash handlers shipped beside a game are reliably not the
    /// game. Matched case-insensitively against the file name, as a substring.
    #[serde(default = "default_ignored_executables")]
    pub ignored_executables: Vec<String>,

    /// Which palette to use.
    #[serde(default)]
    pub theme: Theme,

    /// Start Gameyfin when the user logs in.
    #[serde(default)]
    pub autostart: bool,
    /// Identifies this installation to the server, so save history can name the device
    /// a version came from. Generated once, on first use.
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub save_sync_enabled: bool,
    /// Restore a newer save before the game starts.
    #[serde(default = "on")]
    pub sync_saves_on_launch: bool,
    /// Back up and upload after the game exits.
    #[serde(default = "on")]
    pub sync_saves_on_exit: bool,
    /// Where synced saves are kept. Chosen explicitly rather than inferred, so it is always
    /// clear which one is in use.
    #[serde(default)]
    pub save_backend: SaveBackend,
    #[serde(default)]
    pub save_folder: Option<String>,
    #[serde(default)]
    pub webdav_url: Option<String>,
    #[serde(default)]
    pub webdav_username: Option<String>,
    /// Stored unencrypted, like the session cookies beside it. The settings screen says so
    /// where the password is entered, rather than leaving it to be discovered.
    #[serde(default)]
    pub webdav_password: Option<String>,
    /// How many versions a folder or WebDAV store keeps per game. A Gameyfin server owns
    /// its own retention, so this does not apply there.
    #[serde(default = "default_save_versions")]
    pub save_max_versions: u32,
}

/// Executable names that are almost never the game. A default, not hard-coded, so it stays editable.
fn default_ignored_executables() -> Vec<String> {
    [
        "unitycrashhandler",
        "unrealcefsubprocess",
        "crashreport",
        "crashpad",
        "vcredist",
        "vc_redist",
        "dxsetup",
        "directx",
        "dotnetfx",
        "oalinst",
        "uninstall",
        "unins000",
        "notification_helper",
        "quickswitch",
        "cefprocess",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Which palette the interface uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    /// Follow the desktop's own preference.
    System,
    Light,
    /// The palette the app was designed around, and what it used before this was a choice.
    #[default]
    Dark,
}

impl Theme {
    pub fn key(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }
}

/// Default for the several settings that are on unless turned off.
fn on() -> bool {
    true
}

/// A quarter of full deflection: past the slop of a well-used stick, well short of where
/// a deliberate nudge lands.
fn default_deadzone() -> f64 {
    0.25
}

/// 3 GiB: comfortably above what an installer needs, and below the point where a 32-bit
/// process can be handed the contiguous 2 GB block that triggers the overflow.
fn default_installer_memory_limit() -> u32 {
    3072
}

/// Hand-written, not derived: a derived `Default` would zero `installer_memory_limit_mb`
/// (the `#[serde(default)]` fns only run on deserialize), disabling the cap.
/// Where synced saves are kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveBackend {
    /// A Gameyfin server that supports save sync.
    #[default]
    Server,
    /// Any directory: an rclone or Syncthing folder, a NextCloud client's folder, a mount.
    Folder,
    WebDav,
}

fn default_save_versions() -> u32 {
    10
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: None,
            library_root: None,
            extra_library_roots: Vec::new(),
            cookies: HashMap::new(),
            username: None,
            log_level: LogLevel::default(),
            download_limit_kib: 0,
            installer_memory_limit_mb: default_installer_memory_limit(),
            wine_variant: gameyfin_core::wine::WineVariant::default(),
            download_provider: None,
            wine_prompt_dismissed: false,
            notify_transfers: true,
            notify_failures: true,
            notify_updates: true,
            close_to_tray: true,
            start_minimized: false,
            auto_install: false,
            umu_fixes: true,
            gamepad_enabled: true,
            gamepad_deadzone: default_deadzone(),
            couch_mode_auto: true,
            check_for_updates: true,
            installation_id: None,
            save_sync_enabled: false,
            sync_saves_on_launch: true,
            sync_saves_on_exit: true,
            save_backend: SaveBackend::default(),
            save_folder: None,
            webdav_url: None,
            webdav_username: None,
            webdav_password: None,
            save_max_versions: default_save_versions(),
            extraction_password: None,
            ignored_executables: default_ignored_executables(),
            theme: Theme::default(),
            autostart: false,
        }
    }
}

/// Verbosity of the app's log.
///
/// Deliberately a small fixed set rather than a free-text filter: the point is that a user
/// can turn detail up when reporting a problem, not that they compose tracing directives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    /// Problems only.
    Error,
    /// Problems and things that might become problems.
    Warn,
    /// What the app did: downloads, installs, launches. The default.
    #[default]
    Info,
    /// Everything above plus the detail needed to diagnose a failure.
    Debug,
    /// Very noisy; includes library internals.
    Trace,
}

impl LogLevel {
    /// The tracing filter for this level.
    ///
    /// Third-party crates stay at `warn` throughout: their debug output buries ours and
    /// is rarely what a bug report needs.
    pub fn filter(self) -> String {
        let app = match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        };
        format!(
            "warn,gameyfin_app={app},gameyfin_api={app},gameyfin_core={app},gameyfin_saves={app}"
        )
    }

    pub fn key(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "error" => Some(LogLevel::Error),
            "warn" => Some(LogLevel::Warn),
            "info" => Some(LogLevel::Info),
            "debug" => Some(LogLevel::Debug),
            "trace" => Some(LogLevel::Trace),
            _ => None,
        }
    }
}

impl Settings {
    pub fn is_configured(&self) -> bool {
        self.server_url.as_ref().is_some_and(|u| !u.is_empty())
    }

    pub fn has_session(&self) -> bool {
        self.is_configured() && !self.cookies.is_empty()
    }

    pub async fn load(config_dir: &Path) -> Self {
        let path = config_dir.join(SETTINGS_FILE);
        let Ok(bytes) = tokio::fs::read(&path).await else {
            return Self::default();
        };
        // Corrupt settings should not stop the app from starting; the wizard will run
        // again, which is a recoverable outcome rather than a crash.
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!("ignoring unreadable settings at {path:?}: {e}");
            Self::default()
        })
    }

    pub async fn save(&self, config_dir: &Path) -> std::io::Result<()> {
        tokio::fs::create_dir_all(config_dir).await?;
        let path = config_dir.join(SETTINGS_FILE);
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        tokio::fs::write(&path, json).await?;

        // The file holds session cookies, so keep it readable only by its owner.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&path).await?.permissions();
            perms.set_mode(0o600);
            tokio::fs::set_permissions(&path, perms).await?;
        }
        #[cfg(windows)]
        restrict_to_owner(&path);

        Ok(())
    }

    /// Forget the signed-in session, keeping every other preference.
    pub fn clear_session(&mut self) {
        self.cookies.clear();
        self.username = None;
    }

    /// Default library root, used until the user picks one.
    pub fn default_library_root(home: &Path) -> PathBuf {
        home.join("Games")
    }

    /// Every configured games folder, the default first.
    ///
    /// Deduplicated, because the same folder listed twice would offer the user a choice
    /// between two identical destinations and then rescan it twice.
    pub fn library_roots(&self) -> Vec<String> {
        let mut roots: Vec<String> = self
            .library_root
            .iter()
            .chain(self.extra_library_roots.iter())
            .filter(|root| !root.trim().is_empty())
            .cloned()
            .collect();
        let mut seen = std::collections::HashSet::new();
        roots.retain(|root| seen.insert(root.clone()));
        roots
    }

    /// Whether a path is one of the configured roots.
    ///
    /// Used to reject a download destination that did not come from the list, so a
    /// crafted request cannot pick where files land.
    pub fn is_library_root(&self, candidate: &str) -> bool {
        self.library_roots().iter().any(|root| root == candidate)
    }

    /// Whether an executable name is one the user never wants offered.
    pub fn is_ignored_executable(&self, path: &str) -> bool {
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .to_lowercase();
        self.ignored_executables
            .iter()
            .filter(|pattern| !pattern.trim().is_empty())
            .any(|pattern| name.contains(&pattern.trim().to_lowercase()))
    }
}

/// Narrow the file's DACL to its owner (the Unix path uses a 0600 chmod). Best effort: a
/// failure is logged, not fatal.
#[cfg(windows)]
fn restrict_to_owner(path: &Path) {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    // "Protected, inherit nothing; full access for the owner and for SYSTEM." SYSTEM is
    // kept so backup and repair tooling still works.
    let sddl: Vec<u16> = "D:PAI(A;;FA;;;OW)(A;;FA;;;SY)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: the SDDL string is NUL-terminated, and `descriptor` is only read after the
    // call reports success. It is released with LocalFree below, as the API requires.
    let built = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if built == 0 || descriptor.is_null() {
        tracing::warn!("could not build the settings file's access rules");
        return;
    }

    let mut present: i32 = 0;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut defaulted: i32 = 0;
    // SAFETY: `descriptor` is a valid security descriptor built from SDDL above, so its
    // DACL pointer can be read out.
    let read =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) };

    let result = if read != 0 && present != 0 {
        // SAFETY: `wide` is NUL-terminated and `dacl` points into `descriptor`, which is
        // still alive until the LocalFree below.
        unsafe {
            SetNamedSecurityInfoW(
                wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            )
        }
    } else {
        tracing::warn!("the settings file's access rules had no DACL");
        ERROR_SUCCESS
    };

    // SAFETY: `descriptor` came from a Local* allocation and is not used afterwards.
    unsafe { LocalFree(descriptor as _) };

    if result != ERROR_SUCCESS {
        tracing::warn!(code = result, "could not restrict the settings file");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_url_does_not_count_as_configured() {
        let mut s = Settings::default();
        assert!(!s.is_configured());
        s.server_url = Some(String::new());
        assert!(!s.is_configured());
        s.server_url = Some("https://games.example".into());
        assert!(s.is_configured());
    }

    #[test]
    fn a_session_needs_both_a_server_and_cookies() {
        let mut s = Settings {
            server_url: Some("https://games.example".into()),
            ..Default::default()
        };
        assert!(!s.has_session());
        s.cookies.insert("JSESSIONID".into(), "abc".into());
        assert!(s.has_session());
    }

    #[test]
    fn log_levels_round_trip_through_their_keys() {
        for level in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            assert_eq!(LogLevel::from_key(level.key()), Some(level));
        }
        assert_eq!(LogLevel::from_key("nonsense"), None);
    }

    #[test]
    fn the_filter_keeps_third_party_crates_quiet() {
        let filter = LogLevel::Debug.filter();
        assert!(filter.starts_with("warn,"), "got {filter}");
        assert!(filter.contains("gameyfin_app=debug"));
        assert!(filter.contains("gameyfin_core=debug"));
    }

    #[test]
    fn the_installer_memory_cap_is_on_by_default() {
        assert_eq!(Settings::default().installer_memory_limit_mb, 3072);
    }

    #[test]
    fn a_constructed_default_matches_a_deserialized_one() {
        // The two are written separately, so they drift silently: a `#[serde(default)]`
        // that says `true` beside a struct literal that says `false` gives a fresh
        // install different behaviour from an upgraded one, with nothing to show for it.
        let from_empty: Settings = serde_json::from_str("{}").expect("all fields default");
        assert_eq!(from_empty, Settings::default());
    }

    #[test]
    fn a_settings_file_written_before_these_options_existed_still_loads() {
        // Upgrading must not reset someone's server and session because the file has no
        // `gamepadEnabled` in it.
        let old = r#"{"serverUrl":"https://games.example","libraryRoot":"/games"}"#;
        let settings: Settings = serde_json::from_str(old).expect("old files still load");
        assert_eq!(
            settings.server_url.as_deref(),
            Some("https://games.example")
        );
        assert!(settings.close_to_tray, "new options take their default");
        assert!(!settings.auto_install);
    }

    #[test]
    fn the_roots_list_starts_with_the_primary_and_drops_duplicates() {
        let settings = Settings {
            library_root: Some("/games".into()),
            extra_library_roots: vec!["/mnt/big".into(), "/games".into(), "  ".into()],
            ..Default::default()
        };
        assert_eq!(settings.library_roots(), vec!["/games", "/mnt/big"]);
        assert!(settings.is_library_root("/mnt/big"));
        // A destination that is not on the list cannot be chosen.
        assert!(!settings.is_library_root("/etc"));
    }

    #[test]
    fn an_unconfigured_client_has_no_roots() {
        assert!(Settings::default().library_roots().is_empty());
        assert!(!Settings::default().is_library_root(""));
    }

    #[test]
    fn the_ignore_list_matches_the_file_name_not_the_path() {
        let settings = Settings::default();
        assert!(settings.is_ignored_executable("UnityCrashHandler64.exe"));
        assert!(settings.is_ignored_executable("bin/win64/vcredist_x64.exe"));
        assert!(settings.is_ignored_executable(r"bin\UnrealCEFSubProcess.exe"));
        // The game itself must survive, even inside a folder whose name matches.
        assert!(!settings.is_ignored_executable("Celeste.exe"));
        assert!(!settings.is_ignored_executable("vcredist/Game.exe"));
    }

    #[test]
    fn an_empty_ignore_pattern_does_not_match_everything() {
        // A blank line left in the editable list would otherwise hide every executable.
        let settings = Settings {
            ignored_executables: vec![String::new(), "  ".into()],
            ..Default::default()
        };
        assert!(!settings.is_ignored_executable("Celeste.exe"));
    }

    #[test]
    fn themes_round_trip_through_their_keys() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_key(theme.key()), Some(theme));
        }
        assert_eq!(Theme::from_key("nonsense"), None);
        // Dark is what the app was designed around and what it used before the choice.
        assert_eq!(Theme::default(), Theme::Dark);
    }

    #[test]
    fn the_noisier_options_default_off() {
        // Anything that changes what happens to a user's disk without asking starts off.
        let settings = Settings::default();
        assert!(!settings.auto_install);
        assert!(!settings.start_minimized);
    }

    #[test]
    fn info_is_the_default() {
        assert_eq!(Settings::default().log_level, LogLevel::Info);
    }

    #[tokio::test]
    async fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("gameyfin-settings-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;

        let mut settings = Settings {
            server_url: Some("https://games.example".into()),
            library_root: Some("/games".into()),
            username: Some("alice".into()),
            ..Default::default()
        };
        settings.cookies.insert("JSESSIONID".into(), "abc".into());
        settings.save(&dir).await.unwrap();

        assert_eq!(Settings::load(&dir).await, settings);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn missing_or_corrupt_settings_fall_back_to_defaults() {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-settings-bad-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        assert_eq!(Settings::load(&dir).await, Settings::default());

        tokio::fs::write(dir.join(SETTINGS_FILE), b"{ not json")
            .await
            .unwrap();
        assert_eq!(Settings::load(&dir).await, Settings::default());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
