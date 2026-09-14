//! Finding the tools needed to run Windows games on Linux, up front, so the app can name
//! what is missing rather than failing later with `os error 2`.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowsRuntime {
    /// umu-launcher running a Proton build inside the Steam Linux Runtime container. The
    /// default: Proton's own script, protonfixes and fsync, which is what games are tested on.
    Umu {
        /// The `umu-run` zipapp shipped beside the app.
        launcher: PathBuf,
        /// The Proton build directory, handed to umu as `PROTONPATH`.
        proton: PathBuf,
        /// The build's name, e.g. `UMU-Proton-10.0-4`.
        build: String,
    },
    /// Plain Wine. Works for many games, without Proton's patches or per-title fixes.
    Wine { path: PathBuf },
    /// Wine the app downloaded and owns. Preferred: the same version in every package
    /// format, and spawned by us, so no sandbox boundary for the environment to cross.
    Bundled { path: PathBuf },
    /// The host's Wine, reached from inside the Flatpak sandbox. Bundling a second copy
    /// would mean a second update cycle for one package format.
    HostWine,
}

impl WindowsRuntime {
    pub fn program(&self) -> &Path {
        match self {
            WindowsRuntime::Umu { launcher, .. } => launcher,
            WindowsRuntime::Wine { path } | WindowsRuntime::Bundled { path } => path,
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

    /// Whether the runtime runs outside this process, so variables must travel as arguments: a
    /// lost `WINEPREFIX` silently sends Wine to its default prefix.
    pub fn runs_on_host(&self) -> bool {
        matches!(self, WindowsRuntime::HostWine)
    }

    /// The full argument list for this runtime. Shared by launching and prefix preparation,
    /// so a fix to one cannot leave the other building a prefix somewhere else.
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

    /// Wine without Proton's script, which decides who manages DXVK, the Mono and Gecko
    /// prompts, and the prefix layout.
    pub fn is_wine_family(&self) -> bool {
        !matches!(self, WindowsRuntime::Umu { .. })
    }

    pub fn kind(&self) -> &'static str {
        match self {
            WindowsRuntime::Umu { .. } => "umu",
            WindowsRuntime::Wine { .. } => "wine",
            WindowsRuntime::Bundled { .. } => "bundled-wine",
            WindowsRuntime::HostWine => "host-wine",
        }
    }

    pub fn description(&self) -> String {
        match self {
            WindowsRuntime::Umu { build, .. } => format!("{build} (umu)"),
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

/// Proton builds installed through Steam, newest-looking first: Valve's own under
/// `steamapps/common` and community builds in `compatibilitytools.d`.
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

/// The helper that runs a command outside the sandbox. Public because recognising it is how
/// the launch layer knows our own process limits will not reach the program.
pub const FLATPAK_SPAWN: &str = "/usr/bin/flatpak-spawn";

pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Whether the host has Wine, asked from inside the sandbox.
fn host_has_wine() -> bool {
    host_has_program("wine")
}

/// Whether the host has a program on its `PATH`, asked from inside the sandbox. The name goes
/// into a shell, hence a fixed name and never user input.
pub fn host_has_program(name: &'static str) -> bool {
    std::process::Command::new(FLATPAK_SPAWN)
        .args(["--host", "sh", "-c"])
        .arg(format!("command -v {name}"))
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
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
    /// The Proton build a game or the settings asked for, by name.
    pub proton_build: Option<String>,
    /// The program about to run, when one is known.
    pub program: Option<PathBuf>,
    /// Whether this system can run 32-bit programs at all.
    pub supports_32bit: bool,
}

impl Default for RuntimeContext {
    /// 32-bit support is assumed until something says otherwise, so an unset context never
    /// silently rules the container out.
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
    /// from the system's libraries, which Wine's WoW64 build does not need.
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
        let mut builds = self
            .config_dir
            .as_deref()
            .map(crate::proton::installed)
            .unwrap_or_default();
        builds.extend(crate::proton::steam_builds());
        let build = crate::proton::pick(&builds, self.proton_build.as_deref())?;
        Some(WindowsRuntime::Umu {
            launcher,
            proton: build.path,
            build: build.name,
        })
    }
}

/// Detect the best available way to run Windows programs: umu when it can run and has a
/// Proton build, then Wine, the managed build first.
pub fn detect_windows_runtime(ctx: &RuntimeContext) -> Option<WindowsRuntime> {
    if ctx.container_can_run() {
        if let Some(umu) = ctx.umu() {
            return Some(umu);
        }
    }

    if let Some(installed) = ctx.config_dir.as_deref().and_then(crate::wine::installed) {
        return Some(WindowsRuntime::Bundled {
            path: installed.binary,
        });
    }

    if let Some(path) = find_program("wine") {
        return Some(WindowsRuntime::Wine { path });
    }

    // Inside a Flatpak, the host's Wine is reachable even though the sandbox has none.
    if in_flatpak() && host_has_wine() {
        return Some(WindowsRuntime::HostWine);
    }

    None
}

/// Finds one named runtime for a game that overrides the automatic choice. `None` is
/// reported rather than quietly run with something the user did not ask for.
pub fn find_windows_runtime(ctx: &RuntimeContext, kind: &str) -> Option<WindowsRuntime> {
    match kind {
        "umu" => ctx.umu(),
        "bundled-wine" => ctx
            .config_dir
            .as_deref()
            .and_then(crate::wine::installed)
            .map(|installed| WindowsRuntime::Bundled {
                path: installed.binary,
            }),
        "wine" => find_program("wine").map(|path| WindowsRuntime::Wine { path }),
        "host-wine" => (in_flatpak() && host_has_wine()).then_some(WindowsRuntime::HostWine),
        _ => detect_windows_runtime(ctx),
    }
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

/// Whether this system can start a 32-bit program at all, which the container needs and
/// fails silently without. Looked for once: a loader does not appear while the app runs.
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
                tracing::info!("no 32-bit loader here, so 32-bit games will run on Wine");
            }
            found
        })
    }
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

