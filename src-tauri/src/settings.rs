//! Persisted client settings. Session cookies and passwords live here too, in an owner-only
//! file; moving them to the OS keyring is the intended next step.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gameyfin_core::InstallerKind;
use serde::{Deserialize, Serialize};

/// A partial update: a field that was not sent keeps its value. For text fields an empty
/// string clears the value.
#[derive(Debug, Default, Clone, Deserialize, ts_rs::TS)]
// Refused rather than ignored, so a misspelled field fails loudly instead of changing nothing.
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct SettingsPatch {
    pub notify_transfers: Option<bool>,
    pub notify_failures: Option<bool>,
    pub notify_updates: Option<bool>,
    pub close_to_tray: Option<bool>,
    pub start_minimized: Option<bool>,
    pub autostart: Option<bool>,
    pub auto_install: Option<bool>,
    pub auto_extract: Option<bool>,
    pub gamepad_enabled: Option<bool>,
    pub gamepad_deadzone: Option<f64>,
    pub couch_mode_auto: Option<bool>,
    pub umu_fixes: Option<bool>,
    pub umu_auto_update: Option<bool>,
    pub check_for_updates: Option<bool>,
    pub installer_memory_limit: Option<InstallerMemoryLimit>,
    pub download_limit_kib: Option<u32>,
    pub delete_archive_after_extract: Option<bool>,
    pub delete_download_after_install: Option<bool>,
    pub inno_setup_arguments: Option<String>,
    pub nsis_arguments: Option<String>,
    pub theme: Option<Theme>,
    pub log_level: Option<LogLevel>,
    pub library_root: Option<String>,
    pub extraction_password: Option<String>,
    pub ignored_executables: Option<Vec<String>>,
    pub save_manifest_auto_update: Option<bool>,
    pub device_name: Option<String>,
    pub save_sync_enabled: Option<bool>,
    pub sync_saves_on_launch: Option<bool>,
    pub sync_saves_on_exit: Option<bool>,
    pub save_backend: Option<SaveBackend>,
    pub save_folder: Option<String>,
    pub webdav_url: Option<String>,
    pub webdav_username: Option<String>,
    pub webdav_password: Option<String>,
    pub save_max_versions: Option<u32>,
}

impl SettingsPatch {
    /// Whether the controller's live handle needs refreshing.
    pub fn touches_gamepad(&self) -> bool {
        self.gamepad_enabled.is_some() || self.gamepad_deadzone.is_some()
    }

    pub fn apply(&self, s: &mut Settings) {
        macro_rules! set {
            ($($field:ident),+ $(,)?) => {$(
                if let Some(value) = self.$field.clone() { s.$field = value; }
            )+};
        }
        macro_rules! set_text {
            ($($field:ident),+ $(,)?) => {$(
                if let Some(value) = &self.$field { s.$field = non_blank(value); }
            )+};
        }
        set!(
            notify_transfers,
            notify_failures,
            notify_updates,
            close_to_tray,
            start_minimized,
            autostart,
            auto_install,
            auto_extract,
            gamepad_enabled,
            gamepad_deadzone,
            couch_mode_auto,
            umu_fixes,
            umu_auto_update,
            check_for_updates,
            installer_memory_limit,
            download_limit_kib,
            delete_archive_after_extract,
            delete_download_after_install,
            theme,
            log_level,
            save_manifest_auto_update,
            save_sync_enabled,
            sync_saves_on_launch,
            sync_saves_on_exit,
            save_backend,
        );
        set_text!(
            library_root,
            device_name,
            save_folder,
            webdav_url,
            webdav_username,
        );
        // Passwords are not trimmed: spaces can be part of one.
        if let Some(password) = &self.extraction_password {
            s.extraction_password = Some(password.clone()).filter(|p| !p.is_empty());
        }
        if let Some(password) = &self.webdav_password {
            s.webdav_password = Some(password.clone()).filter(|p| !p.is_empty());
        }
        // Blank restores the default.
        let switches = |value: &String, kind: InstallerKind| {
            non_blank(value).unwrap_or_else(|| kind.default_unattended_args().to_string())
        };
        if let Some(value) = &self.inno_setup_arguments {
            s.inno_setup_arguments = switches(value, InstallerKind::InnoSetup);
        }
        if let Some(value) = &self.nsis_arguments {
            s.nsis_arguments = switches(value, InstallerKind::Nsis);
        }
        if let Some(versions) = self.save_max_versions {
            s.save_max_versions = versions.max(1);
        }
        if let Some(entries) = &self.ignored_executables {
            // A blank entry would match every executable.
            s.ignored_executables = entries.iter().filter_map(|e| non_blank(e)).collect();
        }
    }
}

