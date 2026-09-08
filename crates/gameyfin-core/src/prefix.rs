//! Preparing a Wine or Proton prefix.
//!
//! Two problems this solves.
//!
//! **Where does the user install to?** A Windows installer offers a Windows path,
//! `C:\GOG Games\Wall World`, which lands inside the prefix, not in the games folder.
//! Telling someone to type a Linux path into a Windows file dialog does not work either.
//! So the games folder is mapped to a spare drive letter inside the prefix: the user
//! picks `G:\`, and the files arrive exactly where the app expects them.
//!
//! **Why are the windows tiny?** Wine assumes 96 DPI. On a high-resolution display an
//! installer renders at a fraction of its intended size. The prefix's DPI is set to match
//! the actual screen instead.

use std::path::{Path, PathBuf};

use crate::error::CoreResult;
use crate::runtime::WindowsRuntime;

/// Environment that stops Wine asking to install its optional components.
///
/// A fresh prefix otherwise pops up "install Mono?" and "install Gecko?" dialogs. Those
/// appear behind the setup window or off-screen, and until they are answered the prefix
/// update never finishes, which looks exactly like Wine hanging forever. Games need
/// neither component in the overwhelming majority of cases.
pub const NO_PROMPTS: &str = "mscoree=,mshtml=";

/// Keep the desktop's input method out of Wine's key handling.
///
/// With ibus or fcitx running, the input method sees key presses through XIM before Wine
/// does and holds them back until it knows they are not the start of a composed
/// character. A game reading key state directly then needs a long press on A, S or D
/// before it registers anything, and the desktop's accent picker appears over the game.
///
/// `XMODIFIERS` is the only variable involved: it is what Wine reads to decide whether to
/// open an input method at all, and `@im=none` means it does not. The GTK and Qt
/// equivalents are deliberately not set, because Wine uses neither toolkit.
///
/// The cost is composing accented or CJK text inside a Windows program, which a game
/// launcher does not need. Plain typing never goes through the input method and is
/// unaffected.
pub fn without_input_method(env: &mut std::collections::BTreeMap<String, String>) {
    env.insert("XMODIFIERS".to_string(), "@im=none".to_string());
}

/// Marker recording that a prefix has been prepared, so it happens once.
const READY_MARKER: &str = ".gameyfin-ready";

/// Bumped whenever preparation starts doing something new.
///
/// The marker holds this alongside the DPI, so a prefix prepared by an older version is
/// brought up to date on its next launch instead of keeping whatever it was given first.
/// Version 2 added the theme.
const PREPARATION_VERSION: u32 = 2;

/// The directory Wine actually keeps `drive_c` and `dosdevices` in.
///
/// Wine treats the prefix directory as the prefix itself. Proton does not: it is handed
/// the same directory as `STEAM_COMPAT_DATA_PATH` and builds the real Wine prefix in a
/// `pfx` subdirectory of it, so everything Wine owns sits one level deeper.
///
/// Assuming the Wine layout makes a Proton prefix look broken immediately after it was
/// built correctly, `drive_c` is missing from where we looked, and drive mappings get
/// written to a `dosdevices` that Wine never reads.
///
/// `pfx` is created by Proton's own first run, so before that this correctly reports the
/// prefix root and preparation proceeds.
pub fn wine_root(prefix: &Path) -> PathBuf {
    let proton = prefix.join("pfx");
    if proton.is_dir() {
        proton
    } else {
        prefix.to_path_buf()
    }
}

/// What the marker holds: the preparation version and the DPI it was prepared at.
fn marker_contents(dpi: u32) -> String {
    format!("{PREPARATION_VERSION}:{dpi}")
}

