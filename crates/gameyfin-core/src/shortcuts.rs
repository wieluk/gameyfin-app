//! Desktop and menu shortcuts. Each runs the app with `--launch <id>`, not the game, so the
//! prefix, runtime choice and playtime tracking still apply.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Location {
    /// The user's desktop.
    Desktop,
    /// The application menu, or the Start menu on Windows.
    Menu,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub game_id: i64,
    pub title: String,
    /// The Gameyfin executable the shortcut runs.
    pub launcher: PathBuf,
    /// Arguments before `--launch`: empty for an ordinary install, `run <app-id>` inside a
    /// Flatpak, where the launcher is `flatpak` itself.
    pub launcher_args: Vec<String>,
    /// Absolute path to the game's icon file, written by the caller.
    pub icon: Option<PathBuf>,
}

/// A shortcut's file name. Windows shows it, so it is the title alone; elsewhere `Name=` is
/// shown and the id goes in the file name, where it survives a rename.
pub fn stem(game_id: i64, title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if crate::prefix::is_illegal_filename_char(c) || c == '.' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        format!("gameyfin-{game_id}")
    } else if cfg!(windows) {
        trimmed
    } else {
        format!("gameyfin-{game_id} {trimmed}")
    }
}

/// Builds a freedesktop desktop entry. `Exec` is unquoted words with `%` escapes, so a path
/// with a space or a percent has to be quoted rather than pasted in.
pub fn desktop_entry(target: &Target) -> String {
    let mut entry = String::from("[Desktop Entry]\n");
    entry.push_str("Type=Application\n");
    entry.push_str("Version=1.0\n");
    entry.push_str(&format!("Name={}\n", desktop_value(&target.title)));
    entry.push_str(&format!(
        "Comment={}\n",
        desktop_value(&format!("Play {} with Gameyfin", target.title))
    ));
    entry.push_str(&format!("Exec={}\n", exec_line(target)));
    if let Some(icon) = &target.icon {
        entry.push_str(&format!(
            "Icon={}\n",
            desktop_value(&icon.to_string_lossy())
        ));
    } else {
        // The name the deb and rpm install the app's icon under, for when no file was written.
        entry.push_str("Icon=gameyfin-app\n");
    }
    entry.push_str("Terminal=false\n");
    entry.push_str("Categories=Game;\n");
    // Read back by `installed_for` to tell our shortcuts from the user's own.
    entry.push_str(&format!("X-Gameyfin-Game-Id={}\n", target.game_id));
    entry
}

/// The whole `Exec` value: the launcher, anything it needs first, then the game.
fn exec_line(target: &Target) -> String {
    let mut parts = vec![exec_argument(&target.launcher.to_string_lossy())];
    parts.extend(target.launcher_args.iter().map(|arg| exec_argument(arg)));
    parts.push("--launch".to_string());
    parts.push(target.game_id.to_string());
    parts.join(" ")
}

/// The arguments a `.lnk` carries, in the same order.
fn launch_arguments(target: &Target) -> String {
    let mut parts = target.launcher_args.clone();
    parts.push("--launch".to_string());
    parts.push(target.game_id.to_string());
    parts.join(" ")
}

