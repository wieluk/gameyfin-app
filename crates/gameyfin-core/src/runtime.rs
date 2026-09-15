//! Choosing how a Windows program runs on Linux: Proton through umu, or Gameyfin's own Wine for
//! what Proton cannot run on this system.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsRuntime {
    /// umu-launcher running a Proton build inside the Steam Runtime container.
    Umu {
        launcher: PathBuf,
        /// The Proton build directory, handed to umu as `PROTONPATH`.
        proton: PathBuf,
        /// The build's name, e.g. `UMU-Proton-10.0-4`.
        build: String,
    },
    /// Gameyfin's WoW64 Wine, for a 32-bit program without 32-bit libraries or when umu fails.
    Wine { path: PathBuf },
}

impl WindowsRuntime {
    pub fn program(&self) -> &Path {
        match self {
            WindowsRuntime::Umu { launcher, .. } => launcher,
            WindowsRuntime::Wine { path } => path,
        }
    }

    /// Wine without Proton's script, which otherwise manages DXVK, Mono and the prefix layout.
    pub fn is_wine(&self) -> bool {
        matches!(self, WindowsRuntime::Wine { .. })
    }

    pub fn kind(&self) -> &'static str {
        match self {
            WindowsRuntime::Umu { .. } => "umu",
            WindowsRuntime::Wine { .. } => "wine",
        }
    }

    pub fn description(&self) -> String {
        match self {
            WindowsRuntime::Umu { build, .. } => build.clone(),
            WindowsRuntime::Wine { .. } => "Wine".to_string(),
        }
    }
}

/// The helper that runs a command outside the sandbox.
pub const FLATPAK_SPAWN: &str = "/usr/bin/flatpak-spawn";

pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Locations worth checking beyond `PATH`, which a desktop-launched app often lacks.
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

/// The same, on Windows, where neither 7-Zip nor WinRAR puts itself on `PATH`.
#[cfg(windows)]
fn extra_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // All three cover every mix of installer and process bitness; `ProgramW6432` is the
    // 64-bit folder even from a 32-bit process.
    for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        let Some(base) = std::env::var_os(key).map(PathBuf::from) else {
            continue;
        };
        dirs.push(base.join("7-Zip"));
        dirs.push(base.join("WinRAR"));
        dirs.push(base.join("NanaZip"));
    }

    // 7-Zip and NanaZip can also be installed per-user, the likelier case on a shared machine.
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

/// Extensions a program name may carry on Windows. Not `PATHEXT`: only runnable binaries,
/// never a `.vbs`/`.js` that happens to sit on `PATH`.
#[cfg(windows)]
const WINDOWS_EXECUTABLE_EXTENSIONS: &[&str] = &[".exe", ".com", ".bat", ".cmd"];

/// The filenames to try for a program named `name` (on Windows, `7z.exe` not `7z`).
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

/// What runtime detection needs that only the app can find out.
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub config_dir: Option<PathBuf>,
    /// The bundled `umu-run`, set only once its preflight has passed.
    pub umu_launcher: Option<PathBuf>,
    /// The Proton build a game is pinned to, by name.
    pub proton_build: Option<String>,
    /// The program about to run, when one is known.
    pub program: Option<PathBuf>,
    pub supports_32bit: bool,
}

impl Default for RuntimeContext {
    /// 32-bit support is assumed, so an unset context never silently rules Proton out.
    fn default() -> Self {
        Self {
            config_dir: None,
            umu_launcher: None,
            proton_build: None,
            program: None,
            supports_32bit: true,
        }
    }
}

impl RuntimeContext {
    /// Whether the Steam Runtime container can run this program: it builds its 32-bit half
    /// from the system's libraries, which the WoW64 Wine does not need.
    fn container_can_run(&self) -> bool {
        if self.supports_32bit {
            return true;
        }
        let Some(program) = self.program.as_deref() else {
            return true;
        };
        if crate::executable::is_32bit_windows_program(program) {
            tracing::info!(
                ?program,
                "32-bit program on a system with no 32-bit libraries; using Wine instead"
            );
            return false;
        }
        true
    }

