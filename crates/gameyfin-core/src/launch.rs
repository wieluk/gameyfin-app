//! Launching games.
//!
//! Windows and native Linux builds are executed directly. Windows games on Linux go
//! through Wine: one runtime for every package format, available from any distribution's
//! repositories, so there is one thing for a user to install and one thing to diagnose.
//! Proton, through umu-launcher or a Steam build, is used only when Wine is absent and
//! it happens to be present. Each game gets its own Wine prefix so one game's
//! configuration or crash cannot disturb another's.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Proton build used when the game does not specify one.
pub const DEFAULT_PROTON: &str = "GE-Proton";

/// Identifier `umu-run` falls back to when a game has no known per-title fixes.
pub const DEFAULT_UMU_ID: &str = "umu-default";

/// How a game should be started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Runtime {
    /// Run the binary directly: a Windows game on Windows, or a native Linux build.
    Native,
    /// Run a Windows game on Linux through umu-launcher and Proton.
    Proton {
        prefix: PathBuf,
        /// Proton build, e.g. `GE-Proton` or `GE-Proton9-20`.
        proton: String,
        /// umu game id, used to apply per-title fixes.
        umu_id: String,
        /// Path to `umu-run`. Resolved rather than assumed to be on `PATH`: a desktop
        /// entry does not inherit a shell's `PATH`, and a Flatpak install puts it
        /// somewhere else again.
        launcher: PathBuf,
    },
    /// Run a Windows game through a Proton build installed by Steam.
    ///
    /// Proton is invoked as `proton run <exe>` and reads its configuration from two
    /// Steam-specific variables rather than `WINEPREFIX`.
    SteamProton {
        prefix: PathBuf,
        script: PathBuf,
        steam_root: PathBuf,
    },
    /// Run a Windows game on Linux through plain Wine.
    ///
    /// The default on Linux: one runtime for every package format, from any
    /// distribution's repositories.
    Wine {
        prefix: PathBuf,
        wine: PathBuf,
        /// Arguments that must precede the program, for a Wine reached indirectly,
        /// `flatpak-spawn --host wine` from inside a sandbox.
        prefix_args: Vec<String>,
    },
}

/// Everything needed to start one game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchConfig {
    pub executable: PathBuf,
    /// Working directory; defaults to the executable's own directory.
    pub working_dir: Option<PathBuf>,
    pub arguments: Vec<String>,
    /// Extra environment variables, e.g. `DXVK_HUD` or `PROTON_USE_WINED3D`.
    pub environment: BTreeMap<String, String>,
    pub runtime: Runtime,
}

