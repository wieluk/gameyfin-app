//! Finding the tools needed to run Windows games on Linux, up front, so the app can name
//! what is missing rather than failing later with `os error 2`.

use std::path::{Path, PathBuf};

/// A way of running Windows programs on Linux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsRuntime {
    /// umu-launcher: Proton plus the Steam runtime. The best option, and what Lutris and
    /// Heroic use for non-Steam titles.
    Umu { path: PathBuf },
    /// A Proton build that came with Steam.
    ///
    /// Worth finding because most people running Windows games on Linux already have
    /// Steam, and therefore already have Proton, no install step at all. Proton needs
    /// two Steam-specific environment variables, which is why it is not simply "wine".
    SteamProton {
        /// The `proton` script inside the build.
        script: PathBuf,
        /// Steam's root, for `STEAM_COMPAT_CLIENT_INSTALL_PATH`.
        steam_root: PathBuf,
        /// Human-readable build name, e.g. `GE-Proton9-20`.
        name: String,
    },
    /// Plain Wine. Works for many games, without Proton's patches or per-title fixes.
    Wine { path: PathBuf },
    /// Wine the app downloaded and owns, under its own config directory.
    ///
    /// Preferred over everything else: it is the same version in every package format, it
    /// needs nothing installed on the host, and because we spawn it ourselves there is no
    /// sandbox boundary for environment variables or process limits to fail to cross.
    Bundled { path: PathBuf },
    /// Wine on the host, reached from inside a Flatpak sandbox.
    ///
    /// Bundling Wine in the Flatpak would mean maintaining a second copy with its own
    /// update cycle; using the host's keeps every package format on the same runtime, so
    /// there is one thing to install and one thing that can go wrong.
    HostWine,
}

impl WindowsRuntime {
    pub fn program(&self) -> &Path {
        match self {
            WindowsRuntime::Umu { path }
            | WindowsRuntime::Wine { path }
            | WindowsRuntime::Bundled { path } => path,
            WindowsRuntime::SteamProton { script, .. } => script,
            // Commands are routed through `flatpak-spawn`, which runs on the host.
            WindowsRuntime::HostWine => Path::new(FLATPAK_SPAWN),
        }
    }

    /// Arguments that must precede the program when invoking this runtime.
    pub fn prefix_args(&self) -> Vec<String> {
        match self {
            WindowsRuntime::HostWine => {
                vec!["--host".to_string(), "wine".to_string()]
            }
            _ => Vec::new(),
        }
    }

    /// Whether this runtime executes outside the current process's environment.
    ///
    /// True for a Wine reached through `flatpak-spawn`: the command runs on the host, and
    /// variables set on our own process do not necessarily cross that boundary. They have
    /// to be passed as arguments instead, a `WINEPREFIX` that fails to arrive sends Wine
    /// to its default prefix, where it succeeds while leaving ours empty.
    pub fn runs_on_host(&self) -> bool {
        matches!(self, WindowsRuntime::HostWine)
    }

    /// Build the full argument list for invoking this runtime.
    ///
    /// Shared by launching and by prefix preparation so the two cannot drift: fixing the
    /// environment handling in one and not the other is exactly how a prefix ends up
    /// being built in the wrong place.
    pub fn wrap_args<I, S>(
        &self,
        env: &std::collections::BTreeMap<String, String>,
        working_dir: Option<&Path>,
        args: I,
    ) -> Vec<std::ffi::OsString>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString>,
    {
        use std::ffi::OsString;

        let mut out: Vec<OsString> = Vec::new();
        let prefix_args = self.prefix_args();

        if self.runs_on_host() && !prefix_args.is_empty() {
            // `--host` first, then the environment and working directory, then the
            // program the helper should run.
            out.push(OsString::from(&prefix_args[0]));
            for (key, value) in env {
                out.push(OsString::from(format!("--env={key}={value}")));
            }
            if let Some(dir) = working_dir {
                out.push(OsString::from(format!("--directory={}", dir.display())));
            }
            out.extend(prefix_args[1..].iter().map(OsString::from));
        } else {
            out.extend(prefix_args.iter().map(OsString::from));
        }

        out.extend(args.into_iter().map(Into::into));
        out
    }

    pub fn kind(&self) -> &'static str {
        match self {
            WindowsRuntime::Umu { .. } => "umu",
            WindowsRuntime::SteamProton { .. } => "steam-proton",
            WindowsRuntime::Wine { .. } => "wine",
            WindowsRuntime::Bundled { .. } => "bundled-wine",
            WindowsRuntime::HostWine => "host-wine",
        }
    }

    /// What to show the user.
    pub fn description(&self) -> String {
        match self {
            WindowsRuntime::Umu { .. } => "umu-launcher (Proton)".to_string(),
            WindowsRuntime::SteamProton { name, .. } => format!("{name} (from Steam)"),
            WindowsRuntime::Wine { .. } => "Wine".to_string(),
            WindowsRuntime::Bundled { .. } => "Wine (managed by Gameyfin)".to_string(),
            WindowsRuntime::HostWine => "Wine (on the host)".to_string(),
        }
    }
}