impl Settings {
    pub fn unattended_args(&self, kind: InstallerKind) -> Vec<String> {
        let typed = match kind {
            InstallerKind::InnoSetup => &self.inno_setup_arguments,
            InstallerKind::Nsis => &self.nsis_arguments,
            InstallerKind::InstallShield | InstallerKind::Unknown => return Vec::new(),
        };
        gameyfin_core::arguments::split(typed)
    }
}

fn non_blank(value: &str) -> Option<String> {
    Some(value.trim().to_string()).filter(|v| !v.is_empty())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    /// Without a trailing slash.
    pub server_url: Option<String>,
    /// The default games folder, first of [`Self::library_roots`].
    pub library_root: Option<String>,
    pub extra_library_roots: Vec<String>,
    #[ts(skip)]
    pub cookies: HashMap<String, String>,
    pub username: Option<String>,
    pub log_level: LogLevel,
    /// Zero is unlimited. Client-side, since Gameyfin has no server throttle.
    pub download_limit_kib: u32,
    /// Repack installers size buffers from all the RAM they find, so they are capped.
    pub installer_memory_limit: InstallerMemoryLimit,
    /// `None` is the server's highest-priority provider.
    pub download_provider: Option<String>,
    pub notify_transfers: bool,
    /// Separate so failures stay audible with the routine chatter off.
    pub notify_failures: bool,
    pub notify_updates: bool,
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub auto_install: bool,
    /// Unpacks archives, while downloading where the format allows. Auto install implies it.
    pub auto_extract: bool,
    pub delete_archive_after_extract: bool,
    pub delete_download_after_install: bool,
    pub inno_setup_arguments: String,
    pub nsis_arguments: String,
    pub umu_fixes: bool,
    pub umu_auto_update: bool,
    pub gamepad_enabled: bool,
    /// 0.0 to 1.0; a worn stick rests off-centre.
    pub gamepad_deadzone: f64,
    pub couch_mode_auto: bool,
    pub check_for_updates: bool,
    #[ts(skip)]
    pub extraction_password: Option<String>,
    /// Case-insensitive substrings of file names never offered as the game.
    pub ignored_executables: Vec<String>,
    pub theme: Theme,
    pub autostart: bool,
    /// Lets the server name the device a save came from.
    pub installation_id: Option<String>,
    pub save_manifest_auto_update: bool,
    /// Falls back to the host name.
    pub device_name: Option<String>,
    pub save_sync_enabled: bool,
    pub sync_saves_on_launch: bool,
    pub sync_saves_on_exit: bool,
    pub save_backend: SaveBackend,
    pub save_folder: Option<String>,
    pub webdav_url: Option<String>,
    pub webdav_username: Option<String>,
    #[ts(skip)]
    pub webdav_password: Option<String>,
    /// Folder and WebDAV retention; a Gameyfin server keeps its own.
    pub save_max_versions: u32,
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
            installer_memory_limit: InstallerMemoryLimit::default(),
            download_provider: None,
            notify_transfers: true,
            notify_failures: true,
            notify_updates: true,
            close_to_tray: false,
            start_minimized: false,
            auto_install: false,
            auto_extract: true,
            delete_archive_after_extract: true,
            delete_download_after_install: false,
            inno_setup_arguments: InstallerKind::InnoSetup.default_unattended_args().into(),
            nsis_arguments: InstallerKind::Nsis.default_unattended_args().into(),
            umu_fixes: true,
            umu_auto_update: true,
            gamepad_enabled: true,
            gamepad_deadzone: 0.25,
            couch_mode_auto: true,
            check_for_updates: true,
            extraction_password: None,
            ignored_executables: [
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
            .map(String::from)
            .to_vec(),
            theme: Theme::default(),
            autostart: false,
            installation_id: None,
            save_manifest_auto_update: true,
            device_name: None,
            save_sync_enabled: true,
            sync_saves_on_launch: true,
            sync_saves_on_exit: true,
            save_backend: SaveBackend::default(),
            save_folder: None,
            webdav_url: None,
            webdav_username: None,
            webdav_password: None,
            save_max_versions: 10,
        }
    }
}

/// What the webview may see: no cookies, and passwords reduced to whether one is set.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PublicSettings {
    #[serde(flatten)]
    pub settings: Settings,
    pub has_extraction_password: bool,
    pub has_webdav_password: bool,
}

