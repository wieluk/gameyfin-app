//! Desktop and application-menu shortcuts for installed games. Linux gets freedesktop
//! `.desktop` entries; Windows a `.url`-style shim, since a real `.lnk` needs COM.
//!
//! Every shortcut runs **the app** with `--launch <id>`, not the `.exe`, so the prefix,
//! runtime choice and playtime supervision still apply.

use std::path::{Path, PathBuf};

/// Where a shortcut should be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Location {
    /// The user's desktop.
    Desktop,
    /// The application menu, or the Start menu on Windows.
    Menu,
}

/// What a shortcut needs to know to start a game.
#[derive(Debug, Clone)]
pub struct Target {
    pub game_id: i64,
    pub title: String,
    /// The Gameyfin executable the shortcut runs.
    pub launcher: PathBuf,
    /// Arguments that come before `--launch`.
    ///
    /// Empty for an ordinary install, where the launcher is the app itself. Inside a
    /// Flatpak the launcher is `flatpak`, which needs `run <app-id>` first: without this
    /// the shortcut would read `flatpak --launch 12`, which is not a command flatpak has.
    pub launcher_args: Vec<String>,
    /// Absolute path to an icon file, when one has been cached for this game.
    pub icon: Option<PathBuf>,
}

/// The file name a game's shortcut gets, without an extension.
///
/// The id is in the name so two games with the same title do not collide, and so a
/// shortcut can be found again to remove it after the title changes on the server.
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
    } else {
        format!("gameyfin-{game_id} {trimmed}")
    }
}

/// Build the contents of a freedesktop desktop entry.
///
/// `Exec` values are unquoted words with `%`-escapes, so a path containing a space or a
/// literal percent has to be quoted and escaped rather than pasted in.
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
        // The app's own icon, installed by every Linux package under this name.
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

/// Escape a value for the right-hand side of a desktop-entry key.
///
/// The spec gives backslash, newline, tab and carriage return meaning inside values, so a
/// Windows-style path or a title containing a newline has to be escaped or the entry is
/// silently mis-parsed.
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

/// Quote one argument for an `Exec` line.
///
/// Inside a quoted `Exec` argument the reserved characters are backslash, double quote,
/// backtick and dollar; `%` is doubled because a single one introduces a field code.
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

/// The PowerShell that creates a Windows `.lnk`.
///
/// A `.lnk` rather than a `.url`, because an internet shortcut cannot carry arguments and
/// the whole design depends on passing `--launch <id>`. Real `.lnk` files are a COM
/// structure, so rather than marshalling `IShellLink` by hand this drives the shell's own
/// `WScript.Shell`, which is present on every supported Windows and produces exactly the
/// file Explorer would.
///
/// Returned as a script rather than run here so its quoting can be tested.
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

/// Quote a value as a PowerShell single-quoted string.
///
/// Single quotes rather than double, because PowerShell does no expansion inside them: a
/// game called `$(Get-Process)` is then a title rather than a command. The only character
/// with meaning is the quote itself, escaped by doubling.
fn powershell_string(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The file extension shortcuts use on this platform.
pub fn extension() -> &'static str {
    if cfg!(windows) {
        "lnk"
    } else {
        "desktop"
    }
}

/// Resolve where shortcuts of a given kind belong, given the user's home directory.
///
/// Returns `None` when the platform has no such location, rather than inventing one:
/// scattering files into a guessed directory is worse than not making the shortcut.
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

/// The user's desktop directory.
///
/// Not simply `~/Desktop`: the folder is localised, so on a German or Norwegian system it
/// is `Schreibtisch` or `Skrivebord`, and writing to the English name creates a second
/// folder the desktop does not display. `XDG_DESKTOP_DIR` in the environment wins, then
/// the value recorded in `user-dirs.dirs`, then the English default.
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