/// Escape a value for a desktop-entry key: backslash, newline, tab and CR carry meaning.
fn desktop_value(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\t' => out.push_str(r"\t"),
            '\r' => out.push_str(r"\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Quotes one `Exec` argument: backslash, quote, backtick and dollar are reserved, and `%`
/// doubles because a single one introduces a field code.
fn exec_argument(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' | '"' | '`' | '$' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// PowerShell that writes a `.lnk`, since a `.url` cannot carry arguments. Returned, not run,
/// so its quoting can be tested.
pub fn windows_shortcut_script(target: &Target, destination: &Path) -> String {
    let icon = target
        .icon
        .clone()
        .unwrap_or_else(|| target.launcher.clone());
    format!(
        "$ErrorActionPreference='Stop'; \
         $s=(New-Object -ComObject WScript.Shell).CreateShortcut({lnk}); \
         $s.TargetPath={exe}; \
         $s.Arguments={args}; \
         $s.WorkingDirectory={cwd}; \
         $s.IconLocation={icon}; \
         $s.Description={description}; \
         $s.Save()",
        lnk = powershell_string(&destination.to_string_lossy()),
        exe = powershell_string(&target.launcher.to_string_lossy()),
        args = powershell_string(&launch_arguments(target)),
        cwd = powershell_string(
            &target
                .launcher
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        ),
        icon = powershell_string(&icon.to_string_lossy()),
        description = powershell_string(&format!("Play {} with Gameyfin", target.title)),
    )
}

/// Quote a value as a PowerShell single-quoted string (no expansion inside; a literal `'`
/// is doubled), so a title like `$(Get-Process)` stays a title.
fn powershell_string(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Windows' flag for "start this process without a console window".
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn extension() -> &'static str {
    if cfg!(windows) {
        "lnk"
    } else {
        "desktop"
    }
}

/// Where shortcuts of a kind belong. `None` rather than a guess: scattering files into an
/// invented directory is worse than making no shortcut.
pub fn directory_for(home: &Path, location: Location) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        match location {
            Location::Desktop => Some(home.join("Desktop")),
            // The per-user Start menu, which needs no administrator rights.
            Location::Menu => Some(
                home.join("AppData")
                    .join("Roaming")
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs")
                    .join("Gameyfin"),
            ),
        }
    }
    #[cfg(not(windows))]
    {
        match location {
            Location::Desktop => Some(desktop_dir(home)),
            Location::Menu => Some(home.join(".local").join("share").join("applications")),
        }
    }
}

/// The user's desktop directory, which is localised: `XDG_DESKTOP_DIR`, then
/// `user-dirs.dirs`, then the English default.
#[cfg(not(windows))]
fn desktop_dir(home: &Path) -> PathBuf {
    if let Some(from_env) = std::env::var_os("XDG_DESKTOP_DIR") {
        let path = PathBuf::from(from_env);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }

    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));

    std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|contents| parse_user_dir(&contents, "XDG_DESKTOP_DIR", home))
        .unwrap_or_else(|| home.join("Desktop"))
}

/// Reads one directory out of `user-dirs.dirs`, whose shell-style values are written
/// relative to `$HOME` and have to be expanded.
#[cfg(not(windows))]
fn parse_user_dir(contents: &str, key: &str, home: &Path) -> Option<PathBuf> {
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }

        let value = value.trim().trim_matches('"');
        if value.is_empty() {
            return None;
        }
        return Some(match value.strip_prefix("$HOME/") {
            Some(relative) => home.join(relative),
            // A bare `$HOME` means the desktop *is* the home directory, which is how the
            // spec says to disable the folder.
            None if value == "$HOME" => home.to_path_buf(),
            None => PathBuf::from(value),
        });
    }
    None
}

pub fn create(home: &Path, location: Location, target: &Target) -> std::io::Result<PathBuf> {
    let directory = directory_for(home, location).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "there is no shortcut folder for this location on this system",
        )
    })?;
    std::fs::create_dir_all(&directory)?;

    // A renamed game gets a renamed shortcut, so the one under the old name goes.
    if let Some(previous) = existing_shortcut(&directory, target.game_id) {
        let _ = std::fs::remove_file(previous);
    }
    let path = destination(
        &directory,
        &stem(target.game_id, &target.title),
        target.game_id,
    );

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let script = windows_shortcut_script(target, &path);
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            // Without this a console window flashes up while the shortcut is written.
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(format!(
                "the shell refused to create the shortcut: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    #[cfg(not(windows))]
    std::fs::write(&path, desktop_entry(target))?;

    // Without the executable bit a desktop entry opens in a text editor instead of starting.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)?;
    }

    Ok(path)
}

/// Where to write, never over a file that belongs to something else: a game the user
/// already has their own shortcut for keeps it, and ours goes beside it.
fn destination(directory: &Path, stem: &str, game_id: i64) -> PathBuf {
    let plain = directory.join(format!("{stem}.{}", extension()));
    if !plain.exists() || is_shortcut_for(&plain, game_id) {
        return plain;
    }
    directory.join(format!("{stem} (Gameyfin).{}", extension()))
}