/// Whether a prefix is already prepared, at this DPI and by this version.
pub fn is_prepared(prefix: &Path, dpi: u32) -> bool {
    // `drive_c` is checked as well as the marker: a prefix built by an earlier version
    // may have been recorded as ready while actually being unusable, and reusing it would
    // repeat the failure on every launch.
    wine_root(prefix).join("drive_c").is_dir()
        && std::fs::read_to_string(prefix.join(READY_MARKER))
            .map(|content| content.trim() == marker_contents(dpi))
            .unwrap_or(false)
}

/// Record that a prefix has been prepared.
pub fn mark_prepared(prefix: &Path, dpi: u32) -> CoreResult<()> {
    std::fs::write(prefix.join(READY_MARKER), marker_contents(dpi))?;
    Ok(())
}

/// The command that initialises a prefix without prompting.
pub fn boot_command(runtime: &WindowsRuntime, prefix: &Path) -> crate::launch::ResolvedCommand {
    registry_command(runtime, prefix, &["wineboot", "-u"])
}

/// The command that sets a prefix's DPI.
///
/// Applied through the runtime rather than by editing `user.reg`, because Wine rewrites
/// that file when it starts and an appended fragment can simply be lost.
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

/// Where wine.inf installs the theme Wine bundles, relative to the prefix root.
const AERO_THEME: &str = "drive_c/windows/resources/themes/aero/aero.msstyles";

/// The same path as Wine sees it.
const AERO_THEME_WINDOWS: &str = "C:\\windows\\resources\\themes\\aero\\aero.msstyles";

/// Whether this prefix has the bundled theme for the registry to point at.
///
/// Checked rather than assumed: a stripped Wine build can omit `aero.msstyles`, and
/// pointing the registry at a file that is not there leaves the prefix looking exactly as
/// it did, with nothing to say why.
pub fn has_bundled_theme(prefix: &Path) -> bool {
    wine_root(prefix).join(AERO_THEME).is_file()
}

/// Commands that turn on the theme Wine ships.
///
/// Without it Wine draws the classic Windows 2000 caption and controls: a flat blue title
/// bar and square grey buttons. Wine's own `wine.inf` sets these values when it creates a
/// prefix, so this is a repair for prefixes where that did not take effect, and it is a
/// no-op everywhere else.
///
/// `ColorName` really is "Blue" rather than the "NormalColor" most Windows themes use;
/// that is the name inside Wine's own theme, and a wrong one is ignored silently.
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

/// Build a command that runs a Wine tool inside a prefix.
fn registry_command(
    runtime: &WindowsRuntime,
    prefix: &Path,
    args: &[&str],
) -> crate::launch::ResolvedCommand {
    use std::collections::BTreeMap;

    let mut env = BTreeMap::new();
    env.insert("WINEDLLOVERRIDES".to_string(), NO_PROMPTS.to_string());
    without_input_method(&mut env);

    match runtime {
        WindowsRuntime::Umu { .. } => {
            env.insert(
                "WINEPREFIX".to_string(),
                prefix.to_string_lossy().into_owned(),
            );
            env.insert(
                "PROTONPATH".to_string(),
                crate::launch::DEFAULT_PROTON.into(),
            );
            env.insert("GAMEID".to_string(), crate::launch::DEFAULT_UMU_ID.into());
        }
        WindowsRuntime::SteamProton { steam_root, .. } => {
            env.insert(
                "STEAM_COMPAT_DATA_PATH".to_string(),
                prefix.to_string_lossy().into_owned(),
            );
            env.insert(
                "STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(),
                steam_root.to_string_lossy().into_owned(),
            );
        }
        WindowsRuntime::Wine { .. } | WindowsRuntime::Bundled { .. } | WindowsRuntime::HostWine => {
            env.insert(
                "WINEPREFIX".to_string(),
                prefix.to_string_lossy().into_owned(),
            );
        }
    }

    // Proton is invoked as `proton run <tool>`; the others take the tool directly.
    let leading: Vec<String> = match runtime {
        WindowsRuntime::SteamProton { .. } => vec!["run".to_string()],
        _ => Vec::new(),
    };
    let full: Vec<String> = leading
        .into_iter()
        .chain(args.iter().map(|a| a.to_string()))
        .collect();

    // The same wrapper the launcher uses, so environment handling cannot diverge between
    // preparing a prefix and running a game in it.
    let full_args = runtime.wrap_args(&env, None, full);

    crate::launch::ResolvedCommand {
        program: runtime.program().to_path_buf().into_os_string(),
        args: full_args,
        env,
        working_dir: None,
    }
}