impl From<Settings> for PublicSettings {
    fn from(mut settings: Settings) -> Self {
        settings.cookies.clear();
        let has_extraction_password = settings.extraction_password.take().is_some();
        let has_webdav_password = settings.webdav_password.take().is_some();
        Self {
            settings,
            has_extraction_password,
            has_webdav_password,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Theme {
    System,
    Light,
    #[default]
    Dark,
}

/// A small fixed set, so a user can turn detail up for a bug report without tracing syntax.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Third-party crates stay at `warn`: their debug output buries ours.
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
}

/// Stored as `"auto"`, `"off"` or a number of MiB.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ts_rs::TS)]
#[ts(export, type = "\"auto\" | \"off\" | number")]
pub enum InstallerMemoryLimit {
    /// Half the machine's RAM, never below [`InstallerMemoryLimit::FLOOR_MIB`].
    #[default]
    Auto,
    Off,
    Mib(u32),
}

impl InstallerMemoryLimit {
    /// The cap counts address space, and a 32-bit Wine process reserves nearly 4 GB at start.
    pub const FLOOR_MIB: u64 = 4096;

    pub fn resolve(self, total_ram_bytes: Option<u64>) -> Option<u64> {
        match self {
            Self::Off | Self::Mib(0) => None,
            Self::Mib(mib) => Some(u64::from(mib)),
            // No RAM figure means no guess: guessing low breaks installs.
            Self::Auto => {
                total_ram_bytes.map(|bytes| (bytes / 2 / (1024 * 1024)).max(Self::FLOOR_MIB))
            }
        }
    }
}

impl Serialize for InstallerMemoryLimit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Off => serializer.serialize_str("off"),
            Self::Mib(mib) => serializer.serialize_u32(*mib),
        }
    }
}