    /// umu with the build it would run, when there is both a launcher and a build.
    fn umu(&self) -> Option<WindowsRuntime> {
        let launcher = self.umu_launcher.clone()?;
        let builds = self
            .config_dir
            .as_deref()
            .map(crate::proton::installed)
            .unwrap_or_default();
        let build = crate::proton::pick(&builds, self.proton_build.as_deref())?;
        Some(WindowsRuntime::Umu {
            launcher,
            proton: build.path,
            build: build.name,
        })
    }
}

/// Proton when it can run this program, else Gameyfin's Wine once it is downloaded.
pub fn detect_windows_runtime(ctx: &RuntimeContext) -> Option<WindowsRuntime> {
    if ctx.container_can_run() {
        if let Some(umu) = ctx.umu() {
            return Some(umu);
        }
    }
    ctx.config_dir
        .as_deref()
        .and_then(crate::wine::installed)
        .map(|installed| WindowsRuntime::Wine {
            path: installed.binary,
        })
}

/// Where an i386 dynamic loader lives, across the layouts in use: the old `lib32` split,
/// Debian-style multiarch, and the two mount points a Flatpak's i386 extension takes.
#[cfg(unix)]
const I386_LOADERS: [&str; 7] = [
    "/lib/ld-linux.so.2",
    "/lib32/ld-linux.so.2",
    "/usr/lib/ld-linux.so.2",
    "/usr/lib32/ld-linux.so.2",
    "/lib/i386-linux-gnu/ld-linux.so.2",
    "/usr/lib/i386-linux-gnu/ld-linux.so.2",
    "/app/lib/i386-linux-gnu/ld-linux.so.2",
];

/// Whether this system can start a 32-bit program at all. Looked for once: a loader does not
/// appear while the app runs.
pub fn has_32bit_support() -> bool {
    #[cfg(not(unix))]
    {
        true
    }
    #[cfg(unix)]
    {
        static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *FOUND.get_or_init(|| {
            let found = I386_LOADERS.iter().any(|path| Path::new(path).exists());
            if !found {
                tracing::info!("no 32-bit loader here, so 32-bit programs will run on Wine");
            }
            found
        })
    }
}

/// Whether a Flatpak has 32-bit OpenGL drivers, which the GL32 extension mounts here. Outside
/// one they come from the system, which is not looked into.
pub fn has_32bit_graphics() -> bool {
    if !in_flatpak() {
        return true;
    }
    std::fs::read_dir("/app/lib/i386-linux-gnu/GL")
        .is_ok_and(|mut entries| entries.next().is_some())
}

/// The host's `os-release`. Inside a Flatpak `/etc/os-release` describes the runtime, not
/// the machine, so the host copy at `/run/host/os-release` is preferred.
fn host_os_release() -> String {
    if in_flatpak() {
        if let Ok(host) = std::fs::read_to_string("/run/host/os-release") {
            return host;
        }
    }
    std::fs::read_to_string("/etc/os-release").unwrap_or_default()
}

/// How this distribution installs a package, as a command the user can paste. Falls back
/// to naming the package alone rather than guessing the wrong package manager.
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

