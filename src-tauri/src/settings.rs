//! Persisted client settings.
//!
//! Stored as JSON in the platform config directory. The session cookies live here too so
//! the user is not asked to sign in on every launch, they are written with owner-only
//! permissions on Unix. Moving them into the OS keyring (Credential Manager / libsecret)
//! is the intended next step; the device-token work makes that simpler, because a single
//! long-lived token is a far better fit for a keyring entry than a cookie jar.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Gameyfin server, without a trailing slash.
    pub server_url: Option<String>,
    /// Where games are downloaded and installed.
    pub library_root: Option<String>,
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
}

/// 3 GiB: comfortably above what an installer needs, and below the point where a 32-bit
/// process can be handed the contiguous 2 GB block that triggers the overflow.
fn default_installer_memory_limit() -> u32 {
    3072
}

/// Written by hand rather than derived.
///
/// A derived `Default` would zero `installer_memory_limit_mb`, and `#[serde(default =
/// "...")]` only applies when deserializing, so a freshly constructed `Settings` would
/// silently disable the installer memory cap while a loaded one enabled it.
impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: None,
            library_root: None,
            cookies: HashMap::new(),
            username: None,
            log_level: LogLevel::default(),
            download_limit_kib: 0,
            installer_memory_limit_mb: default_installer_memory_limit(),
            wine_variant: gameyfin_core::wine::WineVariant::default(),
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
}

/// Narrow the file's DACL so only its owner can read it.
///
/// The Unix path uses a 0600 chmod. Windows has no equivalent in std, and while the
/// config directory under %APPDATA% already carries per-user ACLs, the file inherits
/// whatever the profile grants rather than stating its own intent.
///
/// Best effort: a failure is logged, not fatal, so an unusual security policy cannot
/// stop the user signing in.
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