impl<'de> Deserialize<'de> for InstallerMemoryLimit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Mib(u32),
            Word(String),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Mib(mib) => Ok(Self::Mib(mib)),
            Wire::Word(word) => match word.as_str() {
                "auto" => Ok(Self::Auto),
                "off" => Ok(Self::Off),
                _ => Err(serde::de::Error::custom(format!(
                    "{word} is not an installer memory limit"
                ))),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum SaveBackend {
    #[default]
    Server,
    /// Any directory: an rclone or Syncthing folder, a mount.
    Folder,
    WebDav,
}

impl Settings {
    pub fn is_configured(&self) -> bool {
        self.server_url.as_ref().is_some_and(|u| !u.is_empty())
    }

    pub fn has_session(&self) -> bool {
        self.is_configured() && !self.cookies.is_empty()
    }

    pub fn clear_session(&mut self) {
        self.cookies.clear();
        self.username = None;
    }

    /// Which store saves go to. Version ids from one store mean nothing to another.
    pub fn save_store_identity(&self) -> (SaveBackend, Option<String>) {
        let location = match self.save_backend {
            SaveBackend::Server => &self.server_url,
            SaveBackend::Folder => &self.save_folder,
            SaveBackend::WebDav => &self.webdav_url,
        };
        let location = location
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string);
        (self.save_backend, location)
    }

    pub fn default_library_root(home: &Path) -> PathBuf {
        home.join("Games")
    }

    /// Every games folder, the default first, without blanks or duplicates.
    pub fn library_roots(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.library_root
            .iter()
            .chain(&self.extra_library_roots)
            .filter(|root| !root.trim().is_empty() && seen.insert(root.as_str()))
            .cloned()
            .collect()
    }

    pub fn set_library_roots(&mut self, roots: Vec<String>) {
        let mut roots = roots.into_iter();
        self.library_root = roots.next();
        self.extra_library_roots = roots.collect();
    }

    pub fn is_library_root(&self, candidate: &str) -> bool {
        self.library_roots().iter().any(|root| root == candidate)
    }

    /// A requested folder, which must be configured, or the default one.
    pub fn require_root(&self, requested: Option<String>) -> Result<String, String> {
        match requested {
            Some(root) if self.is_library_root(&root) => Ok(root),
            Some(root) => Err(format!("{root} is not one of your games folders.")),
            None => self
                .library_root
                .clone()
                .ok_or_else(|| "No games folder configured yet. Set one in Settings.".into()),
        }
    }

    pub fn is_ignored_executable(&self, path: &str) -> bool {
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .to_lowercase();
        self.ignored_executables
            .iter()
            .map(|pattern| pattern.trim().to_lowercase())
            .any(|pattern| !pattern.is_empty() && name.contains(&pattern))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_needs_a_non_blank_server_and_cookies() {
        let mut s = Settings::default();
        assert!(!s.is_configured());
        s.server_url = Some(String::new());
        assert!(!s.is_configured());
        s.server_url = Some("https://games.example".into());
        assert!(s.is_configured() && !s.has_session());
        s.cookies.insert("JSESSIONID".into(), "abc".into());
        assert!(s.has_session());
    }

    #[test]
    fn the_save_store_changes_with_the_backend_or_its_location_only() {
        let mut s = Settings {
            server_url: Some("https://games.example".into()),
            ..Default::default()
        };
        let server = s.save_store_identity();

        s.save_folder = Some("/saves".into());
        s.webdav_url = Some("https://dav.example".into());
        assert_eq!(
            server,
            s.save_store_identity(),
            "an inactive location is not the store"
        );

        s.save_backend = SaveBackend::Folder;
        let folder = s.save_store_identity();
        assert_ne!(server, folder);

        s.save_folder = Some(" /saves ".into());
        assert_eq!(folder, s.save_store_identity());
        s.save_folder = Some("/elsewhere".into());
        assert_ne!(folder, s.save_store_identity());
    }

    #[test]
    fn the_filter_keeps_third_party_crates_quiet() {
        let filter = LogLevel::Debug.filter();
        assert!(filter.starts_with("warn,"), "got {filter}");
        assert!(filter.contains("gameyfin_app=debug"));
    }

    /// The names `src/lib/backend.ts` sends; a drifted name would change nothing silently.
    const CLIENT_FIELDS: &[(&str, &str)] = &[
        ("notifyTransfers", "false"),
        ("notifyFailures", "false"),
        ("notifyUpdates", "false"),
        ("closeToTray", "true"),
        ("startMinimized", "true"),
        ("autostart", "true"),
        ("autoInstall", "true"),
        ("autoExtract", "false"),
        ("gamepadEnabled", "false"),
        ("gamepadDeadzone", "0.4"),
        ("couchModeAuto", "false"),
        ("umuFixes", "false"),
        ("umuAutoUpdate", "false"),
        ("checkForUpdates", "false"),
        ("installerMemoryLimit", "2048"),
        ("downloadLimitKib", "512"),
        ("deleteArchiveAfterExtract", "false"),
        ("deleteDownloadAfterInstall", "true"),
        ("innoSetupArguments", "\"/SILENT\""),
        ("nsisArguments", "\"/S /NCRC\""),
        ("theme", "\"light\""),
        ("logLevel", "\"debug\""),
        ("libraryRoot", "\"/games\""),
        ("extractionPassword", "\"hunter2\""),
        ("ignoredExecutables", "[\"x.exe\"]"),
        ("saveManifestAutoUpdate", "false"),
        ("deviceName", "\"Desk\""),
        ("saveSyncEnabled", "false"),
        ("syncSavesOnLaunch", "false"),
        ("syncSavesOnExit", "false"),
        ("saveBackend", "\"webdav\""),
        ("saveFolder", "\"/saves\""),
        ("webdavUrl", "\"https://dav\""),
        ("webdavUsername", "\"me\""),
        ("webdavPassword", "\"pw\""),
        ("saveMaxVersions", "3"),
    ];

    #[test]
    fn every_field_the_client_sends_is_understood() {
        for (name, value) in CLIENT_FIELDS {
            let patch: SettingsPatch = serde_json::from_str(&format!("{{\"{name}\": {value}}}"))
                .unwrap_or_else(|e| panic!("{name} is not a field of SettingsPatch: {e}"));
            let mut settings = Settings::default();
            patch.apply(&mut settings);
            assert_ne!(settings, Settings::default(), "{name} changed nothing");
        }
    }

    #[test]
    fn unknown_fields_and_values_are_refused() {
        assert!(serde_json::from_str::<SettingsPatch>(r#"{"autoinstall": true}"#).is_err());
        let error = serde_json::from_str::<SettingsPatch>(r#"{"theme": "neon"}"#).unwrap_err();
        assert!(error.to_string().contains("neon"), "got {error}");
    }

    #[test]
    fn a_patch_changes_only_what_it_sets_and_blanks_clear_text() {
        let mut settings = Settings {
            close_to_tray: true,
            device_name: Some("Desk".into()),
            inno_setup_arguments: "/SILENT".into(),
            ..Default::default()
        };
        let patch = SettingsPatch {
            inno_setup_arguments: Some("  ".into()),
            notify_failures: Some(false),
            device_name: Some("   ".into()),
            extraction_password: Some("  ".into()),
            ignored_executables: Some(vec![" vcredist.exe ".into(), "  ".into()]),
            save_max_versions: Some(0),
            ..Default::default()
        };
        patch.apply(&mut settings);

        assert!(!settings.notify_failures && settings.close_to_tray);
        assert_eq!(settings.device_name, None);
        assert_eq!(settings.extraction_password.as_deref(), Some("  "));
        assert_eq!(settings.ignored_executables, vec!["vcredist.exe"]);
        assert_eq!(settings.save_max_versions, 1);
        assert_eq!(
            settings.unattended_args(InstallerKind::InnoSetup),
            vec!["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
            "a blank field falls back to the default"
        );
        assert!(settings.unattended_args(InstallerKind::Unknown).is_empty());
    }

    #[test]
    fn the_webview_never_sees_cookies_or_passwords() {
        let mut settings = Settings {
            webdav_password: Some("secret".into()),
            ..Default::default()
        };
        settings.cookies.insert("JSESSIONID".into(), "abc".into());
        let json = serde_json::to_string(&PublicSettings::from(settings)).unwrap();
        assert!(!json.contains("secret") && !json.contains("abc"), "{json}");
        assert!(json.contains("\"hasWebdavPassword\":true"));
        assert!(json.contains("\"hasExtractionPassword\":false"));
    }

    #[test]
    fn defaults_are_what_we_intend_and_match_deserialization() {
        let settings = Settings::default();
        assert!(
            !settings.auto_install
                && settings.auto_extract
                && settings.delete_archive_after_extract
        );
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(settings.installer_memory_limit, InstallerMemoryLimit::Auto);
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), settings);
    }

    #[test]
    fn the_automatic_memory_limit_is_half_the_ram_with_a_floor() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let auto = InstallerMemoryLimit::Auto;
        assert_eq!(auto.resolve(Some(32 * GIB)), Some(16 * 1024));
        assert_eq!(auto.resolve(Some(4 * GIB)), Some(4096));
        assert_eq!(auto.resolve(None), None);
        assert_eq!(InstallerMemoryLimit::Mib(3072).resolve(None), Some(3072));
        assert_eq!(InstallerMemoryLimit::Off.resolve(Some(GIB)), None);
        assert_eq!(InstallerMemoryLimit::Mib(0).resolve(Some(GIB)), None);
    }

    #[test]
    fn memory_limits_round_trip_through_their_stored_form() {
        for (limit, wire) in [
            (InstallerMemoryLimit::Auto, r#""auto""#),
            (InstallerMemoryLimit::Off, r#""off""#),
            (InstallerMemoryLimit::Mib(4096), "4096"),
        ] {
            assert_eq!(serde_json::to_string(&limit).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<InstallerMemoryLimit>(wire).unwrap(),
                limit
            );
        }
        assert!(serde_json::from_str::<InstallerMemoryLimit>(r#""lots""#).is_err());
    }

    #[test]
    fn roots_start_with_the_default_and_only_configured_ones_are_accepted() {
        let mut settings = Settings {
            library_root: Some("/games".into()),
            extra_library_roots: vec!["/mnt/big".into(), "/games".into(), "  ".into()],
            ..Default::default()
        };
        assert_eq!(settings.library_roots(), vec!["/games", "/mnt/big"]);
        assert_eq!(settings.require_root(None).unwrap(), "/games");
        assert!(settings.require_root(Some("/etc".into())).is_err());

        settings.set_library_roots(vec!["/mnt/big".into(), "/games".into()]);
        assert_eq!(settings.library_root.as_deref(), Some("/mnt/big"));
        assert!(Settings::default().library_roots().is_empty());
    }

    #[test]
    fn the_ignore_list_matches_the_file_name_not_the_path() {
        let settings = Settings::default();
        assert!(settings.is_ignored_executable("UnityCrashHandler64.exe"));
        assert!(settings.is_ignored_executable(r"bin\UnrealCEFSubProcess.exe"));
        assert!(!settings.is_ignored_executable("vcredist/Game.exe"));

        let blank = Settings {
            ignored_executables: vec![String::new(), "  ".into()],
            ..Default::default()
        };
        assert!(!blank.is_ignored_executable("Celeste.exe"));
    }
}
