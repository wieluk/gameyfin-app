//! Launching games. Windows and native Linux builds run directly; Windows games on Linux
//! go through umu and Proton, with plain Wine as the fallback. Each game gets its own prefix.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Identifier `umu-run` falls back to when a game has no known per-title fixes.
pub const DEFAULT_UMU_ID: &str = "umu-default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Runtime {
    /// Run the binary directly: a Windows game on Windows, or a native Linux build.
    Native,
    /// Run a Windows game on Linux through umu-launcher and Proton.
    Proton {
        prefix: PathBuf,
        /// The Proton build directory, handed to umu as `PROTONPATH`.
        proton: PathBuf,
        /// umu game id, used to apply per-title fixes.
        umu_id: String,
        /// Resolved rather than taken from `PATH`: a desktop entry lacks the shell's `PATH`, and a
        /// Flatpak puts it elsewhere.
        launcher: PathBuf,
    },
    /// Plain Wine: the fallback for a game that misbehaves in the Steam Runtime container,
    /// or a system where that container cannot start.
    Wine {
        prefix: PathBuf,
        wine: PathBuf,
        /// Arguments that must precede the program, for a Wine reached indirectly,
        /// `flatpak-spawn --host wine` from inside a sandbox.
        prefix_args: Vec<String>,
    },
}

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
        proton: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            working_dir: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            runtime: Runtime::Proton {
                prefix: prefix.into(),
                proton: proton.into(),
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
            crate::runtime::WindowsRuntime::Umu {
                launcher, proton, ..
            } => Self::proton(executable, prefix, launcher.clone(), proton.clone()),
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

    /// Same, with the mscoree and mshtml prompts suppressed: they open behind the window and
    /// block the prefix update until answered, which looks exactly like a hang.
    pub fn for_windows_program_unattended(
        executable: impl Into<PathBuf>,
        prefix: impl Into<PathBuf>,
        runtime: &crate::runtime::WindowsRuntime,
    ) -> Self {
        Self::for_windows_program_with(
            executable,
            prefix,
            runtime,
            &crate::prefix::DllOverrides::default(),
        )
    }

    /// The same, with the prefix's own overrides layered on: the graphics DLLs it holds,
    /// and whatever the game's options asked for.
    pub fn for_windows_program_with(
        executable: impl Into<PathBuf>,
        prefix: impl Into<PathBuf>,
        runtime: &crate::runtime::WindowsRuntime,
        overrides: &crate::prefix::DllOverrides,
    ) -> Self {
        let mut config = Self::for_windows_program(executable, prefix, runtime);
        // Proton ships wine-mono and gecko and installs them itself, so disabling them
        // there breaks .NET games instead of silencing a prompt.
        let all = if runtime.is_wine_family() {
            crate::prefix::DllOverrides::no_prompts().merged_with(overrides.clone())
        } else {
            overrides.clone()
        };
        if !all.is_empty() {
            config
                .environment
                .insert("WINEDLLOVERRIDES".to_string(), all.to_env());
        }
        crate::prefix::without_input_method(&mut config.environment);
        config
    }

    pub fn resolved_working_dir(&self) -> Option<PathBuf> {
        self.working_dir
            .clone()
            .or_else(|| self.executable.parent().map(Path::to_path_buf))
    }
}

/// A resolved command line, not yet run. Separating resolution from spawning is what lets a
/// test assert the exact program, arguments and environment without starting a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub env: BTreeMap<String, String>,
    pub working_dir: Option<PathBuf>,
}

/// How an address-space cap reached its process. Two mechanisms, because the wrong one
/// still caps something, just not the process that needed it, and nothing reports that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceCap {
    /// Set with `setrlimit` between `fork` and `exec`, and inherited by the whole tree.
    BeforeExec(u64),
    /// Carried in the command line, for a program spawned on the far side of a sandbox
    /// boundary that our own limits do not cross.
    InCommandLine,
}

impl AddressSpaceCap {
    pub fn before_exec(self) -> Option<u64> {
        match self {
            AddressSpaceCap::BeforeExec(bytes) => Some(bytes),
            AddressSpaceCap::InCommandLine => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AddressSpaceCap::BeforeExec(_) => "setrlimit",
            AddressSpaceCap::InCommandLine => "ulimit on the host",
        }
    }
}

impl ResolvedCommand {
    /// Whether the program starts outside our process tree: `flatpak-spawn` only sends a
    /// D-Bus request, and nothing set on our own child crosses that gap.
    pub fn runs_on_host(&self) -> bool {
        Path::new(&self.program) == Path::new(crate::runtime::FLATPAK_SPAWN)
    }