/// This game's shortcut in a directory, if it has one. Found by id, so a game renamed on
/// the server is still recognised.
fn existing_shortcut(directory: &Path, game_id: i64) -> Option<PathBuf> {
    std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .find_map(|entry| {
            let path = entry.path();
            let ours = path.extension().and_then(|e| e.to_str()) == Some(extension())
                && is_shortcut_for(&path, game_id);
            ours.then_some(path)
        })
}

/// Whether one file is this game's shortcut: on Windows from the `--launch` argument inside
/// it, since its name is only the title; elsewhere from the id its name carries.
fn is_shortcut_for(path: &Path, game_id: i64) -> bool {
    #[cfg(windows)]
    {
        std::fs::read(path).is_ok_and(|bytes| carries_launch_id(&bytes, game_id))
    }
    #[cfg(not(windows))]
    {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        name.starts_with(&format!("gameyfin-{game_id} "))
            || name.starts_with(&format!("gameyfin-{game_id}."))
    }
}

/// Whether a `.lnk`'s bytes carry `--launch <id>`, searched as UTF-16 and plain text rather
/// than parsing the format.
// Windows-only in use, compiled everywhere so its tests run on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
fn carries_launch_id(bytes: &[u8], game_id: i64) -> bool {
    let needle = format!("--launch {game_id}");
    let utf16: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    whole_match(bytes, needle.as_bytes(), 1) || whole_match(bytes, &utf16, 2)
}

/// Finds `needle` where the next character is not another digit, so the shortcut for game 1
/// is not found in the one for game 12. `stride` is one character's width in the encoding.
#[cfg_attr(not(windows), allow(dead_code))]
fn whole_match(haystack: &[u8], needle: &[u8], stride: usize) -> bool {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle)
        .any(|(at, _)| {
            let next = at + needle.len();
            match haystack.get(next..next + stride) {
                Some(character) => {
                    !(character[0].is_ascii_digit() && character[1..].iter().all(|b| *b == 0))
                }
                None => true,
            }
        })
}

/// Remove a game's shortcut from one location. True when there was one.
pub fn remove(home: &Path, location: Location, game_id: i64) -> bool {
    directory_for(home, location)
        .and_then(|directory| existing_shortcut(&directory, game_id))
        .is_some_and(|path| std::fs::remove_file(path).is_ok())
}

