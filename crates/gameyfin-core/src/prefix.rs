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

/// Marker recording that a prefix has been prepared, so it happens once.
const READY_MARKER: &str = ".gameyfin-ready";

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

/// Whether a prefix has already been prepared at this DPI.
pub fn is_prepared(prefix: &Path, dpi: u32) -> bool {
    // `drive_c` is checked as well as the marker: a prefix built by an earlier version
    // may have been recorded as ready while actually being unusable, and reusing it would
    // repeat the failure on every launch.
    wine_root(prefix).join("drive_c").is_dir()
        && std::fs::read_to_string(prefix.join(READY_MARKER))
            .map(|content| content.trim() == dpi.to_string())
            .unwrap_or(false)
}

/// Record that a prefix has been prepared.
pub fn mark_prepared(prefix: &Path, dpi: u32) -> CoreResult<()> {
    std::fs::write(prefix.join(READY_MARKER), dpi.to_string())?;
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

/// Build a command that runs a Wine tool inside a prefix.
fn registry_command(
    runtime: &WindowsRuntime,
    prefix: &Path,
    args: &[&str],
) -> crate::launch::ResolvedCommand {
    use std::collections::BTreeMap;

    let mut env = BTreeMap::new();
    env.insert("WINEDLLOVERRIDES".to_string(), NO_PROMPTS.to_string());

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

    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &link)?;

    Ok(link)
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