/// Steam's root directory, wherever this system keeps it.
pub fn steam_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        home.join(".steam/root"),
    ]
    .into_iter()
    .find(|path| path.join("steamapps").is_dir())
}

/// Proton builds installed through Steam, newest-looking first.
///
/// Covers both Valve's own builds under `steamapps/common` and community builds such as
/// GE-Proton dropped into `compatibilitytools.d`.
pub fn steam_proton_builds() -> Vec<(String, PathBuf)> {
    let Some(root) = steam_root() else {
        return Vec::new();
    };

    let mut builds = Vec::new();
    for dir in [
        root.join("steamapps/common"),
        root.join("compatibilitytools.d"),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let script = path.join("proton");
            if script.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                builds.push((name, script));
            }
        }
    }

    // Sorting by name puts higher version numbers last for a given series, so reversing
    // gives a reasonable "newest" without parsing Valve's inconsistent version strings.
    builds.sort_by(|a, b| a.0.cmp(&b.0));
    builds.reverse();
    builds
}

/// The helper that runs a command outside the Flatpak sandbox.
///
/// Public because recognising it is what tells the launch layer that a command crosses
/// the sandbox boundary, and therefore that our own process limits will not reach it.
pub const FLATPAK_SPAWN: &str = "/usr/bin/flatpak-spawn";

/// Whether this process is running inside a Flatpak sandbox.
pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Whether the host has Wine, asked from inside the sandbox.
fn host_has_wine() -> bool {
    std::process::Command::new(FLATPAK_SPAWN)
        .args(["--host", "sh", "-c", "command -v wine"])
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

/// Locations worth checking beyond `PATH`.
///
/// A Flatpak or a user-local pip install puts `umu-run` somewhere the desktop session's
/// `PATH` may not include, particularly when the app is launched from a desktop entry
/// rather than a shell.
#[cfg(not(windows))]
fn extra_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/app/bin"),
        PathBuf::from("/var/lib/flatpak/exports/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".local/share/flatpak/exports/bin"));
    }
    dirs
}

/// The same, on Windows.
///
/// Neither 7-Zip nor WinRAR puts itself on `PATH`, so a machine with 7-Zip installed
/// still found nothing by name alone and every RAR download failed with "install unar".
/// Both install to a predictable folder, which is what makes "install 7-Zip and try
/// again" advice the user can actually act on.
#[cfg(windows)]
fn extra_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // `ProgramW6432` is the 64-bit folder even when this process is 32-bit; the other two
    // are what a 64-bit process sees. Listing all three costs a few `stat` calls and
    // covers every combination of installer and host bitness.
    for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        let Some(base) = std::env::var_os(key).map(PathBuf::from) else {
            continue;
        };
        dirs.push(base.join("7-Zip"));
        dirs.push(base.join("WinRAR"));
        dirs.push(base.join("NanaZip"));
    }

    // 7-Zip and NanaZip can also be installed per-user, without administrator rights,
    // which is the likelier case on a machine the user does not own.
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        dirs.push(local.join("Programs").join("7-Zip"));
        dirs.push(local.join("Programs").join("NanaZip"));
    }

    // `tar.exe`, which is libarchive, has shipped with Windows since 10 1803.
    if let Some(root) = std::env::var_os("SystemRoot").map(PathBuf::from) {
        dirs.push(root.join("System32"));
    }

    dirs
}

/// Extensions a program name may carry on Windows.
///
/// Deliberately not `PATHEXT`, which also lists `.VBS`, `.JS` and `.WSF`. Everything here
/// is looked up so it can be *run*, and the difference between finding `7z.exe` and
/// finding some `7z.vbs` that happens to sit on `PATH` is worth keeping.
#[cfg(windows)]
const WINDOWS_EXECUTABLE_EXTENSIONS: &[&str] = &[".exe", ".com", ".bat", ".cmd"];

/// The filenames to try for a program named `name`.
///
/// On Windows a program is `7z.exe`, not `7z`: joining the bare name onto a directory
/// matches nothing, which is why every external tool this app looks for was reported
/// missing there however it had been installed.
fn candidate_names(name: &str) -> Vec<String> {
    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }

    #[cfg(windows)]
    {
        // A name that already carries an extension is taken as written.
        if Path::new(name).extension().is_some() {
            return vec![name.to_string()];
        }

        let mut names: Vec<String> = WINDOWS_EXECUTABLE_EXTENSIONS
            .iter()
            .map(|extension| format!("{name}{extension}"))
            .collect();
        // Last, in case the file genuinely has no extension.
        names.push(name.to_string());
        names
    }
}