/// Drive letter the games folder is mapped to inside a prefix.
///
/// `G` for games, far enough down the alphabet to avoid the letters Windows software
/// assumes are taken.
pub const GAMES_DRIVE: char = 'G';

/// Drive letter the installer's own folder is mapped to.
///
/// An installer is run through this rather than by its Linux path, so the path it sees
/// contains none of the characters our folder naming uses. Repack installers in
/// particular shell out to batch scripts, where an unquoted `(` is a syntax error.
pub const SOURCE_DRIVE: char = 'S';

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

/// The Windows path a user should type to install a particular game.
///
/// A bare drive root is not accepted by every installer, Inno Setup in particular
/// rejects `G:\` with "You must enter a full path with drive letter", so the drive is
/// mapped to the installations *root* and the game's own folder named after it.
pub fn games_drive_path(folder: &str) -> String {
    format!("{GAMES_DRIVE}:\\{folder}")
}

/// Point the games drive at a directory.
pub fn map_drive(prefix: &Path, target: &Path) -> CoreResult<PathBuf> {
    map_drive_letter(prefix, GAMES_DRIVE, target, true)
}

/// Point any drive letter at a directory.
///
/// Wine reads `dosdevices` to map drive letters, and a symlink there is all it takes, no
/// registry edit and no `winecfg` round trip.
///
/// `create` controls whether a missing target is created, appropriate for a destination,
/// wrong for a source that should already exist.
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

/// Refused on Windows, where there is no prefix and nothing to map.
///
/// A mapping is a symlink named `g:` inside the prefix, which only means anything to
/// Wine. Windows has no such prefix, cannot use a colon in a filename, and `Path::join`
/// there reads `g:` as a drive specifier and returns it *in place of* the whole path.
/// Refusing keeps that from looking like it worked. Unreachable in practice: every caller
/// is behind `needs_proton`, which is always false on Windows.
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