/// Pull one directory out of a `user-dirs.dirs` file.
///
/// The format is shell-style assignments, and the value is conventionally written
/// relative to `$HOME`, which has to be expanded rather than taken literally.
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

/// Write a shortcut for one game, returning where it went.
pub fn create(home: &Path, location: Location, target: &Target) -> std::io::Result<PathBuf> {
    let directory = directory_for(home, location).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "there is no shortcut folder for this location on this system",
        )
    })?;
    std::fs::create_dir_all(&directory)?;

    let path = directory.join(format!(
        "{}.{}",
        stem(target.game_id, &target.title),
        extension()
    ));

    #[cfg(windows)]
    {
        let script = windows_shortcut_script(target, &path);
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
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

    // A desktop entry the user double-clicks has to be executable, and since GNOME 42 it
    // must also be marked trusted; without the executable bit the file opens in a text
    // editor instead of starting the game.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)?;
    }

    Ok(path)
}

/// Remove a game's shortcut from one location. True when there was one.
pub fn remove(home: &Path, location: Location, game_id: i64, title: &str) -> bool {
    let Some(directory) = directory_for(home, location) else {
        return false;
    };
    let path = directory.join(format!("{}.{}", stem(game_id, title), extension()));
    std::fs::remove_file(path).is_ok()
}

/// Which locations already hold a shortcut for this game.
///
/// Matched by the id in the file name, so a game renamed on the server is still
/// recognised as having one.
pub fn installed_for(home: &Path, game_id: i64) -> Vec<Location> {
    let prefix = format!("gameyfin-{game_id} ");
    let exact = format!("gameyfin-{game_id}.");

    [Location::Desktop, Location::Menu]
        .into_iter()
        .filter(|&location| {
            let Some(directory) = directory_for(home, location) else {
                return false;
            };
            let Ok(entries) = std::fs::read_dir(&directory) else {
                return false;
            };
            entries.flatten().any(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.ends_with(extension())
                    && (name.starts_with(&prefix) || name.starts_with(&exact))
            })
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

    #[test]
    fn the_stem_carries_the_id_so_two_titles_cannot_collide() {
        assert_eq!(stem(12, "Celeste"), "gameyfin-12 Celeste");
        assert_ne!(stem(12, "Celeste"), stem(13, "Celeste"));
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
        target.launcher_args = vec!["run".into(), "org.gameyfin.Gameyfin".into()];

        let entry = desktop_entry(&target);
        assert!(
            entry.contains(
                "Exec=\"/usr/bin/flatpak\" \"run\" \"org.gameyfin.Gameyfin\" --launch 12"
            ),
            "got {entry}"
        );

        let script = windows_shortcut_script(&target, Path::new("C:\\x\\y.lnk"));
        assert!(
            script.contains("$s.Arguments='run org.gameyfin.Gameyfin --launch 12'"),
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

        // Only the menu location is exercised here. The desktop folder is resolved from
        // the real environment, so a test that wrote there could land a file on the
        // machine's actual desktop; `parse_user_dir` covers that resolution instead.
        assert!(!installed_for(&home, 12).contains(&Location::Menu));

        let path = create(&home, Location::Menu, &a_target()).unwrap();
        assert!(path.exists());
        assert!(installed_for(&home, 12).contains(&Location::Menu));
        // A different game is not confused for this one.
        assert!(!installed_for(&home, 13).contains(&Location::Menu));

        assert!(remove(&home, Location::Menu, 12, "Celeste"));
        assert!(!installed_for(&home, 12).contains(&Location::Menu));
        assert!(!remove(&home, Location::Menu, 12, "Celeste"));

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

        let mut target = a_target();
        create(&home, Location::Menu, &target).unwrap();

        target.title = "Celeste: Farewell Edition".to_string();
        assert!(
            installed_for(&home, target.game_id).contains(&Location::Menu),
            "found by id even though the title changed"
        );

        std::fs::remove_dir_all(&home).unwrap();
    }
}
