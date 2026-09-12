//! Preparing a Wine or Proton prefix: the games folder gets a drive letter so an installer
//! path lands where the app expects, and the DPI follows the real screen.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CoreResult;
use crate::runtime::WindowsRuntime;

/// Wine DLL overrides. A type rather than a string because three sources combine: the
/// prompts always suppressed, the prefix's graphics DLLs, and a game's own options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DllOverrides(std::collections::BTreeMap<String, String>);

impl DllOverrides {
    /// Stops a fresh prefix popping up "install Mono?"/"install Gecko?" dialogs that appear
    /// off-screen and block the prefix update until answered. Games rarely need either.
    pub fn no_prompts() -> Self {
        Self(
            ["mscoree", "mshtml"]
                .iter()
                .map(|dll| ((*dll).to_string(), String::new()))
                .collect(),
        )
    }

    /// Prefer the copies installed in the prefix over Wine's own, for DXVK and vkd3d-proton.
    pub fn native(names: &[&str]) -> Self {
        Self(
            names
                .iter()
                .map(|dll| ((*dll).to_string(), "native,builtin".to_string()))
                .collect(),
        )
    }

    /// Read a `WINEDLLOVERRIDES` value, as a user would paste one from a wiki.
    pub fn parse(value: &str) -> Self {
        let mut out = std::collections::BTreeMap::new();
        for entry in value.split(';') {
            let Some((names, mode)) = entry.split_once('=') else {
                continue;
            };
            for name in names.split(',') {
                let name = name.trim();
                if !name.is_empty() {
                    out.insert(name.to_string(), mode.trim().to_string());
                }
            }
        }
        Self(out)
    }

    /// Combine two sets, with `other` winning per DLL so a game's own choice beats ours.
    pub fn merged_with(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        self
    }

    /// The `WINEDLLOVERRIDES` value. `;` separates entries; a comma inside one joins names
    /// that share a mode, so `mscoree=,mshtml=` would set nothing for mshtml.
    pub fn to_env(&self) -> String {
        self.0
            .iter()
            .map(|(dll, mode)| format!("{dll}={mode}"))
            .collect::<Vec<_>>()
            .join(";")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(dll, mode)| (dll.as_str(), mode.as_str()))
    }
}

/// Keep the desktop's input method out of Wine: with ibus or fcitx, XIM holds key presses back
/// from games reading key state. The cost is composing CJK text inside a game.
pub fn without_input_method(env: &mut std::collections::BTreeMap<String, String>) {
    env.insert("XMODIFIERS".to_string(), "@im=none".to_string());
}

/// Marker recording that a prefix has been prepared, so it happens once.
const READY_MARKER: &str = ".gameyfin-ready";

/// Where `drive_c` and `dosdevices` live: the prefix itself for Wine, a `pfx` subdirectory
/// for Proton.
pub fn wine_root(prefix: &Path) -> PathBuf {
    let proton = prefix.join("pfx");
    if proton.is_dir() {
        proton
    } else {
        prefix.to_path_buf()
    }
}

/// What the marker holds, and therefore what a change to re-prepares a prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PrefixState {
    pub dpi: u32,
    /// The Wine build the prefix was made with. A different one reinstalls the builtin
    /// Direct3D DLLs on its first `wineboot`, silently reverting the graphics copies.
    pub runtime: String,
    pub dxvk: Option<String>,
    pub vkd3d: Option<String>,
}

/// The Windows user home inside a prefix, so a save can cross between Windows and Proton.
/// Proton uses `steamuser`, Wine the host username; `None` when ambiguous.
pub fn prefix_home(prefix: &Path) -> Option<PathBuf> {
    let users = wine_root(prefix).join("drive_c").join("users");

    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&users)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        // `Public` is Windows' shared profile, never the user's own.
        .filter(|path| !matches!(file_name_of(path).as_deref(), Some("Public")))
        .collect();
    candidates.sort();

    if candidates.len() == 1 {
        return candidates.pop();
    }
    // Several accounts in one prefix: Proton's is the one a game actually runs as.
    candidates
        .into_iter()
        .find(|path| file_name_of(path).as_deref() == Some("steamuser"))
}