/// Screen DPI for a scale factor, rounded to something Windows software expects.
///
/// Wine's baseline is 96 DPI at 100%.
pub fn dpi_for_scale(scale: f64) -> u32 {
    let dpi = (96.0 * scale).round() as u32;
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
    fn windows_safe_names_drop_shell_metacharacters() {
        // `(63) Artisan TD` is legal on Windows but breaks the batch scripts a repack
        // installer runs internally.
        assert_eq!(windows_safe_name("(63) Artisan TD"), "63 Artisan TD");
        assert_eq!(windows_safe_name("Command & Conquer"), "Command Conquer");
        assert_eq!(windows_safe_name("Game [GOG]"), "Game GOG");
    }

    #[test]
    fn windows_safe_names_never_come_back_empty() {
        assert_eq!(windows_safe_name("()"), "Game");
        assert_eq!(windows_safe_name("   "), "Game");
    }

    #[test]
    #[cfg(unix)]
    fn a_source_drive_can_be_mapped_without_creating_it() {
        let dir = scratch("source");
        let prefix = dir.join("pfx");
        let target = dir.join("extracted");
        std::fs::create_dir_all(&target).unwrap();

        let link = map_drive_letter(&prefix, SOURCE_DRIVE, &target, false).unwrap();
        assert_eq!(link, prefix.join("dosdevices/s:"));
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn preparation_commands_suppress_the_optional_component_prompts() {
        // Unanswered Mono/Gecko dialogs are what makes a fresh prefix appear to hang.
        let runtime = WindowsRuntime::Wine {
            path: PathBuf::from("/usr/bin/wine"),
        };
        let boot = boot_command(&runtime, Path::new("/pfx"));
        assert_eq!(boot.env["WINEDLLOVERRIDES"], NO_PROMPTS);
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
    fn proton_preparation_uses_its_own_variables() {
        let runtime = WindowsRuntime::SteamProton {
            script: PathBuf::from("/steam/Proton/proton"),
            steam_root: PathBuf::from("/steam"),
            name: "Proton".into(),
        };
        let cmd = boot_command(&runtime, Path::new("/pfx"));
        assert_eq!(cmd.env["STEAM_COMPAT_DATA_PATH"], "/pfx");
        assert_eq!(cmd.args.first().unwrap(), "run");
        assert!(!cmd.env.contains_key("WINEPREFIX"));
    }

    #[test]
    fn a_prefix_prepared_by_an_older_version_is_prepared_again() {
        // The marker carries the preparation version, so a prefix that predates a change
        // to what preparation does is brought up to date rather than left behind.
        let dir = scratch("stale");
        std::fs::create_dir_all(dir.join("drive_c")).unwrap();
        std::fs::write(dir.join(".gameyfin-ready"), "96").unwrap();

        assert!(
            !is_prepared(&dir, 96),
            "a bare DPI marker is from an older version"
        );

        mark_prepared(&dir, 96).unwrap();
        assert!(is_prepared(&dir, 96));

        std::fs::remove_dir_all(&dir).unwrap();
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
        assert!(!is_prepared(&dir, 144));

        mark_prepared(&dir, 144).unwrap();
        assert!(is_prepared(&dir, 144));
        // A changed scale factor must trigger preparation again.
        assert!(!is_prepared(&dir, 192));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_proton_prefix_is_found_inside_pfx() {
        // Proton is handed the compat data directory and builds the real prefix in `pfx`.
        // Looking for Wine's layout at the top level reports a freshly built prefix as
        // broken, and writes drive mappings where Wine will never read them.
        let dir = std::env::temp_dir().join(format!("gameyfin-pfx-{}", std::process::id()));
        // Cleared first: a previous run that panicked leaves this behind, and the
        // leftover state is exactly what these assertions are checking for.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pfx").join("drive_c")).unwrap();

        assert_eq!(wine_root(&dir), dir.join("pfx"));

        mark_prepared(&dir, 96).unwrap();
        assert!(is_prepared(&dir, 96), "a Proton prefix must count as ready");

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
        // Cleared first: a previous run that panicked leaves this behind, and the
        // leftover state is exactly what these assertions are checking for.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drive_c")).unwrap();

        // No `pfx`, so nothing is redirected: plain Wine owns the prefix directory itself.
        assert_eq!(wine_root(&dir), dir);
        assert!(!is_prepared(&dir, 96), "not ready until it is marked");
        mark_prepared(&dir, 96).unwrap();
        assert!(is_prepared(&dir, 96));

        let link = map_drive_letter(&dir, 'G', &dir.join("target"), true).unwrap();
        assert!(link.starts_with(dir.join("dosdevices")), "got: {link:?}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_prefix_without_drive_c_is_never_considered_ready() {
        // An earlier version could mark a broken prefix as prepared; reusing it would
        // fail identically every time.
        let dir = scratch("broken");
        mark_prepared(&dir, 144).unwrap();
        assert!(!is_prepared(&dir, 144));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dpi_tracks_the_scale_factor() {
        assert_eq!(dpi_for_scale(1.0), 96);
        assert_eq!(dpi_for_scale(1.5), 144);
        assert_eq!(dpi_for_scale(2.0), 192);
    }

    #[test]
    fn dpi_is_clamped_to_a_usable_range() {
        assert_eq!(dpi_for_scale(0.1), 96);
        assert_eq!(dpi_for_scale(10.0), 240);
    }
}