/// What to say when neither Proton nor Wine could be set up.
pub fn windows_runtime_hint() -> String {
    format!(
        "Windows games need Proton or Wine, and Gameyfin could not set up either. Check the \
         connection and try again. Proton also needs python3 3.10 or newer: {}.",
        install_command("python3")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_program_reports_only_what_is_really_there() {
        // `sh` is on PATH on every unix worth supporting.
        #[cfg(unix)]
        assert!(find_program("sh").is_some());
        assert_eq!(find_program("definitely-not-a-real-program-xyz"), None);
    }

    #[test]
    fn candidate_names_add_windows_extensions_to_a_bare_name_only() {
        let bare = candidate_names("7z");
        #[cfg(windows)]
        {
            assert!(bare.contains(&"7z.exe".to_string()), "got {bare:?}");
            assert!(bare.contains(&"7z".to_string()), "got {bare:?}");
        }
        #[cfg(not(windows))]
        assert_eq!(bare, vec!["7z".to_string()]);

        assert_eq!(candidate_names("7z.exe"), vec!["7z.exe".to_string()]);
    }

    #[test]
    fn the_install_command_names_the_package_it_was_asked_about() {
        let command = install_command("unar");
        assert!(command.contains("unar"), "got: {command}");
    }

    #[test]
    fn neither_a_directory_nor_an_unexecutable_file_counts_as_executable() {
        assert!(!is_executable_file(&std::env::temp_dir()));

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
    fn the_hint_names_something_to_install() {
        let hint = windows_runtime_hint();
        assert!(hint.contains("python3"), "got: {hint}");
    }

    #[test]
    fn umu_is_described_by_the_proton_build_it_runs() {
        let runtime = WindowsRuntime::Umu {
            launcher: PathBuf::from("/app/bin/umu-run"),
            proton: PathBuf::from("/cfg/proton/GE-Proton11-6-x86_64"),
            build: "GE-Proton11-6-x86_64".into(),
        };
        assert_eq!(runtime.description(), "GE-Proton11-6-x86_64");
        assert!(!runtime.is_wine());
        assert!(WindowsRuntime::Wine {
            path: PathBuf::from("/wine")
        }
        .is_wine());
    }

    /// A config directory holding managed Proton builds, each just a `proton` script.
    fn config_with_builds(name: &str, builds: &[&str]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-rtctx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for build in builds {
            let path = crate::proton::proton_root(&dir).join(build);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("proton"), "").unwrap();
        }
        dir
    }

    #[test]
    fn umu_is_chosen_only_with_both_a_launcher_and_a_proton_build() {
        let dir = config_with_builds("both", &["UMU-Proton-10.0-4"]);
        let with_launcher = RuntimeContext {
            config_dir: Some(dir.clone()),
            umu_launcher: Some(PathBuf::from("/app/bin/umu-run")),
            ..RuntimeContext::default()
        };
        match detect_windows_runtime(&with_launcher) {
            Some(WindowsRuntime::Umu { proton, build, .. }) => {
                assert_eq!(build, "UMU-Proton-10.0-4");
                assert_eq!(
                    proton,
                    crate::proton::proton_root(&dir).join("UMU-Proton-10.0-4")
                );
            }
            other => panic!("expected umu, got {other:?}"),
        }

        // No launcher means its preflight failed, so a build alone must not pick umu.
        let without_launcher = RuntimeContext {
            umu_launcher: None,
            ..with_launcher.clone()
        };
        assert_eq!(detect_windows_runtime(&without_launcher), None);

        let no_builds = RuntimeContext {
            config_dir: Some(config_with_builds("none", &[])),
            ..with_launcher
        };
        assert_eq!(detect_windows_runtime(&no_builds), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_32_bit_program_without_32_bit_libraries_skips_the_container() {
        let dir = config_with_builds("thirty-two", &["UMU-Proton-10.0-4"]);
        let mut bytes = vec![0u8; 0x100];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014cu16.to_le_bytes());
        let setup = dir.join("setup.exe");
        std::fs::write(&setup, bytes).unwrap();

        let ctx = RuntimeContext {
            config_dir: Some(dir.clone()),
            umu_launcher: Some(PathBuf::from("/app/bin/umu-run")),
            program: Some(setup),
            supports_32bit: false,
            ..RuntimeContext::default()
        };
        assert!(
            !matches!(
                detect_windows_runtime(&ctx),
                Some(WindowsRuntime::Umu { .. })
            ),
            "an Inno Setup stub cannot run in a container with no 32-bit libraries"
        );

        // The same machine runs a 64-bit game in the container as usual.
        let ctx64 = RuntimeContext {
            program: None,
            ..ctx
        };
        assert!(matches!(
            detect_windows_runtime(&ctx64),
            Some(WindowsRuntime::Umu { .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pinned_proton_build_reaches_the_runtime() {
        let dir = config_with_builds("pinned", &["UMU-Proton-10.0-4", "GE-Proton11-6-x86_64"]);
        let ctx = RuntimeContext {
            config_dir: Some(dir.clone()),
            umu_launcher: Some(PathBuf::from("/app/bin/umu-run")),
            proton_build: Some("GE-Proton11-6-x86_64".into()),
            ..RuntimeContext::default()
        };
        match detect_windows_runtime(&ctx) {
            Some(WindowsRuntime::Umu { build, .. }) => assert_eq!(build, "GE-Proton11-6-x86_64"),
            other => panic!("expected the pinned build, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