pub fn installed_for(home: &Path, game_id: i64) -> Vec<Location> {
    [Location::Desktop, Location::Menu]
        .into_iter()
        .filter(|&location| {
            directory_for(home, location)
                .and_then(|directory| existing_shortcut(&directory, game_id))
                .is_some()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_target() -> Target {
        Target {
            game_id: 12,
            title: "Celeste".to_string(),
            launcher: PathBuf::from("/usr/bin/gameyfin-app"),
            launcher_args: Vec::new(),
            icon: None,
        }
    }

    /// The same with a launcher that exists here: Windows validates the path for a `.lnk`,
    /// so the Unix one above suits only tests that stop at building the script.
    fn a_local_target() -> Target {
        Target {
            launcher: std::env::current_exe().expect("the test binary knows its own path"),
            ..a_target()
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn the_stem_carries_the_id_so_two_titles_cannot_collide() {
        assert_eq!(stem(12, "Celeste"), "gameyfin-12 Celeste");
        assert_ne!(stem(12, "Celeste"), stem(13, "Celeste"));
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_shortcut_is_named_after_the_game_alone() {
        // The file name is what the desktop and the Start menu show.
        assert_eq!(stem(12, "Celeste"), "Celeste");
    }

    #[test]
    fn a_stem_is_filesystem_safe() {
        let name = stem(1, "Where/Are: My *Saves?");
        assert!(!name.contains('/'));
        assert!(!name.contains(':'));
        assert!(!name.contains('*'));
        // A trailing dot is refused by Windows, so dots are dropped entirely.
        assert!(!stem(2, "Portal 2...").ends_with('.'));
    }

    #[test]
    fn an_unnameable_title_still_produces_a_stem() {
        assert_eq!(stem(9, "///"), "gameyfin-9");
        assert_eq!(stem(9, "   "), "gameyfin-9");
    }

    #[test]
    fn a_shortcut_is_recognised_by_the_game_it_launches() {
        let lnk = |arguments: &str| -> Vec<u8> {
            // How a .lnk holds its arguments: UTF-16, in amongst the rest of the structure.
            let mut bytes = vec![0x4c, 0x00, 0x00, 0x00];
            bytes.extend(arguments.encode_utf16().flat_map(u16::to_le_bytes));
            bytes.extend([0x00, 0x00]);
            bytes
        };

        assert!(carries_launch_id(&lnk("--launch 12"), 12));
        // The shortcut for game 1 must not answer for game 12, nor the other way round.
        assert!(!carries_launch_id(&lnk("--launch 12"), 1));
        assert!(!carries_launch_id(&lnk("--launch 1"), 12));
        assert!(carries_launch_id(
            &lnk("run org.gameyfin.gameyfin-app --launch 7"),
            7
        ));
        // Some shells write the arguments a second time as plain bytes.
        assert!(carries_launch_id(b"...--launch 3", 3));
        assert!(!carries_launch_id(b"...--launch 33", 3));
        assert!(!carries_launch_id(&lnk("--launch 4"), 5));
    }

    #[test]
    fn the_entry_launches_the_app_rather_than_the_game() {
        // Launching the .exe directly would skip prefix setup and record no playtime.
        let entry = desktop_entry(&a_target());
        assert!(
            entry.contains("Exec=\"/usr/bin/gameyfin-app\" --launch 12"),
            "got {entry}"
        );
        assert!(entry.contains("X-Gameyfin-Game-Id=12"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
    }

    #[test]
    fn a_launcher_reached_indirectly_keeps_its_own_arguments_first() {
        // The Flatpak case. `flatpak --launch 12` is not a command flatpak has, so the
        // shortcut has to read `flatpak run <app-id> --launch 12`.
        let mut target = a_target();
        target.launcher = PathBuf::from("/usr/bin/flatpak");
        target.launcher_args = vec!["run".into(), "org.gameyfin.gameyfin-app".into()];

        let entry = desktop_entry(&target);
        assert!(
            entry.contains(
                "Exec=\"/usr/bin/flatpak\" \"run\" \"org.gameyfin.gameyfin-app\" --launch 12"
            ),
            "got {entry}"
        );

        let script = windows_shortcut_script(&target, Path::new("C:\\x\\y.lnk"));
        assert!(
            script.contains("$s.Arguments='run org.gameyfin.gameyfin-app --launch 12'"),
            "got {script}"
        );
    }

    #[test]
    fn a_path_with_a_space_is_quoted() {
        let mut target = a_target();
        target.launcher = PathBuf::from("/opt/My Games/gameyfin-app");
        let entry = desktop_entry(&target);
        assert!(
            entry.contains("Exec=\"/opt/My Games/gameyfin-app\" --launch 12"),
            "got {entry}"
        );
    }

    #[test]
    fn exec_reserved_characters_are_escaped() {
        // An unescaped $ or ` would be substituted by the shell the desktop uses.
        assert_eq!(exec_argument("/a b/c$d`e\"f"), "\"/a b/c\\$d\\`e\\\"f\"");
        // A single % introduces a field code, so a literal one is doubled.
        assert_eq!(exec_argument("/100%/app"), "\"/100%%/app\"");
    }

    #[test]
    fn a_newline_in_a_title_cannot_forge_another_key() {
        // Unescaped, this would inject its own key into the entry.
        let mut target = a_target();
        target.title = "Evil\nExec=/bin/sh".to_string();
        let entry = desktop_entry(&target);
        assert!(entry.contains(r"Name=Evil\nExec=/bin/sh"), "got {entry}");
        assert_eq!(
            entry.lines().filter(|l| l.starts_with("Exec=")).count(),
            1,
            "exactly one Exec line"
        );
    }

    #[test]
    fn the_icon_falls_back_to_the_apps_own() {
        assert!(desktop_entry(&a_target()).contains("Icon=gameyfin-app"));

        let mut target = a_target();
        target.icon = Some(PathBuf::from("/cache/covers/12.png"));
        assert!(desktop_entry(&target).contains("Icon=/cache/covers/12.png"));
    }

    #[test]
    fn a_windows_shortcut_passes_the_launch_argument() {
        // The reason it is a .lnk and not a .url: an internet shortcut cannot carry one.
        let script = windows_shortcut_script(&a_target(), Path::new("C:\\x\\Celeste.lnk"));
        assert!(
            script.contains("$s.Arguments='--launch 12'"),
            "got {script}"
        );
        assert!(
            script.contains("CreateShortcut('C:\\x\\Celeste.lnk')"),
            "got {script}"
        );
        assert!(
            script.contains("$s.TargetPath='/usr/bin/gameyfin-app'"),
            "got {script}"
        );
    }

    #[test]
    fn a_title_cannot_break_out_of_the_powershell_string() {
        // Single quotes suppress expansion, so the remaining risk is a quote in the title
        // closing the string early and leaving the rest as code.
        let mut target = a_target();
        target.title = "Don't Starve'; Remove-Item C:\\ #".to_string();
        let script = windows_shortcut_script(&target, Path::new("C:\\x\\y.lnk"));
        assert!(
            script.contains("Don''t Starve''"),
            "quotes are doubled: {script}"
        );
        // Every quote in the script is either a delimiter or half of a doubled pair.
        assert_eq!(script.matches('\'').count() % 2, 0, "balanced: {script}");
    }

    #[test]
    fn shortcuts_round_trip_through_a_home_directory() {
        let home = std::env::temp_dir().join(format!("gameyfin-sc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        // Menu only: the desktop resolves from the real environment and a test could write there.
        assert!(!installed_for(&home, 12).contains(&Location::Menu));

        let path = create(&home, Location::Menu, &a_local_target()).unwrap();
        assert!(path.exists());
        assert!(installed_for(&home, 12).contains(&Location::Menu));
        // A different game is not confused for this one.
        assert!(!installed_for(&home, 13).contains(&Location::Menu));

        assert!(remove(&home, Location::Menu, 12));
        assert!(!installed_for(&home, 12).contains(&Location::Menu));
        assert!(!remove(&home, Location::Menu, 12));

        std::fs::remove_dir_all(&home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_desktop_entry_is_written_executable() {
        // Without the bit, double-clicking opens a text editor rather than the game.
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("gameyfin-scx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        let path = create(&home, Location::Menu, &a_target()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "executable for everyone who can read it"
        );

        std::fs::remove_dir_all(&home).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn a_localised_desktop_folder_is_honoured() {
        // Writing to a hard-coded ~/Desktop on a German system creates a folder the
        // desktop does not show, so the shortcut silently goes nowhere visible.
        let home = Path::new("/home/ana");
        let contents = "# generated\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DESKTOP_DIR=\"$HOME/Skrivebord\"\n";
        assert_eq!(
            parse_user_dir(contents, "XDG_DESKTOP_DIR", home),
            Some(PathBuf::from("/home/ana/Skrivebord"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn an_absolute_or_disabled_desktop_dir_is_understood() {
        let home = Path::new("/home/ana");
        assert_eq!(
            parse_user_dir("XDG_DESKTOP_DIR=\"/mnt/desk\"\n", "XDG_DESKTOP_DIR", home),
            Some(PathBuf::from("/mnt/desk"))
        );
        // `$HOME` on its own is how the spec says to turn the folder off.
        assert_eq!(
            parse_user_dir("XDG_DESKTOP_DIR=\"$HOME\"\n", "XDG_DESKTOP_DIR", home),
            Some(PathBuf::from("/home/ana"))
        );
        // A commented-out or absent key falls through to the caller's default.
        assert_eq!(
            parse_user_dir("#XDG_DESKTOP_DIR=\"$HOME/x\"\n", "XDG_DESKTOP_DIR", home),
            None
        );
    }

    #[test]
    fn a_renamed_game_is_still_recognised() {
        // The id in the name is what makes this work; matching on the title would not.
        let home = std::env::temp_dir().join(format!("gameyfin-scr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();

        let mut target = a_local_target();
        create(&home, Location::Menu, &target).unwrap();

        target.title = "Celeste: Farewell Edition".to_string();
        assert!(
            installed_for(&home, target.game_id).contains(&Location::Menu),
            "found by id even though the title changed"
        );

        std::fs::remove_dir_all(&home).unwrap();
    }
}