fn file_name_of(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// Whether a prefix is prepared exactly as `wanted` describes. An older marker does not
/// parse and so reads as unprepared, which costs one slow launch and nothing else.
pub fn is_prepared(prefix: &Path, wanted: &PrefixState) -> bool {
    // `drive_c` is checked as well as the marker: a prefix marked ready but missing it is
    // unusable, and reusing it would repeat the failure on every launch.
    if !wine_root(prefix).join("drive_c").is_dir() {
        return false;
    }
    std::fs::read_to_string(prefix.join(READY_MARKER))
        .ok()
        .and_then(|text| serde_json::from_str::<PrefixState>(&text).ok())
        .is_some_and(|found| &found == wanted)
}

pub fn mark_prepared(prefix: &Path, state: &PrefixState) -> CoreResult<()> {
    let text = serde_json::to_string_pretty(state).map_err(|e| {
        crate::error::CoreError::Other(format!("could not record the prefix state: {e}"))
    })?;
    std::fs::write(prefix.join(READY_MARKER), text)?;
    Ok(())
}

pub fn boot_command(runtime: &WindowsRuntime, prefix: &Path) -> crate::launch::ResolvedCommand {
    registry_command(runtime, prefix, &["wineboot", "-u"])
}

/// The command that opens a Wine tool against a prefix. Needs [`boot_command`]'s
/// environment, or `winecfg` configures the default prefix instead of the game's.
pub fn tool_command(
    runtime: &WindowsRuntime,
    prefix: &Path,
    tool: &str,
) -> crate::launch::ResolvedCommand {
    registry_command(runtime, prefix, &[tool])
}

/// The command that sets a prefix's DPI, applied through the runtime since Wine rewrites
/// `user.reg` on start and an appended fragment can be lost.
pub fn dpi_command(
    runtime: &WindowsRuntime,
    prefix: &Path,
    dpi: u32,
) -> crate::launch::ResolvedCommand {
    registry_command(
        runtime,
        prefix,
        &[
            "reg",
            "add",
            "HKCU\\Control Panel\\Desktop",
            "/v",
            "LogPixels",
            "/t",
            "REG_DWORD",
            "/d",
            &dpi.to_string(),
            "/f",
        ],
    )
}

/// Records DLL overrides in the prefix registry, which covers what a game's own launcher
/// starts as well as the launch this app controls.
pub fn override_commands(
    runtime: &WindowsRuntime,
    prefix: &Path,
    overrides: &DllOverrides,
) -> Vec<crate::launch::ResolvedCommand> {
    const KEY: &str = "HKCU\\Software\\Wine\\DllOverrides";

    overrides
        .iter()
        .map(|(dll, mode)| {
            registry_command(
                runtime,
                prefix,
                &["reg", "add", KEY, "/v", dll, "/d", mode, "/f"],
            )
        })
        .collect()
}

/// Where wine.inf installs the theme Wine bundles, relative to the prefix root.
const AERO_THEME: &str = "drive_c/windows/resources/themes/aero/aero.msstyles";

const AERO_THEME_WINDOWS: &str = "C:\\windows\\resources\\themes\\aero\\aero.msstyles";

/// Whether this prefix has the bundled theme, since a stripped Wine build can omit
/// `aero.msstyles` and pointing the registry at a missing file does nothing.
pub fn has_bundled_theme(prefix: &Path) -> bool {
    wine_root(prefix).join(AERO_THEME).is_file()
}

/// Turns on Wine's bundled theme where `wine.inf` did not. `ColorName` is "Blue", the name
/// inside Wine's theme, not the usual "NormalColor".
pub fn theme_commands(
    runtime: &WindowsRuntime,
    prefix: &Path,
) -> Vec<crate::launch::ResolvedCommand> {
    const KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\ThemeManager";

    [
        ("ThemeActive", "1"),
        ("DllName", AERO_THEME_WINDOWS),
        ("ColorName", "Blue"),
        ("SizeName", "NormalSize"),
    ]
    .iter()
    .map(|(name, value)| {
        registry_command(
            runtime,
            prefix,
            &["reg", "add", KEY, "/v", name, "/d", value, "/f"],
        )
    })
    .collect()
}

fn registry_command(
    runtime: &WindowsRuntime,
    prefix: &Path,
    args: &[&str],
) -> crate::launch::ResolvedCommand {
    registry_command_with(runtime, prefix, &DllOverrides::default(), args)
}

fn registry_command_with(
    runtime: &WindowsRuntime,
    prefix: &Path,
    overrides: &DllOverrides,
    args: &[&str],
) -> crate::launch::ResolvedCommand {
    use std::collections::BTreeMap;

    let mut env = BTreeMap::new();
    // Proton ships wine-mono and gecko and installs them itself, so the prompt suppression
    // is Wine's alone: applied to Proton it breaks .NET games instead.
    let overrides = if runtime.is_wine_family() {
        DllOverrides::no_prompts().merged_with(overrides.clone())
    } else {
        overrides.clone()
    };
    if !overrides.is_empty() {
        env.insert("WINEDLLOVERRIDES".to_string(), overrides.to_env());
    }
    without_input_method(&mut env);
    env.insert(
        "WINEPREFIX".to_string(),
        prefix.to_string_lossy().into_owned(),
    );

    if let WindowsRuntime::Umu { proton, .. } = runtime {
        env.insert(
            "PROTONPATH".to_string(),
            proton.to_string_lossy().into_owned(),
        );
        env.insert(
            "GAMEID".to_string(),
            crate::launch::DEFAULT_UMU_ID.to_string(),
        );
        env.insert("STORE".to_string(), "none".to_string());
    }

    // The same wrapper the launcher uses, so environment handling cannot diverge between
    // preparing a prefix and running a game in it.
    let full_args = runtime.wrap_args(&env, None, args.iter().map(|a| a.to_string()));

    crate::launch::ResolvedCommand {
        program: runtime.program().to_path_buf().into_os_string(),
        args: full_args,
        env,
        working_dir: None,
    }
}

/// Where the setup registry file is written: inside the prefix, which the Steam Runtime
/// container always sees.
pub const REGISTRY_FILE: &str = ".gameyfin-setup.reg";

/// A `.reg` file carrying the prefix setup, imported in one run. Through umu every command
/// starts the Steam Runtime container, so one import beats a command per value.
pub fn registry_file(dpi: u32) -> String {
    format!(
        "REGEDIT4\r\n\r\n[HKEY_CURRENT_USER\\Control Panel\\Desktop]\r\n\"LogPixels\"=dword:{dpi:08x}\r\n"
    )
}

pub fn import_command(
    runtime: &WindowsRuntime,
    prefix: &Path,
    file: &Path,
) -> crate::launch::ResolvedCommand {
    registry_command(runtime, prefix, &["regedit", "/S", &z_drive_path(file)])
}

/// A host path as Wine sees it through `Z:`, which maps the root of the filesystem. A bare
/// Unix path handed to a Windows program reads as relative to the current drive.
pub fn z_drive_path(path: &Path) -> String {
    format!("Z:{}", path.to_string_lossy().replace('/', "\\"))
}

/// The games folder's drive letter inside a prefix: `G`, far enough down the alphabet to
/// miss the letters Windows software assumes are taken.
pub const GAMES_DRIVE: char = 'G';

/// The installer source folder's drive letter, so it sees a clean path rather than our
/// `(id) Title` naming. Not `S`: Proton owns that one and clears it on every run.
pub const SOURCE_DRIVE: char = 'R';

/// Make a directory name safe for a Windows installer and the shells it invokes.
pub fn windows_safe_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if is_illegal_filename_char(c) || is_shell_metachar(c) {
                ' '
            } else {
                c
            }
        })
        .collect();

    // Collapse the runs of spaces the replacements leave behind.
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "Game".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Characters Windows refuses outright in a filename.
pub fn is_illegal_filename_char(c: char) -> bool {
    matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 0x20
}