/// Find an executable by name, on `PATH` and in the usual extra places.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let names = candidate_names(name);
    let on_path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();

    on_path
        .into_iter()
        .chain(extra_search_dirs())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Detect the best available way to run Windows programs.
///
/// Kept for callers with no config directory to hand; it simply cannot see a downloaded
/// Wine. Prefer [`detect_windows_runtime_in`].
pub fn detect_windows_runtime() -> Option<WindowsRuntime> {
    detect_windows_runtime_in(None)
}

/// Detect the best available way to run Windows programs, including one we downloaded.
///
/// The Wine this app manages wins over everything else when it is present. That is the
/// whole point of downloading it: one version, identical across the deb, rpm, AppImage and
/// Flatpak, so a bug report describes the same runtime every time, and no dependence on
/// what the host does or does not have installed.
pub fn detect_windows_runtime_in(config_dir: Option<&Path>) -> Option<WindowsRuntime> {
    if let Some(installed) = config_dir.and_then(crate::wine::installed) {
        return Some(WindowsRuntime::Bundled {
            path: installed.binary,
        });
    }

    // Then a system Wine, deliberately ahead of Proton.
    //
    // Proton has better per-title compatibility, but it arrives through umu or Steam,
    // each with its own installation, download and failure modes, and umu inside a
    // Flatpak has to bring a Python stack with it. Preferring Wine means one runtime
    // across every package format, installable from any distribution's repositories,
    // with one thing to diagnose when it breaks. Proton is still used when Wine is
    // absent and it happens to be there.
    if let Some(path) = find_program("wine") {
        return Some(WindowsRuntime::Wine { path });
    }

    // Inside a Flatpak, the host's Wine is reachable even though the sandbox has none.
    if in_flatpak() && host_has_wine() {
        return Some(WindowsRuntime::HostWine);
    }

    if let Some(path) = find_program("umu-run") {
        return Some(WindowsRuntime::Umu { path });
    }

    if let (Some(steam_root), Some((name, script))) =
        (steam_root(), steam_proton_builds().into_iter().next())
    {
        return Some(WindowsRuntime::SteamProton {
            script,
            steam_root,
            name,
        });
    }

    None
}

/// The distribution's `os-release`, as seen from wherever this app is installed.
///
/// Inside a Flatpak, `/etc/os-release` describes the *runtime*, a GNOME platform image,
/// and not the machine the user would be typing a package command on. Flatpak mounts the
/// host's copy at `/run/host/os-release` for exactly this reason, and without it a Fedora
/// user gets told to install nothing in particular.
fn host_os_release() -> String {
    if in_flatpak() {
        if let Ok(host) = std::fs::read_to_string("/run/host/os-release") {
            return host;
        }
    }
    std::fs::read_to_string("/etc/os-release").unwrap_or_default()
}

/// How this distribution installs a package, as a command the user can paste.
///
/// Shared by every "you need to install X" message, so a Fedora user is never told to
/// run `apt`. Falls back to naming the package without a command rather than guessing,
/// which is worse than saying nothing.
pub fn install_command(package: &str) -> String {
    let distro = host_os_release();
    let id_line = distro
        .lines()
        .find(|l| l.starts_with("ID="))
        .unwrap_or("")
        .to_ascii_lowercase();

    if id_line.contains("fedora") || id_line.contains("nobara") {
        format!("sudo dnf install {package}")
    } else if id_line.contains("arch") || id_line.contains("cachyos") || id_line.contains("manjaro")
    {
        format!("sudo pacman -S {package}")
    } else if id_line.contains("debian")
        || id_line.contains("ubuntu")
        || id_line.contains("mint")
        || id_line.contains("pop")
    {
        format!("sudo apt install {package}")
    } else if id_line.contains("opensuse") || id_line.contains("suse") {
        format!("sudo zypper install {package}")
    } else {
        format!("install {package} from your distribution")
    }
}