    /// Ends everything in this command's prefix with `wineserver -k`. Signals miss the tree
    /// because `wineserver` daemonizes out of the process group.
    pub fn wine_teardown(&self) -> Option<ResolvedCommand> {
        let prefix = self.env.get("WINEPREFIX")?;
        let program = Path::new(&self.program);

        let wineserver = match self.env.get("PROTONPATH") {
            Some(proton) => Path::new(proton)
                .join("files")
                .join("bin")
                .join("wineserver"),
            None => {
                if program.file_name()? != "wine" {
                    return None;
                }
                program.parent()?.join("wineserver")
            }
        };
        if !wineserver.is_file() {
            return None;
        }

        let mut env = std::collections::BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), prefix.clone());
        Some(ResolvedCommand {
            program: wineserver.into_os_string(),
            args: vec![OsString::from("-k")],
            env,
            working_dir: None,
        })
    }

    /// Caps the program's address space. Behind `flatpak-spawn` an rlimit would land on the
    /// D-Bus stub, so there the cap travels inside the command.
    pub fn cap_address_space(&mut self, bytes: u64) -> AddressSpaceCap {
        if !self.runs_on_host() {
            return AddressSpaceCap::BeforeExec(bytes);
        }

        // Leading options belong to `flatpak-spawn`; the host command after them is what gets wrapped.
        let split = self
            .args
            .iter()
            .position(|a| !a.to_string_lossy().starts_with("--"))
            .unwrap_or(self.args.len());

        // `exec` puts the limit on Wine itself. A shell that refuses the limit still runs the
        // installer, just uncapped.
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

/// Builds the command for a launch configuration. Never through a shell: an argument vector
/// removes the whole class of quoting bugs a path with a space or a quote would bring.
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
            env.insert(
                "PROTONPATH".to_string(),
                proton.to_string_lossy().into_owned(),
            );
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

/// Whether a game needs Proton on this host: only a `.exe` on Linux does.
pub fn needs_proton(executable: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }
    executable
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wine build laid out as the app's own is: `<root>/bin/wine` beside `wineserver`.
    fn fake_wine_build(name: &str) -> std::path::PathBuf {
        let bin = std::env::temp_dir()
            .join(format!("gameyfin-wine-{}-{name}", std::process::id()))
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for program in ["wine", "wineserver"] {
            std::fs::write(bin.join(program), b"#!/bin/sh\n").unwrap();
        }
        bin
    }

    #[test]
    fn the_teardown_runs_wineserver_against_this_games_prefix() {
        let bin = fake_wine_build("ok");
        let mut env = std::collections::BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), "/games/Prefixes/93".to_string());
        let command = ResolvedCommand {
            program: bin.join("wine").into_os_string(),
            args: vec![OsString::from("setup.exe")],
            env,
            working_dir: None,
        };

        let teardown = command
            .wine_teardown()
            .expect("a wine build has a wineserver");
        assert_eq!(teardown.program, bin.join("wineserver").into_os_string());
        assert_eq!(teardown.args, vec![OsString::from("-k")]);
        // Scoped: killing every prefix would take out another game's install too.
        assert_eq!(teardown.env["WINEPREFIX"], "/games/Prefixes/93");
    }

    #[test]
    fn nothing_is_torn_down_when_there_is_no_prefix_to_scope_it_to() {
        let bin = fake_wine_build("noprefix");
        let command = ResolvedCommand {
            program: bin.join("wine").into_os_string(),
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
            working_dir: None,
        };
        assert!(
            command.wine_teardown().is_none(),
            "without WINEPREFIX this would kill the user's default prefix"
        );
    }

    #[test]
    fn only_a_wine_with_a_wineserver_beside_it_is_torn_down() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), "/pfx".to_string());

        // Proton, umu and flatpak-spawn all reach Wine another way.
        for program in [
            "/usr/bin/umu-run",
            "/steam/proton",
            crate::runtime::FLATPAK_SPAWN,
        ] {
            let command = ResolvedCommand {
                program: OsString::from(program),
                args: Vec::new(),
                env: env.clone(),
                working_dir: None,
            };
            assert!(command.wine_teardown().is_none(), "{program}");
        }

        // A wine with no wineserver next to it is not one we can end this way.
        let lonely = std::env::temp_dir().join(format!("gameyfin-lonely-{}", std::process::id()));
        std::fs::create_dir_all(&lonely).unwrap();
        std::fs::write(lonely.join("wine"), b"#!/bin/sh\n").unwrap();
        let command = ResolvedCommand {
            program: lonely.join("wine").into_os_string(),
            args: Vec::new(),
            env,
            working_dir: None,
        };
        assert!(command.wine_teardown().is_none(), "no wineserver beside it");
    }

    #[test]
    fn a_umu_game_is_torn_down_with_its_proton_builds_own_wineserver() {
        let build =
            std::env::temp_dir().join(format!("gameyfin-proton-teardown-{}", std::process::id()));
        let bin = build.join("files").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("wineserver"), b"#!/bin/sh\n").unwrap();

        let mut env = std::collections::BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), "/pfx".to_string());
        env.insert(
            "PROTONPATH".to_string(),
            build.to_string_lossy().into_owned(),
        );
        let command = ResolvedCommand {
            program: OsString::from("/app/bin/umu-run"),
            args: Vec::new(),
            env,
            working_dir: None,
        };

        let teardown = command.wine_teardown().expect("a teardown for umu");
        assert_eq!(PathBuf::from(&teardown.program), bin.join("wineserver"));
        assert_eq!(teardown.args, vec![OsString::from("-k")]);
        assert_eq!(teardown.env["WINEPREFIX"], "/pfx");
        std::fs::remove_dir_all(&build).unwrap();
    }

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
            "/cfg/proton/UMU-Proton-10.0-4",
        );
        let cmd = resolve_command(&config).unwrap();

        assert_eq!(cmd.program, OsString::from("/usr/bin/umu-run"));
        assert_eq!(cmd.args, vec![OsString::from("/games/celeste/Celeste.exe")]);
        assert_eq!(cmd.env["WINEPREFIX"], "/prefixes/12");
        // An absolute build path, never a codename umu would go and download on its own.
        assert_eq!(cmd.env["PROTONPATH"], "/cfg/proton/UMU-Proton-10.0-4");
        assert_eq!(cmd.env["GAMEID"], DEFAULT_UMU_ID);
        assert_eq!(cmd.env["STORE"], "none");
    }

    #[test]
    fn user_environment_is_preserved_alongside_proton_settings() {
        let mut config =
            LaunchConfig::proton("/g/Game.exe", "/prefixes/1", "/usr/bin/umu-run", "/p/UMU");
        config
            .environment
            .insert("DXVK_HUD".into(), "fps".to_string());
        let cmd = resolve_command(&config).unwrap();
        assert_eq!(cmd.env["DXVK_HUD"], "fps");
        assert_eq!(cmd.env["WINEPREFIX"], "/prefixes/1");
    }

    #[test]
    fn an_explicit_store_is_not_overwritten() {
        let mut config =
            LaunchConfig::proton("/g/Game.exe", "/prefixes/1", "/usr/bin/umu-run", "/p/UMU");
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
                proton: "/cfg/proton/GE-Proton11-6-x86_64".into(),
                umu_id: "umu-1234".into(),
                launcher: "/usr/bin/umu-run".into(),
            },
        };
        let cmd = resolve_command(&config).unwrap();
        assert_eq!(cmd.env["PROTONPATH"], "/cfg/proton/GE-Proton11-6-x86_64");
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
    fn a_umu_launch_never_disables_protons_own_mono_and_gecko() {
        // Proton installs wine-mono itself; `mscoree=` there breaks every .NET game.
        let runtime = crate::runtime::WindowsRuntime::Umu {
            launcher: PathBuf::from("/app/bin/umu-run"),
            proton: PathBuf::from("/cfg/proton/UMU-Proton-10.0-4"),
            build: "UMU-Proton-10.0-4".into(),
        };
        let config =
            LaunchConfig::for_windows_program_unattended("/g/Game.exe", "/prefixes/5", &runtime);
        let cmd = resolve_command(&config).unwrap();

        assert_eq!(cmd.program, OsString::from("/app/bin/umu-run"));
        assert!(!cmd.env.contains_key("WINEDLLOVERRIDES"), "{:?}", cmd.env);
        assert_eq!(cmd.env["PROTONPATH"], "/cfg/proton/UMU-Proton-10.0-4");
        assert_eq!(cmd.env["XMODIFIERS"], "@im=none");
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
        let mut config = LaunchConfig::proton("/g/Game.exe", "", "/usr/bin/umu-run", "/p/UMU");
        config.runtime = Runtime::Proton {
            prefix: PathBuf::new(),
            proton: PathBuf::from("/p/UMU"),
            umu_id: DEFAULT_UMU_ID.into(),
            launcher: "/usr/bin/umu-run".into(),
        };
        assert!(resolve_command(&config).is_err());
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
    fn an_unattended_launch_keeps_the_input_method_out_of_the_way() {
        // With ibus active the accent picker swallows key presses, so WASD needs a long press.
        let runtime = crate::runtime::WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let config =
            LaunchConfig::for_windows_program_unattended("/g/Game.exe", "/prefixes/7", &runtime);
        assert_eq!(config.environment["XMODIFIERS"], "@im=none");
        // The prompt suppression must survive alongside it.
        assert_eq!(
            config.environment["WINEDLLOVERRIDES"],
            crate::prefix::DllOverrides::no_prompts().to_env()
        );
    }

    #[test]
    #[cfg(windows)]
    fn nothing_needs_proton_on_windows() {
        assert!(!needs_proton(Path::new("C:/games/Celeste.exe")));
    }
}