/// Legal on Windows, but metacharacters to `cmd`, which installers invoke for their own
/// helper scripts. A path containing them can fail deep inside an install with no error.
pub fn is_shell_metachar(c: char) -> bool {
    matches!(
        c,
        '(' | ')' | '[' | ']' | '&' | '^' | ';' | ',' | '!' | '%' | '=' | '\'' | '`'
    )
}

/// The Windows path a user should type to install a game. The drive is mapped to the
/// installations root with a per-game subfolder, since Inno Setup rejects a bare `G:\`.
pub fn games_drive_path(folder: &str) -> String {
    format!("{GAMES_DRIVE}:\\{folder}")
}

pub fn map_drive(prefix: &Path, target: &Path) -> CoreResult<PathBuf> {
    map_drive_letter(prefix, GAMES_DRIVE, target, true)
}

/// Point any drive letter at a directory via a `dosdevices` symlink. `create` makes a
/// missing target (right for a destination, wrong for a source that should exist).
#[cfg(unix)]
pub fn map_drive_letter(
    prefix: &Path,
    letter: char,
    target: &Path,
    create: bool,
) -> CoreResult<PathBuf> {
    // Wine reads this from inside its own prefix, which for Proton is `pfx`, not the
    // compat data directory we were given.
    let dosdevices = wine_root(prefix).join("dosdevices");
    std::fs::create_dir_all(&dosdevices)?;
    if create {
        std::fs::create_dir_all(target)?;
    }

    let link = dosdevices.join(format!("{}:", letter.to_ascii_lowercase()));

    // Replace any previous mapping: the games folder can move between installs.
    match std::fs::remove_file(&link) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    std::os::unix::fs::symlink(target, &link)?;

    Ok(link)
}