/// What to do when no Windows runtime is present.
///
/// The managed download leads because it is the only fix that works everywhere, needs no
/// root, and is unaffected by the Flatpak sandbox having no package manager. A
/// distribution package and an existing Steam Proton are offered after it, for anyone who
/// would rather not have a second Wine on disk.
pub fn windows_runtime_hint() -> String {
    let wine = install_command("wine");

    // Under Flatpak the command has to be run on the host: the sandbox has no package
    // manager, and running it in a terminal inside the sandbox would silently do nothing.
    let where_to_run = if in_flatpak() {
        " Run it on your computer, not inside the Flatpak."
    } else {
        ""
    };

    // The download is offered first because it is the only fix that works everywhere,
    // including a Flatpak and an atomic distribution where "install wine" means layering
    // an rpm and rebooting.
    format!(
        "No way to run Windows programs was found. Gameyfin can download Wine for you \
         from Settings, which works on every system and needs no administrator rights. \
         To use your distribution's Wine instead: {wine}.{where_to_run} \
         If you use Steam, installing any game's Proton also works and this app will find \
         it automatically."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_program_that_exists_on_path() {
        // `sh` is on PATH on every unix worth supporting.
        #[cfg(unix)]
        assert!(find_program("sh").is_some());
    }

    #[test]
    fn does_not_invent_a_missing_program() {
        assert_eq!(find_program("definitely-not-a-real-program-xyz"), None);
    }

    #[test]
    fn a_bare_name_gains_windows_extensions() {
        let names = candidate_names("7z");
        #[cfg(windows)]
        {
            assert!(names.contains(&"7z.exe".to_string()), "got {names:?}");
            // The bare name stays available for a file that really has no extension.
            assert!(names.contains(&"7z".to_string()), "got {names:?}");
        }
        #[cfg(not(windows))]
        assert_eq!(names, vec!["7z".to_string()]);
    }

    #[test]
    fn a_name_with_an_extension_is_taken_as_written() {
        assert_eq!(candidate_names("7z.exe"), vec!["7z.exe".to_string()]);
    }

    #[test]
    fn the_install_command_names_the_package_it_was_asked_about() {
        let command = install_command("unar");
        assert!(command.contains("unar"), "got: {command}");
    }

    #[test]
    fn a_directory_is_not_an_executable() {
        assert!(!is_executable_file(&std::env::temp_dir()));
    }

    #[test]
    fn a_non_executable_file_is_rejected() {
        let path = std::env::temp_dir().join(format!("gameyfin-rt-{}", std::process::id()));
        std::fs::write(&path, b"data").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(!is_executable_file(&path));
        }
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn the_hint_names_a_concrete_command() {
        let hint = windows_runtime_hint();
        // Whatever the distribution, there must be something to actually type or visit.
        assert!(
            hint.contains("install") || hint.contains("github.com"),
            "got: {hint}"
        );
    }

    #[test]
    fn the_hint_leads_with_something_actually_installable() {
        // Fedora has no umu-launcher package, so leading with it sends people nowhere.
        let hint = windows_runtime_hint();
        assert!(hint.to_lowercase().contains("wine"), "got: {hint}");
    }

    #[test]
    fn host_wine_routes_through_the_sandbox_helper() {
        let runtime = WindowsRuntime::HostWine;
        assert_eq!(runtime.kind(), "host-wine");
        assert_eq!(runtime.program(), Path::new(FLATPAK_SPAWN));
        assert_eq!(runtime.prefix_args(), vec!["--host", "wine"]);
    }

    #[test]
    fn host_wine_carries_its_environment_in_the_argument_list() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), "/games/Prefixes/93".to_string());

        let args = WindowsRuntime::HostWine.wrap_args(&env, None, ["wineboot", "-u"]);
        let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into()).collect();

        assert_eq!(args.first().unwrap(), "--host");
        assert!(args.contains(&"--env=WINEPREFIX=/games/Prefixes/93".to_string()));
        let wine_at = args.iter().position(|a| a == "wine").expect("wine");
        let env_at = args
            .iter()
            .position(|a| a.starts_with("--env="))
            .expect("env");
        assert!(env_at < wine_at);
        assert_eq!(args.last().unwrap(), "-u");
    }

    #[test]
    fn a_local_runtime_passes_arguments_through_untouched() {
        let env = std::collections::BTreeMap::new();
        let runtime = WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let args = runtime.wrap_args(&env, None, ["wineboot", "-u"]);
        assert_eq!(args, vec!["wineboot", "-u"]);
        assert!(!runtime.runs_on_host());
    }

    #[test]
    fn other_runtimes_need_no_prefix_arguments() {
        assert!(WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine")
        }
        .prefix_args()
        .is_empty());
    }

    #[test]
    fn steam_proton_is_described_by_its_build_name() {
        let runtime = WindowsRuntime::SteamProton {
            script: PathBuf::from("/steam/GE-Proton9-20/proton"),
            steam_root: PathBuf::from("/steam"),
            name: "GE-Proton9-20".into(),
        };
        assert_eq!(runtime.kind(), "steam-proton");
        assert_eq!(runtime.description(), "GE-Proton9-20 (from Steam)");
    }

    #[test]
    fn steam_detection_does_not_invent_a_root() {
        // On a machine without Steam this must simply find nothing.
        if steam_root().is_none() {
            assert!(steam_proton_builds().is_empty());
        }
    }

    #[test]
    fn runtime_reports_its_kind_and_program() {
        let umu = WindowsRuntime::Umu {
            path: PathBuf::from("/usr/bin/umu-run"),
        };
        assert_eq!(umu.kind(), "umu");
        assert_eq!(umu.program(), Path::new("/usr/bin/umu-run"));
    }
}