/// What to do when no Windows runtime is present. The managed download leads (works
/// everywhere, no root); a distro package and Steam Proton are offered after it.
pub fn windows_runtime_hint() -> String {
    let wine = install_command("wine");

    // Under Flatpak the command has to be run on the host: the sandbox has no package
    // manager, and running it in a terminal inside the sandbox would silently do nothing.
    let where_to_run = if in_flatpak() {
        " Run it on your computer, not inside the Flatpak."
    } else {
        ""
    };

    // The download leads: it is the only fix that works everywhere, Flatpak and atomic
    // distributions included.
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
            // The bare name stays available for a file that really has no extension.
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
    fn the_hint_leads_with_a_concrete_installable_command() {
        let hint = windows_runtime_hint();
        // Whatever the distribution, there must be something to actually type or visit.
        assert!(
            hint.contains("install") || hint.contains("github.com"),
            "got: {hint}"
        );
        // Fedora has no umu-launcher package, so leading with it sends people nowhere.
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
    fn each_runtime_reports_its_kind_program_and_prefix() {
        // HostWine is the only one that runs outside the sandbox, so it is the only one
        // whose program is the spawn helper rather than the runtime itself.
        let cases = [
            (
                WindowsRuntime::HostWine,
                "host-wine",
                PathBuf::from(FLATPAK_SPAWN),
                vec!["--host", "wine"],
            ),
            (
                WindowsRuntime::Wine {
                    path: PathBuf::from("/usr/bin/wine"),
                },
                "wine",
                PathBuf::from("/usr/bin/wine"),
                vec![],
            ),
            (
                WindowsRuntime::Umu {
                    launcher: PathBuf::from("/app/bin/umu-run"),
                    proton: PathBuf::from("/cfg/proton/UMU-Proton-10.0-4"),
                    build: "UMU-Proton-10.0-4".into(),
                },
                "umu",
                PathBuf::from("/app/bin/umu-run"),
                vec![],
            ),
        ];
        for (runtime, kind, program, prefix) in cases {
            assert_eq!(runtime.kind(), kind);
            assert_eq!(runtime.program(), program.as_path(), "{kind}");
            assert_eq!(runtime.prefix_args(), prefix, "{kind}");
        }
    }

    #[test]
    fn umu_is_described_by_the_proton_build_it_runs() {
        let runtime = WindowsRuntime::Umu {
            launcher: PathBuf::from("/app/bin/umu-run"),
            proton: PathBuf::from("/cfg/proton/GE-Proton11-6-x86_64"),
            build: "GE-Proton11-6-x86_64".into(),
        };
        assert_eq!(runtime.description(), "GE-Proton11-6-x86_64 (umu)");
        assert!(!runtime.is_wine_family());
        assert!(WindowsRuntime::HostWine.is_wine_family());
    }

    #[test]
    fn an_unknown_override_falls_back_to_the_automatic_choice() {
        // A settings file naming a runtime this build no longer has, such as the old
        // `steam-proton`, must not stop a launch.
        let ctx = RuntimeContext::default();
        assert_eq!(
            find_windows_runtime(&ctx, "steam-proton").is_some(),
            detect_windows_runtime(&ctx).is_some()
        );
    }

    #[test]
    fn an_override_never_returns_a_runtime_of_a_different_kind() {
        // Silently running under Wine when the user asked for Proton is the failure mode
        // this guards: it looks like the override did nothing.
        let ctx = RuntimeContext::default();
        for kind in ["bundled-wine", "wine", "host-wine", "umu"] {
            if let Some(found) = find_windows_runtime(&ctx, kind) {
                assert_eq!(found.kind(), kind);
            }
        }
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
        assert!(!matches!(
            detect_windows_runtime(&without_launcher),
            Some(WindowsRuntime::Umu { .. })
        ));

        // A launcher with nothing to run falls through, unless Steam happens to have Proton.
        if steam_proton_builds().is_empty() {
            let no_builds = RuntimeContext {
                config_dir: Some(config_with_builds("none", &[])),
                ..with_launcher
            };
            assert!(find_windows_runtime(&no_builds, "umu").is_none());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_32_bit_program_skips_the_container_but_an_explicit_choice_still_wins() {
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

        // Asked for by name, umu is still what the user gets: the setting must not be
        // quietly ignored.
        assert!(matches!(
            find_windows_runtime(&ctx, "umu"),
            Some(WindowsRuntime::Umu { .. })
        ));

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
        match find_windows_runtime(&ctx, "umu") {
            Some(WindowsRuntime::Umu { build, .. }) => assert_eq!(build, "GE-Proton11-6-x86_64"),
            other => panic!("expected the pinned build, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn steam_detection_does_not_invent_a_root() {
        // On a machine without Steam this must simply find nothing.
        if steam_root().is_none() {
            assert!(steam_proton_builds().is_empty());
        }
    }
}