impl LaunchConfig {
    pub fn native(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            working_dir: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            runtime: Runtime::Native,
        }
    }

    pub fn proton(
        executable: impl Into<PathBuf>,
        prefix: impl Into<PathBuf>,
        launcher: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            working_dir: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            runtime: Runtime::Proton {
                prefix: prefix.into(),
                proton: DEFAULT_PROTON.to_string(),
                umu_id: DEFAULT_UMU_ID.to_string(),
                launcher: launcher.into(),
            },
        }
    }

    pub fn wine(
        executable: impl Into<PathBuf>,
        prefix: impl Into<PathBuf>,
        wine: impl Into<PathBuf>,
    ) -> Self {
        Self::wine_with(executable, prefix, wine, Vec::new())
    }

    pub fn wine_with(
        executable: impl Into<PathBuf>,
        prefix: impl Into<PathBuf>,
        wine: impl Into<PathBuf>,
        prefix_args: Vec<String>,
    ) -> Self {
        Self {
            executable: executable.into(),
            working_dir: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            runtime: Runtime::Wine {
                prefix: prefix.into(),
                wine: wine.into(),
                prefix_args,
            },
        }
    }

    /// Build a config for a Windows program using whatever runtime the machine has.
    pub fn for_windows_program(
        executable: impl Into<PathBuf>,
        prefix: impl Into<PathBuf>,
        runtime: &crate::runtime::WindowsRuntime,
    ) -> Self {
        match runtime {
            crate::runtime::WindowsRuntime::Umu { path } => {
                Self::proton(executable, prefix, path.clone())
            }
            crate::runtime::WindowsRuntime::SteamProton {
                script, steam_root, ..
            } => Self {
                executable: executable.into(),
                working_dir: None,
                arguments: Vec::new(),
                environment: BTreeMap::new(),
                runtime: Runtime::SteamProton {
                    prefix: prefix.into(),
                    script: script.clone(),
                    steam_root: steam_root.clone(),
                },
            },
            // A downloaded Wine is plain Wine that happens to live in our own directory:
            // same invocation, same prefix handling, no wrapper.
            crate::runtime::WindowsRuntime::Wine { path }
            | crate::runtime::WindowsRuntime::Bundled { path } => {
                Self::wine(executable, prefix, path.clone())
            }
            crate::runtime::WindowsRuntime::HostWine => Self::wine_with(
                executable,
                prefix,
                runtime.program().to_path_buf(),
                runtime.prefix_args(),
            ),
        }
    }

    /// Same, with Wine's mscoree/mshtml prompts suppressed.
    ///
    /// Those dialogs open behind the game window and block the prefix update until
    /// answered, which is indistinguishable from a hang.
    pub fn for_windows_program_unattended(
        executable: impl Into<PathBuf>,
        prefix: impl Into<PathBuf>,
        runtime: &crate::runtime::WindowsRuntime,
    ) -> Self {
        let mut config = Self::for_windows_program(executable, prefix, runtime);
        config.environment.insert(
            "WINEDLLOVERRIDES".to_string(),
            crate::prefix::NO_PROMPTS.to_string(),
        );
        config
    }

    /// Directory the process should start in.
    pub fn resolved_working_dir(&self) -> Option<PathBuf> {
        self.working_dir
            .clone()
            .or_else(|| self.executable.parent().map(Path::to_path_buf))
    }
}

/// A command line, resolved from a [`LaunchConfig`] but not yet run.
///
/// Separating resolution from spawning is what makes launching testable: the exact
/// program, arguments and environment can be asserted without starting a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub env: BTreeMap<String, String>,
    pub working_dir: Option<PathBuf>,
}

/// How an address-space cap reached the process it was meant for.
///
/// Two mechanisms, because there are two kinds of spawn, and picking the wrong one fails
/// silently. The cap is still applied to *something*, just not to the process that needed
/// it, so there is no error to notice and nothing in any log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceCap {
    /// Set with `setrlimit` between `fork` and `exec`, and inherited by the whole tree.
    BeforeExec(u64),
    /// Carried in the command line, for a program spawned on the far side of a sandbox
    /// boundary that our own limits do not cross.
    InCommandLine,
}

impl AddressSpaceCap {
    /// The limit to apply before `exec`, if that is how this cap travels.
    pub fn before_exec(self) -> Option<u64> {
        match self {
            AddressSpaceCap::BeforeExec(bytes) => Some(bytes),
            AddressSpaceCap::InCommandLine => None,
        }
    }

    /// What to call this in a log line.
    pub fn label(self) -> &'static str {
        match self {
            AddressSpaceCap::BeforeExec(_) => "setrlimit",
            AddressSpaceCap::InCommandLine => "ulimit on the host",
        }
    }
}

impl ResolvedCommand {
    /// Whether this command starts its program outside our own process tree.
    ///
    /// `flatpak-spawn` does not run the program it is given: it sends a D-Bus request and
    /// `flatpak-session-helper` spawns it on the host. Nothing we set on our own child
    /// crosses that gap.
    pub fn runs_on_host(&self) -> bool {
        Path::new(&self.program) == Path::new(crate::runtime::FLATPAK_SPAWN)
    }