/// Refused on Windows, where a `g:` symlink means nothing and `Path::join` would read it
/// as a drive specifier and drop the rest of the path.
#[cfg(windows)]
pub fn map_drive_letter(
    _prefix: &Path,
    _letter: char,
    _target: &Path,
    _create: bool,
) -> CoreResult<PathBuf> {
    Err(crate::error::CoreError::Other(
        "drive letters are only mapped inside a Wine prefix".into(),
    ))
}

/// Screen DPI from a scale factor and logical height. Wine's baseline is 96 DPI at 100%,
/// and GTK reports only whole factors, so the height has to make up for a dense screen.
pub fn dpi_for_screen(scale: f64, logical_height: Option<u32>) -> u32 {
    let from_scale = 96.0 * scale;
    // The share of the screen an installer takes at 1080p, in Windows' 25% steps.
    let from_height = logical_height.map_or(0.0, |height| {
        (f64::from(height) / 1080.0 * 4.0).round() * 24.0
    });
    let dpi = from_scale.max(from_height).round() as u32;
    // Below 96 nothing renders correctly, and beyond 240 some installers lay out badly.
    dpi.clamp(96, 240)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-prefix-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn windows_safe_names_drop_metacharacters_without_emptying() {
        // `(63) Artisan TD` is legal on Windows but breaks the batch scripts a repack
        // installer runs internally. A name made only of those falls back to "Game".
        for (title, expected) in [
            ("(63) Artisan TD", "63 Artisan TD"),
            ("Command & Conquer", "Command Conquer"),
            ("Game [GOG]", "Game GOG"),
            ("()", "Game"),
            ("   ", "Game"),
        ] {
            assert_eq!(windows_safe_name(title), expected, "{title:?}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_source_drive_can_be_mapped_without_creating_it() {
        let dir = scratch("source");
        let prefix = dir.join("pfx");
        let target = dir.join("extracted");
        std::fs::create_dir_all(&target).unwrap();

        let link = map_drive_letter(&prefix, SOURCE_DRIVE, &target, false).unwrap();
        assert_eq!(link, prefix.join("dosdevices/r:"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_mapped_drives_avoid_the_letters_proton_manages() {
        // Proton removes `s:` and `t:` on every launch, so neither may be one of ours.
        for drive in [GAMES_DRIVE, SOURCE_DRIVE] {
            assert!(!"CZST".contains(drive), "{drive}: is Proton's to manage");
        }
    }

    #[test]
    fn the_drive_path_names_the_games_own_folder() {
        // A bare "G:\" is rejected by Inno Setup, so the path must reach a subfolder.
        assert_eq!(games_drive_path("(78) Wall World"), "G:\\(78) Wall World");
    }

    #[test]
    #[cfg(unix)]
    fn maps_a_drive_letter_to_the_games_folder() {
        let dir = scratch("map");
        let prefix = dir.join("pfx");
        let target = dir.join("Installations/(1) Game");

        let link = map_drive(&prefix, &target).unwrap();
        assert_eq!(link, prefix.join("dosdevices/g:"));
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert!(target.is_dir());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn remapping_replaces_the_previous_target() {
        // The games folder can move, and a stale symlink would send an installer to the
        // wrong place.
        let dir = scratch("remap");
        let prefix = dir.join("pfx");

        map_drive(&prefix, &dir.join("first")).unwrap();
        let link = map_drive(&prefix, &dir.join("second")).unwrap();

        assert_eq!(std::fs::read_link(&link).unwrap(), dir.join("second"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn both_optional_component_prompts_are_actually_disabled() {
        // `mscoree=,mshtml=` is one entry whose mode never reaches mshtml; entries need `;`.
        assert_eq!(DllOverrides::no_prompts().to_env(), "mscoree=;mshtml=");
    }

    #[test]
    fn overrides_from_two_sources_combine_instead_of_replacing_each_other() {
        let combined =
            DllOverrides::no_prompts().merged_with(DllOverrides::native(&["d3d11", "dxgi"]));
        assert_eq!(
            combined.to_env(),
            "d3d11=native,builtin;dxgi=native,builtin;mscoree=;mshtml="
        );
    }

    #[test]
    fn a_games_own_override_wins_over_the_one_we_set() {
        // Pasting `dxgi=builtin` from a wiki is how a user turns DXVK off for one game.
        let combined = DllOverrides::native(&["d3d11", "dxgi"])
            .merged_with(DllOverrides::parse("dxgi=builtin"));
        assert_eq!(combined.to_env(), "d3d11=native,builtin;dxgi=builtin");
    }

    #[test]
    fn an_override_value_round_trips_through_parse_and_back() {
        for value in ["d3d11=native,builtin", "mscoree=;mshtml=", ""] {
            assert_eq!(DllOverrides::parse(value).to_env(), value, "{value:?}");
        }
    }

    #[test]
    fn a_comma_shares_one_mode_between_names_the_way_wine_reads_it() {
        assert_eq!(
            DllOverrides::parse("comdlg32,shell32=n,b").to_env(),
            "comdlg32=n,b;shell32=n,b"
        );
    }

    #[test]
    fn the_override_commands_write_one_registry_value_per_dll() {
        let runtime = WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let commands = override_commands(
            &runtime,
            Path::new("/pfx"),
            &DllOverrides::native(&["d3d12"]),
        );

        assert_eq!(commands.len(), 1);
        let args: Vec<String> = commands[0]
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().any(|a| a == "d3d12"), "{args:?}");
        assert!(args.iter().any(|a| a == "native,builtin"), "{args:?}");
        assert!(args.iter().any(|a| a.contains("DllOverrides")), "{args:?}");
    }

    #[test]
    fn preparation_commands_suppress_the_optional_component_prompts() {
        // Unanswered Mono/Gecko dialogs are what makes a fresh prefix appear to hang.
        let runtime = WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let boot = boot_command(&runtime, Path::new("/pfx"));
        assert_eq!(
            boot.env["WINEDLLOVERRIDES"],
            DllOverrides::no_prompts().to_env()
        );
        assert_eq!(boot.env["WINEPREFIX"], "/pfx");
        assert!(boot.args.iter().any(|a| a == "wineboot"));
    }

    #[test]
    fn the_dpi_command_writes_the_registry_value() {
        let runtime = WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let cmd = dpi_command(&runtime, Path::new("/pfx"), 144);
        assert!(cmd.args.iter().any(|a| a == "LogPixels"));
        assert!(cmd.args.iter().any(|a| a == "144"));
    }

    #[test]
    fn umu_preparation_hands_proton_its_build_and_leaves_mono_alone() {
        let runtime = WindowsRuntime::Umu {
            launcher: PathBuf::from("/app/bin/umu-run"),
            proton: PathBuf::from("/cfg/proton/UMU-Proton-10.0-4"),
            build: "UMU-Proton-10.0-4".into(),
        };
        let cmd = boot_command(&runtime, Path::new("/pfx"));
        assert_eq!(cmd.program, "/app/bin/umu-run");
        assert_eq!(cmd.env["WINEPREFIX"], "/pfx");
        assert_eq!(cmd.env["PROTONPATH"], "/cfg/proton/UMU-Proton-10.0-4");
        assert_eq!(cmd.env["STORE"], "none");
        assert!(!cmd.env.contains_key("WINEDLLOVERRIDES"));
        assert_eq!(cmd.args.first().unwrap(), "wineboot");
    }

    #[test]
    fn the_setup_registry_file_carries_the_dpi_in_hex() {
        let file = registry_file(144);
        assert!(file.starts_with("REGEDIT4"), "{file}");
        assert!(
            file.contains("[HKEY_CURRENT_USER\\Control Panel\\Desktop]"),
            "{file}"
        );
        assert!(file.contains("\"LogPixels\"=dword:00000090"), "{file}");
    }

    #[test]
    fn a_host_path_reaches_a_windows_program_through_the_z_drive() {
        assert_eq!(
            z_drive_path(Path::new("/home/a b/pfx/.gameyfin-setup.reg")),
            "Z:\\home\\a b\\pfx\\.gameyfin-setup.reg"
        );
        let runtime = WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let cmd = import_command(&runtime, Path::new("/pfx"), Path::new("/pfx/x.reg"));
        let args: Vec<String> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["regedit", "/S", "Z:\\pfx\\x.reg"]);
    }

    #[test]
    fn the_theme_is_only_offered_when_the_build_ships_one() {
        let dir = scratch("theme");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!has_bundled_theme(&dir), "nothing installed yet");

        let theme = dir.join("drive_c/windows/resources/themes/aero");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join("aero.msstyles"), b"x").unwrap();
        assert!(has_bundled_theme(&dir));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_theme_commands_name_wines_own_theme() {
        // "Blue" is the colour name inside Wine's aero.msstyles. The "NormalColor" that
        // most Windows themes use is silently ignored, leaving the classic look.
        let runtime = WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let cmds = theme_commands(&runtime, Path::new("/pfx"));
        assert_eq!(cmds.len(), 4);

        let all: Vec<String> = cmds
            .iter()
            .flat_map(|c| c.args.iter().map(|a| a.to_string_lossy().into_owned()))
            .collect();
        assert!(all.contains(&"Blue".to_string()));
        assert!(all.contains(&"NormalSize".to_string()));
        assert!(all.iter().any(|a| a.ends_with("aero.msstyles")));
        assert!(all.iter().any(|a| a.contains("ThemeManager")));
    }

    #[test]
    fn preparation_is_recorded_per_dpi() {
        let dir = scratch("ready");
        std::fs::create_dir_all(dir.join("drive_c")).unwrap();
        let state = PrefixState {
            dpi: 144,
            runtime: "staging-wow64 11.17".to_string(),
            ..PrefixState::default()
        };
        assert!(!is_prepared(&dir, &state));

        mark_prepared(&dir, &state).unwrap();
        assert!(is_prepared(&dir, &state));
        // A changed scale factor must trigger preparation again.
        let rescaled = PrefixState {
            dpi: 192,
            ..state.clone()
        };
        assert!(!is_prepared(&dir, &rescaled));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wine_or_component_change_alone_re_runs_preparation() {
        // `wineboot` puts Wine's own Direct3D DLLs back, so a build change has to reinstall
        // the graphics layer even when nothing else moved.
        let dir = scratch("versions");
        std::fs::create_dir_all(dir.join("drive_c")).unwrap();
        let state = PrefixState {
            dpi: 96,
            runtime: "proton 11.0-2".to_string(),
            dxvk: Some("3.1".to_string()),
            vkd3d: Some("3.0.1".to_string()),
        };
        mark_prepared(&dir, &state).unwrap();
        assert!(is_prepared(&dir, &state));

        for changed in [
            PrefixState {
                runtime: "staging-wow64 11.17".to_string(),
                ..state.clone()
            },
            PrefixState {
                dxvk: Some("3.0.2".to_string()),
                ..state.clone()
            },
            PrefixState {
                vkd3d: None,
                ..state.clone()
            },
        ] {
            assert!(!is_prepared(&dir, &changed), "{changed:?}");
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_marker_that_does_not_parse_means_the_prefix_is_not_prepared() {
        // A marker in an older format must trigger a rebuild.
        let dir = scratch("stale-marker");
        std::fs::create_dir_all(dir.join("drive_c")).unwrap();
        std::fs::write(dir.join(READY_MARKER), "144").unwrap();

        assert!(!is_prepared(
            &dir,
            &PrefixState {
                dpi: 144,
                ..PrefixState::default()
            }
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_proton_prefix_is_found_inside_pfx() {
        // Proton builds the real prefix in `pfx`; Wine's layout at the top level would read as
        // broken and put drive mappings where Wine never looks.
        let dir = std::env::temp_dir().join(format!("gameyfin-pfx-{}", std::process::id()));
        // Cleared first: a panicked earlier run leaves exactly the state checked here.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pfx").join("drive_c")).unwrap();

        assert_eq!(wine_root(&dir), dir.join("pfx"));

        let state = PrefixState {
            dpi: 96,
            ..PrefixState::default()
        };
        mark_prepared(&dir, &state).unwrap();
        assert!(
            is_prepared(&dir, &state),
            "a Proton prefix must count as ready"
        );

        // Drive mappings land where Wine reads them, not beside them.
        let link = map_drive_letter(&dir, 'G', &dir.join("target"), true).unwrap();
        assert!(
            link.starts_with(dir.join("pfx").join("dosdevices")),
            "got: {link:?}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_wine_prefix_stays_at_its_own_root() {
        let dir = std::env::temp_dir().join(format!("gameyfin-wineroot-{}", std::process::id()));
        // Cleared first: a panicked earlier run leaves exactly the state checked here.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drive_c")).unwrap();

        // No `pfx`, so nothing is redirected: plain Wine owns the prefix directory itself.
        assert_eq!(wine_root(&dir), dir);
        let state = PrefixState {
            dpi: 96,
            ..PrefixState::default()
        };
        assert!(!is_prepared(&dir, &state), "not ready until it is marked");
        mark_prepared(&dir, &state).unwrap();
        assert!(is_prepared(&dir, &state));

        let link = map_drive_letter(&dir, 'G', &dir.join("target"), true).unwrap();
        assert!(link.starts_with(dir.join("dosdevices")), "got: {link:?}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_prefix_without_drive_c_is_never_considered_ready() {
        // A broken prefix marked as prepared would fail identically every time.
        let dir = scratch("broken");
        let state = PrefixState {
            dpi: 144,
            ..PrefixState::default()
        };
        mark_prepared(&dir, &state).unwrap();
        assert!(!is_prepared(&dir, &state));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dpi_tracks_the_scale_factor_within_a_usable_range() {
        for (scale, expected) in [
            (1.0, 96),
            (1.5, 144),
            (2.0, 192),
            // Clamped at both ends, so an absurd scale still gives a readable prefix.
            (0.1, 96),
            (10.0, 240),
        ] {
            assert_eq!(dpi_for_screen(scale, None), expected, "scale {scale}");
        }
    }

    #[test]
    fn a_dense_screen_gets_a_larger_dpi_even_when_no_scale_is_reported() {
        for (height, expected) in [
            (1080, 96),
            (1200, 96),
            (1440, 120),
            (1600, 144),
            (1824, 168),
            (2160, 192),
            (2880, 240),
        ] {
            assert_eq!(
                dpi_for_screen(1.0, Some(height)),
                expected,
                "height {height}"
            );
        }
    }

    #[test]
    fn a_reported_scale_is_not_doubled_by_the_height() {
        // A 4K screen at 200% is 1080 logical pixels high: the scale alone decides.
        assert_eq!(dpi_for_screen(2.0, Some(1080)), 192);
        assert_eq!(dpi_for_screen(1.5, Some(1440)), 144);
    }
}

#[cfg(test)]
mod prefix_home_tests {
    use super::*;

    fn prefix(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gameyfin-pfx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn with_users(dir: &Path, users: &[&str]) {
        for user in users {
            std::fs::create_dir_all(dir.join("drive_c").join("users").join(user)).unwrap();
        }
    }

    #[test]
    fn a_plain_wine_prefix_uses_the_single_account() {
        let dir = prefix("wine");
        with_users(&dir, &["alice"]);

        assert_eq!(Some(dir.join("drive_c/users/alice")), prefix_home(&dir));
    }

    #[test]
    fn proton_wins_when_a_prefix_has_several_accounts() {
        let dir = prefix("proton");
        with_users(&dir, &["alice", "steamuser"]);

        assert_eq!(Some(dir.join("drive_c/users/steamuser")), prefix_home(&dir));
    }

    #[test]
    fn the_shared_public_profile_is_never_the_users_own() {
        let dir = prefix("public");
        with_users(&dir, &["Public", "alice"]);

        assert_eq!(Some(dir.join("drive_c/users/alice")), prefix_home(&dir));
    }

    #[test]
    fn an_ambiguous_prefix_gives_up_rather_than_guessing() {
        // Two real accounts and no steamuser: picking one could restore a save into the
        // wrong profile, which is worse than declining.
        let dir = prefix("ambiguous");
        with_users(&dir, &["alice", "bob"]);

        assert_eq!(None, prefix_home(&dir));
    }

    #[test]
    fn a_prefix_that_does_not_exist_yet_has_no_home() {
        assert_eq!(None, prefix_home(Path::new("/nonexistent/prefix")));
    }
}