    /// Cap the address space of the program this command starts.
    ///
    /// Normally we fork and exec ourselves, so a `setrlimit` between the two is inherited
    /// by the whole tree ([`AddressSpaceCap::BeforeExec`]). Under Flatpak, Wine is reached
    /// through `flatpak-spawn` and is never forked by us: an rlimit on our child would
    /// land on the D-Bus helper stub while Wine inherits the host session helper's limits.
    /// There the cap has to travel inside the command instead.
    pub fn cap_address_space(&mut self, bytes: u64) -> AddressSpaceCap {
        if !self.runs_on_host() {
            return AddressSpaceCap::BeforeExec(bytes);
        }

        // Everything up to the first non-option argument belongs to `flatpak-spawn`
        // itself, `--host`, `--env=`, `--directory=`. What follows is the command it
        // should run on the host, and that is the part to wrap.
        let split = self
            .args
            .iter()
            .position(|a| !a.to_string_lossy().starts_with("--"))
            .unwrap_or(self.args.len());

        // `ulimit -v` takes kibibytes and sets `RLIMIT_AS`. `exec` replaces the shell with
        // the program, so the limit lands on Wine itself and no shell is left in the tree.
        // A shell that refuses the limit still execs: that leaves the installer uncapped
        // but running, with the refusal captured in its output rather than a failed
        // install.
        let script = format!("ulimit -v {}; exec \"$0\" \"$@\"", bytes / 1024);

        let mut wrapped: Vec<OsString> = self.args[..split].to_vec();
        wrapped.push(OsString::from("sh"));
        wrapped.push(OsString::from("-c"));
        wrapped.push(OsString::from(script));
        // The first argument after the script becomes `$0`, the rest `$@`.
        wrapped.extend_from_slice(&self.args[split..]);
        self.args = wrapped;

        AddressSpaceCap::InCommandLine
    }
}

/// Build the command for a launch configuration.
///
/// Note this never goes through a shell. The existing Python client builds a string and
/// runs it with `/bin/sh -c`, which means a game installed under a path containing a
/// quote or a space is a quoting bug waiting to happen; passing an argument vector
/// removes that entire class of problem.
pub fn resolve_command(config: &LaunchConfig) -> CoreResult<ResolvedCommand> {
    if config.executable.as_os_str().is_empty() {
        return Err(CoreError::Other("no executable configured".into()));
    }

    let working_dir = config.resolved_working_dir();
    let mut env = config.environment.clone();

    match &config.runtime {
        Runtime::Native => Ok(ResolvedCommand {
            program: config.executable.clone().into_os_string(),
            args: config.arguments.iter().map(OsString::from).collect(),
            env,
            working_dir,
        }),

        Runtime::SteamProton {
            prefix,
            script,
            steam_root,
        } => {
            if prefix.as_os_str().is_empty() {
                return Err(CoreError::Other("Proton runtime needs a prefix".into()));
            }

            // Proton creates `pfx` inside the compat data path, so the prefix directory
            // is handed over whole rather than as WINEPREFIX.
            env.insert(
                "STEAM_COMPAT_DATA_PATH".to_string(),
                prefix.to_string_lossy().into_owned(),
            );
            env.insert(
                "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(),
                steam_root.to_string_lossy().into_owned(),
            );

            let mut args = vec![
                OsString::from("run"),
                config.executable.clone().into_os_string(),
            ];
            args.extend(config.arguments.iter().map(OsString::from));

            Ok(ResolvedCommand {
                program: script.clone().into_os_string(),
                args,
                env,
                working_dir,
            })
        }

        Runtime::Wine {
            prefix,
            wine,
            prefix_args,
        } => {
            if prefix.as_os_str().is_empty() {
                return Err(CoreError::Other("Wine runtime needs a prefix".into()));
            }
            env.insert(
                "WINEPREFIX".to_string(),
                prefix.to_string_lossy().into_owned(),
            );

            // The same wrapper prefix preparation uses, so the two cannot drift.
            let mut program_args: Vec<OsString> = vec![config.executable.clone().into_os_string()];
            program_args.extend(config.arguments.iter().map(OsString::from));

            let runtime_for_wrap = if prefix_args.is_empty() {
                crate::runtime::WindowsRuntime::Wine { path: wine.clone() }
            } else {
                crate::runtime::WindowsRuntime::HostWine
            };
            let args = runtime_for_wrap.wrap_args(&env, working_dir.as_deref(), program_args);

            Ok(ResolvedCommand {
                program: wine.clone().into_os_string(),
                args,
                env,
                working_dir,
            })
        }

        Runtime::Proton {
            prefix,
            proton,
            umu_id,
            launcher,
        } => {
            if prefix.as_os_str().is_empty() {
                return Err(CoreError::Other("Proton runtime needs a prefix".into()));
            }

            // umu-launcher reads its configuration from the environment rather than
            // flags, so these are part of the contract, not decoration.
            env.insert(
                "WINEPREFIX".to_string(),
                prefix.to_string_lossy().into_owned(),
            );
            env.insert("PROTONPATH".to_string(), proton.clone());
            env.insert("GAMEID".to_string(), umu_id.clone());
            // Without a store hint umu warns and falls back; "none" is the value for a
            // game that did not come from a known storefront.
            env.entry("STORE".to_string())
                .or_insert_with(|| "none".into());

            let mut args = vec![config.executable.clone().into_os_string()];
            args.extend(config.arguments.iter().map(OsString::from));

            Ok(ResolvedCommand {
                program: launcher.clone().into_os_string(),
                args,
                env,
                working_dir,
            })
        }
    }
}

/// Whether a game needs Proton on this host.
///
/// A `.exe` on Linux does; anything else is run directly. On Windows nothing needs it.
pub fn needs_proton(executable: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }
    executable
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

/// Per-game Wine prefix directory under a shared prefixes root.
///
/// Keyed by game id so a retitled game keeps its prefix, and its saves with it.
pub fn prefix_for(prefixes_root: &Path, game_id: i64) -> PathBuf {
    prefixes_root.join(game_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the command a Flatpak install actually runs: Wine on the host, reached
    /// through the sandbox helper.
    fn host_wine_command() -> ResolvedCommand {
        let runtime = crate::runtime::WindowsRuntime::HostWine;
        let mut config =
            LaunchConfig::for_windows_program("S:\\setup.exe", "/prefixes/12", &runtime);
        config.working_dir = Some(PathBuf::from("/downloads/12"));
        resolve_command(&config).unwrap()
    }

    #[test]
    fn a_command_we_spawn_ourselves_takes_the_limit_before_exec() {
        let mut cmd = resolve_command(&LaunchConfig::wine(
            "/g/s.exe",
            "/prefixes/12",
            "/usr/bin/wine",
        ))
        .unwrap();
        let before = cmd.args.clone();

        let cap = cmd.cap_address_space(3 * 1024 * 1024 * 1024);

        assert_eq!(cap, AddressSpaceCap::BeforeExec(3 * 1024 * 1024 * 1024));
        assert_eq!(cap.before_exec(), Some(3 * 1024 * 1024 * 1024));
        // Nothing is added to the command line: the rlimit does the work.
        assert_eq!(cmd.args, before, "a direct spawn must not be wrapped");
        assert!(!cmd.runs_on_host());
    }

    #[test]
    fn a_command_run_on_the_host_carries_the_limit_in_its_arguments() {
        let mut cmd = host_wine_command();
        assert!(cmd.runs_on_host());

        let cap = cmd.cap_address_space(3 * 1024 * 1024 * 1024);

        // An rlimit here would land on the `flatpak-spawn` stub, never on Wine.
        assert_eq!(cap, AddressSpaceCap::InCommandLine);
        assert_eq!(cap.before_exec(), None);

        let args: Vec<String> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        // `--host` and the environment still lead: the wrapper goes after the helper's
        // own options, not before them.
        assert_eq!(args[0], "--host");
        let shell = args
            .iter()
            .position(|a| a == "sh")
            .expect("a shell wrapper");
        assert!(
            args[..shell].iter().all(|a| a.starts_with("--")),
            "the wrapper must not displace flatpak-spawn's options: {args:?}"
        );
        assert_eq!(args[shell + 1], "-c");
        // 3 GiB expressed in the kibibytes `ulimit -v` expects.
        assert_eq!(args[shell + 2], "ulimit -v 3145728; exec \"$0\" \"$@\"");
        // `$0` is the program, `$@` its arguments.
        assert_eq!(args[shell + 3], "wine");
        assert!(
            args.iter().any(|a| a.contains("setup.exe")),
            "the installer must survive the wrapping: {args:?}"
        );
    }

    #[test]
    fn the_prefix_still_reaches_a_wrapped_host_command() {
        let mut cmd = host_wine_command();
        cmd.cap_address_space(3 * 1024 * 1024 * 1024);

        // A WINEPREFIX that fails to cross the boundary sends Wine to its default prefix,
        // where it succeeds while leaving ours empty, so the cap must not disturb it.
        assert!(
            cmd.args
                .iter()
                .any(|a| a.to_string_lossy() == "--env=WINEPREFIX=/prefixes/12"),
            "the prefix must still be passed to the host: {:?}",
            cmd.args
        );
        assert!(
            cmd.args
                .iter()
                .any(|a| a.to_string_lossy() == "--directory=/downloads/12"),
            "the working directory must still be passed to the host: {:?}",
            cmd.args
        );
    }

    #[test]
    fn native_runs_the_executable_directly() {
        let config = LaunchConfig::native("/games/celeste/Celeste");
        let cmd = resolve_command(&config).unwrap();
        assert_eq!(cmd.program, OsString::from("/games/celeste/Celeste"));
        assert!(cmd.args.is_empty());
        assert_eq!(cmd.working_dir, Some(PathBuf::from("/games/celeste")));
    }

    #[test]
    fn arguments_are_passed_through_as_a_vector() {
        // Never a shell string: a path with a space or a quote must not need escaping.
        let mut config = LaunchConfig::native("/games/My Game/run.sh");
        config.arguments = vec!["--windowed".into(), "--name=A \"quoted\" thing".into()];
        let cmd = resolve_command(&config).unwrap();
        assert_eq!(
            cmd.args,
            vec![
                OsString::from("--windowed"),
                OsString::from("--name=A \"quoted\" thing")
            ]
        );
    }

    #[test]
    fn proton_launches_through_umu_run() {
        let config = LaunchConfig::proton(
            "/games/celeste/Celeste.exe",
            "/prefixes/12",
            "/usr/bin/umu-run",
        );
        let cmd = resolve_command(&config).unwrap();

        assert_eq!(cmd.program, OsString::from("/usr/bin/umu-run"));
        assert_eq!(cmd.args, vec![OsString::from("/games/celeste/Celeste.exe")]);
        assert_eq!(cmd.env["WINEPREFIX"], "/prefixes/12");
        assert_eq!(cmd.env["PROTONPATH"], DEFAULT_PROTON);
        assert_eq!(cmd.env["GAMEID"], DEFAULT_UMU_ID);
        assert_eq!(cmd.env["STORE"], "none");
    }

    #[test]
    fn user_environment_is_preserved_alongside_proton_settings() {
        let mut config = LaunchConfig::proton("/g/Game.exe", "/prefixes/1", "/usr/bin/umu-run");
        config
            .environment
            .insert("DXVK_HUD".into(), "fps".to_string());
        let cmd = resolve_command(&config).unwrap();
        assert_eq!(cmd.env["DXVK_HUD"], "fps");
        assert_eq!(cmd.env["WINEPREFIX"], "/prefixes/1");
    }

    #[test]
    fn an_explicit_store_is_not_overwritten() {
        let mut config = LaunchConfig::proton("/g/Game.exe", "/prefixes/1", "/usr/bin/umu-run");
        config.environment.insert("STORE".into(), "gog".to_string());
        let cmd = resolve_command(&config).unwrap();
        assert_eq!(cmd.env["STORE"], "gog");
    }

    #[test]
    fn a_specific_proton_build_and_umu_id_are_used() {
        let config = LaunchConfig {
            executable: "/g/Game.exe".into(),
            working_dir: None,
            arguments: vec![],
            environment: BTreeMap::new(),
            runtime: Runtime::Proton {
                prefix: "/prefixes/7".into(),
                proton: "GE-Proton9-20".into(),
                umu_id: "umu-1234".into(),
                launcher: "/usr/bin/umu-run".into(),
            },
        };
        let cmd = resolve_command(&config).unwrap();
        assert_eq!(cmd.env["PROTONPATH"], "GE-Proton9-20");
        assert_eq!(cmd.env["GAMEID"], "umu-1234");
    }

    #[test]
    fn host_wine_passes_its_environment_as_explicit_arguments() {
        let config = LaunchConfig::for_windows_program(
            "/g/Game.exe",
            "/prefixes/9",
            &crate::runtime::WindowsRuntime::HostWine,
        );
        let cmd = resolve_command(&config).unwrap();

        let args: Vec<String> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        // `--host` must lead, the environment must be explicit, and `wine` must come
        // after both.
        assert_eq!(args.first().unwrap(), "--host");
        assert!(args.iter().any(|a| a == "--env=WINEPREFIX=/prefixes/9"));
        let wine_at = args
            .iter()
            .position(|a| a == "wine")
            .expect("wine argument");
        let env_at = args
            .iter()
            .position(|a| a.starts_with("--env="))
            .expect("an env argument");
        assert!(env_at < wine_at, "env must precede the program: {args:?}");
        assert_eq!(args.last().unwrap(), "/g/Game.exe");
    }

    #[test]
    fn wine_runs_the_executable_directly_with_a_prefix() {
        let config = LaunchConfig::wine("/g/Game.exe", "/prefixes/3", "/usr/bin/wine");
        let cmd = resolve_command(&config).unwrap();

        assert_eq!(cmd.program, OsString::from("/usr/bin/wine"));
        assert_eq!(cmd.args, vec![OsString::from("/g/Game.exe")]);
        assert_eq!(cmd.env["WINEPREFIX"], "/prefixes/3");
        // Proton-only settings must not leak into a plain Wine launch.
        assert!(!cmd.env.contains_key("PROTONPATH"));
        assert!(!cmd.env.contains_key("GAMEID"));
    }

    #[test]
    fn steam_proton_is_invoked_with_run_and_its_own_variables() {
        let config = LaunchConfig::for_windows_program(
            "/g/Game.exe",
            "/prefixes/5",
            &crate::runtime::WindowsRuntime::SteamProton {
                script: PathBuf::from("/steam/common/Proton 9.0/proton"),
                steam_root: PathBuf::from("/steam"),
                name: "Proton 9.0".into(),
            },
        );
        let cmd = resolve_command(&config).unwrap();

        assert_eq!(
            cmd.program,
            OsString::from("/steam/common/Proton 9.0/proton")
        );
        assert_eq!(
            cmd.args,
            vec![OsString::from("run"), OsString::from("/g/Game.exe")]
        );
        assert_eq!(cmd.env["STEAM_COMPAT_DATA_PATH"], "/prefixes/5");
        assert_eq!(cmd.env["STEAM_COMPAT_CLIENT_INSTALL_PATH"], "/steam");
        // Proton reads the compat path, not WINEPREFIX.
        assert!(!cmd.env.contains_key("WINEPREFIX"));
    }

    #[test]
    fn an_explicit_working_directory_wins() {
        let mut config = LaunchConfig::native("/games/celeste/bin/Celeste");
        config.working_dir = Some(PathBuf::from("/games/celeste"));
        assert_eq!(
            resolve_command(&config).unwrap().working_dir,
            Some(PathBuf::from("/games/celeste"))
        );
    }

    #[test]
    fn an_empty_executable_is_rejected() {
        let config = LaunchConfig::native("");
        assert!(resolve_command(&config).is_err());
    }

    #[test]
    fn proton_without_a_prefix_is_rejected() {
        let mut config = LaunchConfig::proton("/g/Game.exe", "", "/usr/bin/umu-run");
        config.runtime = Runtime::Proton {
            prefix: PathBuf::new(),
            proton: DEFAULT_PROTON.into(),
            umu_id: DEFAULT_UMU_ID.into(),
            launcher: "/usr/bin/umu-run".into(),
        };
        assert!(resolve_command(&config).is_err());
    }

    #[test]
    fn prefixes_are_keyed_by_game_id() {
        // Keyed by id, not title, so renaming a game does not orphan its prefix.
        assert_eq!(
            prefix_for(Path::new("/data/prefixes"), 42),
            PathBuf::from("/data/prefixes/42")
        );
    }

    #[test]
    #[cfg(unix)]
    fn exe_files_need_proton_on_linux() {
        assert!(needs_proton(Path::new("/games/Celeste.exe")));
        assert!(needs_proton(Path::new("/games/Celeste.EXE")));
        assert!(!needs_proton(Path::new("/games/Celeste.x86_64")));
        assert!(!needs_proton(Path::new("/games/Celeste")));
    }

    #[test]
    #[cfg(windows)]
    fn nothing_needs_proton_on_windows() {
        assert!(!needs_proton(Path::new("C:/games/Celeste.exe")));
    }
}
