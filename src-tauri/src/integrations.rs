//! Where Gameyfin reaches outside its own window: desktop and menu entries, Steam
//! shortcuts, per-game prefixes, the umu fix database. Each touches something another
//! program owns, so each reports what it did.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::error::{CommandError, CommandResult, Context};
use crate::state::AppState;

/// Everything here writes relative to it, so it fails loudly rather than scattering files.
pub fn home() -> CommandResult<PathBuf> {
    #[cfg(windows)]
    let candidate = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let candidate = std::env::var_os("HOME");

    candidate
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| CommandError::msg("could not find your home directory"))
}

/// Inside a Flatpak the binary path means nothing on the host, and an AppImage's binary
/// lives in a mount that disappears, so a shortcut has to run the image file itself.
fn launcher_for(
    flatpak_id: Option<String>,
    appimage: Option<PathBuf>,
    exe: PathBuf,
) -> (PathBuf, Vec<String>) {
    if let Some(app_id) = flatpak_id.filter(|id| !id.is_empty()) {
        return (
            PathBuf::from("/usr/bin/flatpak"),
            vec!["run".to_string(), app_id],
        );
    }
    if let Some(image) = appimage.filter(|path| !path.as_os_str().is_empty()) {
        return (image, Vec::new());
    }
    (exe, Vec::new())
}

fn launcher_invocation() -> CommandResult<(PathBuf, Vec<String>)> {
    let exe = std::env::current_exe().context("could not find the Gameyfin executable")?;
    Ok(launcher_for(
        std::env::var("FLATPAK_ID").ok(),
        std::env::var_os("APPIMAGE").map(PathBuf::from),
        exe,
    ))
}

/// Which shortcuts exist for a game, so the UI can show them as toggles.
#[derive(Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShortcutStatus {
    pub desktop: bool,
    pub menu: bool,
    /// Whether Steam is installed at all, which decides if that option is offered.
    pub steam_available: bool,
    pub steam: bool,
}

#[tauri::command]
pub async fn shortcut_status(
    state: State<'_, AppState>,
    game_id: i64,
) -> CommandResult<ShortcutStatus> {
    use gameyfin_core::shortcuts::Location;

    let home = home()?;
    let present = gameyfin_core::shortcuts::installed_for(&home, game_id);
    let title = state.title(game_id).await;
    let steam_available = gameyfin_core::steam::is_installed(&home);
    Ok(ShortcutStatus {
        desktop: present.contains(&Location::Desktop),
        menu: present.contains(&Location::Menu),
        steam_available,
        steam: steam_available && steam_entry_for(&home, game_id, &title).is_some(),
    })
}

/// Kept beside the image cache but apart from it, since that one prunes.
const ICON_DIR: &str = "shortcut-icons";

/// Written as a file rather than named: every package installs it under a different name.
const APP_ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");

#[tauri::command]
pub async fn set_shortcut(
    state: State<'_, AppState>,
    game_id: i64,
    location: gameyfin_core::shortcuts::Location,
    enabled: bool,
) -> CommandResult<()> {
    let home = home()?;
    let title = state.game(game_id).await?.title;
    let icon_path = gameyfin_core::icon::path_for(&state.config_dir().join(ICON_DIR), game_id);

    if !enabled {
        gameyfin_core::shortcuts::remove(&home, location, game_id);
        // Desktop and menu share one icon, so it goes with the last of them.
        if gameyfin_core::shortcuts::installed_for(&home, game_id).is_empty() {
            let _ = std::fs::remove_file(&icon_path);
        }
        tracing::info!(game_id, ?location, "removed a shortcut");
        return Ok(());
    }

    let (launcher, launcher_args) = launcher_invocation()?;
    let target = gameyfin_core::shortcuts::Target {
        game_id,
        title,
        launcher,
        launcher_args,
        icon: write_shortcut_icon(&state, game_id, icon_path).await,
    };
    let path = gameyfin_core::shortcuts::create(&home, location, &target)
        .context("could not create the shortcut")?;
    tracing::info!(game_id, ?location, ?path, "created a shortcut");
    Ok(())
}

/// The icon the executable carries, else the game's cover, else Gameyfin's own.
async fn write_shortcut_icon(state: &AppState, game_id: i64, path: PathBuf) -> Option<PathBuf> {
    let record = state.library().record(game_id);
    let executable = record
        .install_dir
        .zip(record.executable)
        .map(|(dir, executable)| dir.join(executable));

    // Reading an executable is disk work; decoding and scaling are CPU work.
    let from_executable = match executable {
        Some(executable) => {
            tokio::task::spawn_blocking(move || gameyfin_core::icon::from_executable(&executable))
                .await
                .ok()
                .flatten()
        }
        None => None,
    };
    let (image, source) = match from_executable {
        Some(image) => (image, "executable"),
        None => {
            // Usually already cached, since the library grid shows it.
            let cover = match state.game(game_id).await.ok().and_then(|game| game.cover) {
                Some(image) => crate::images::artwork(state, &image.path()).await.ok(),
                None => None,
            };
            tokio::task::spawn_blocking(move || {
                cover
                    .and_then(|(bytes, _)| gameyfin_core::icon::from_artwork(&bytes))
                    .map(|image| (image, "cover"))
                    .or_else(|| {
                        gameyfin_core::icon::from_artwork(APP_ICON).map(|image| (image, "app"))
                    })
            })
            .await
            .ok()
            .flatten()?
        }
    };

    let written = path.clone();
    match tokio::task::spawn_blocking(move || gameyfin_core::icon::write(&image, &written)).await {
        Ok(Ok(())) => {
            tracing::info!(game_id, source, ?path, "wrote a shortcut icon");
            Some(path)
        }
        Ok(Err(e)) => {
            tracing::warn!(game_id, ?path, error = %e, "could not write a shortcut icon");
            None
        }
        Err(_) => None,
    }
}

/// Steam stores the program quoted and runs it with the arguments that follow.
fn steam_shortcut_for(game_id: i64, title: &str) -> CommandResult<gameyfin_core::steam::Shortcut> {
    let (launcher, launcher_args) = launcher_invocation()?;
    Ok(gameyfin_core::steam::Shortcut {
        app_name: title.to_string(),
        exe: format!("\"{}\"", launcher.display()),
        start_dir: launcher
            .parent()
            .map(|p| format!("\"{}\"", p.display()))
            .unwrap_or_default(),
        icon: String::new(),
        launch_options: launcher_args
            .into_iter()
            .chain(["--launch".to_string(), game_id.to_string()])
            .collect::<Vec<_>>()
            .join(" "),
        tags: vec![gameyfin_core::steam::OWNER_TAG.to_string()],
    })
}

fn steam_entry_for(home: &Path, game_id: i64, title: &str) -> Option<u32> {
    let wanted = steam_shortcut_for(game_id, title).ok()?.app_id();
    gameyfin_core::steam::shortcut_files(home)
        .into_iter()
        .any(|path| {
            let document = gameyfin_core::steam::read_document(&path);
            let Some(gameyfin_core::steam::Value::Map(list)) = document.get("shortcuts") else {
                return false;
            };
            list.iter().any(|(_, entry)| {
                matches!(entry.get("appid"), Some(gameyfin_core::steam::Value::Int(id)) if *id == wanted as i32)
            })
        })
        .then_some(wanted)
}

/// Written to every signed-in account, since which one is at the keyboard is unknowable.
#[tauri::command]
pub async fn set_steam_shortcut(
    state: State<'_, AppState>,
    game_id: i64,
    enabled: bool,
) -> CommandResult<String> {
    let home = home()?;
    let title = state.game(game_id).await?.title;
    let shortcut = steam_shortcut_for(game_id, &title)?;

    let files = gameyfin_core::steam::shortcut_files(&home);
    if files.is_empty() {
        return Err(CommandError::msg(
            "No Steam account was found on this machine. Sign in to Steam once, then try again.",
        ));
    }
    let mut written = 0usize;
    let mut failures = Vec::new();
    for path in &files {
        let mut document = gameyfin_core::steam::read_document(path);
        if enabled {
            gameyfin_core::steam::upsert(&mut document, &shortcut);
        } else if !gameyfin_core::steam::remove(&mut document, shortcut.app_id()) {
            continue;
        }
        match gameyfin_core::steam::write_document(path, &document) {
            Ok(()) => written += 1,
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    if written == 0 && !failures.is_empty() {
        return Err(CommandError::msg(format!(
            "could not write to Steam's shortcuts file. {}",
            failures.join("; ")
        )));
    }
    tracing::info!(
        game_id,
        enabled,
        accounts = written,
        "updated Steam shortcuts"
    );
    Ok(if enabled {
        "Added to Steam. Restart Steam to see it in your library.".to_string()
    } else {
        "Removed from Steam. Restart Steam for the change to show.".to_string()
    })
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrefixEntry {
    pub game_id: i64,
    pub title: Option<String>,
    pub path: String,
    pub bytes: u64,
}

/// Listed from the folders of every games folder, so a prefix left by a removed game shows.
#[tauri::command]
pub async fn list_prefixes(state: State<'_, AppState>) -> CommandResult<Vec<PrefixEntry>> {
    if cfg!(windows) {
        return Ok(Vec::new());
    }
    let roots = state.settings().library_roots();
    let titles: std::collections::HashMap<i64, String> = state
        .games()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|g| (g.id, g.title))
        .collect();

    let mut entries = tokio::task::spawn_blocking(move || {
        roots
            .iter()
            .map(|root| gameyfin_core::InstallLayout::new(root).prefixes_root())
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flat_map(|read| read.flatten())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let path = e.path();
                // The directory is named by game id alone.
                let game_id = path.file_name()?.to_str()?.parse::<i64>().ok()?;
                let bytes = gameyfin_core::extract::directory_size(&path).unwrap_or(0);
                Some((game_id, path, bytes))
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(game_id, path, bytes)| PrefixEntry {
        game_id,
        title: titles.get(&game_id).cloned(),
        path: path.to_string_lossy().into_owned(),
        bytes,
    })
    .collect::<Vec<_>>();

    // Largest first: the reason to look at this list is almost always disk space.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.bytes));
    Ok(entries)
}

/// Rebuilt on the next launch, but anything the game wrote inside goes too; the UI warns.
#[tauri::command]
pub async fn delete_prefix(state: State<'_, AppState>, game_id: i64) -> CommandResult<()> {
    let dir = crate::ipc::layout_for(&state, game_id)?.prefix_dir(game_id);
    if !dir.is_dir() {
        return Ok(());
    }
    tokio::fs::remove_dir_all(&dir)
        .await
        .context(format!("could not delete {}", dir.display()))?;
    tracing::info!(game_id, ?dir, "deleted a compatibility prefix");
    Ok(())
}

/// A Wine tool that can be run against one prefix.
#[derive(Debug, Clone, Copy, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum PrefixTool {
    Winecfg,
    /// The prefix's C: drive, in a file manager.
    Explorer,
    Regedit,
}

impl PrefixTool {
    fn program(self) -> &'static str {
        match self {
            PrefixTool::Winecfg => "winecfg",
            PrefixTool::Explorer => "explorer",
            PrefixTool::Regedit => "regedit",
        }
    }
}

/// Most advice for a misbehaving Windows game is a winecfg or registry change, so a broken
/// prefix should be fixable rather than only deletable.
#[tauri::command]
pub async fn open_prefix_tool(
    state: State<'_, AppState>,
    game_id: i64,
    tool: PrefixTool,
) -> CommandResult<()> {
    let prefix = crate::ipc::layout_for(&state, game_id)?.prefix_dir(game_id);
    if !prefix.is_dir() {
        return Err(CommandError::msg(
            "This game has no compatibility prefix yet. Run it once first.",
        ));
    }
    // The game's own runtime, so a tool opens the prefix the way the game will use it.
    let runtime = crate::proton::runtime_for_game(&state, game_id, None).await?;
    let program = tool.program();
    let mut command = gameyfin_core::prefix::tool_command(&runtime, &prefix, program);
    command
        .env
        .extend(crate::proton::container_mounts(&state, &runtime));

    tracing::info!(game_id, program, ?prefix, "opening a Wine tool");
    let mut spawn = tokio::process::Command::new(&command.program);
    spawn.args(&command.args);
    gameyfin_core::process::clean_for_host(&mut spawn);
    spawn
        .envs(&command.env)
        .spawn()
        .context(format!("could not start {program}"))?;
    Ok(())
}

/// What the app knows about per-title Proton fixes.
#[derive(Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UmuStatus {
    /// Zero means the database has never been fetched.
    pub entries: usize,
    pub age_seconds: Option<u64>,
    pub enabled: bool,
    /// The id a given game would get, when one was asked about.
    pub resolved: Option<String>,
}

#[tauri::command]
pub async fn umu_status(
    state: State<'_, AppState>,
    game_id: Option<i64>,
) -> CommandResult<UmuStatus> {
    // Resolved through the state, so this is exactly what a launch would use.
    let resolved = match game_id {
        Some(id) => Some(state.umu_id_for_game(id).await),
        None => None,
    };
    Ok(UmuStatus {
        entries: state.umu_entry_count(),
        age_seconds: gameyfin_core::umu::cache_age(&state.config_dir()).map(|age| age.as_secs()),
        enabled: state.settings().umu_fixes,
        resolved,
    })
}

#[tauri::command]
pub async fn refresh_umu_database(state: State<'_, AppState>) -> CommandResult<usize> {
    state
        .refresh_umu_database()
        .await
        .context("could not refresh the umu database")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortcut_runs_the_image_or_the_flatpak_rather_than_a_mount() {
        let image = PathBuf::from("/home/me/Applications/Gameyfin.AppImage");
        let mounted = PathBuf::from("/tmp/.mount_GameyfXYZ/usr/bin/gameyfin-app");
        assert_eq!(
            launcher_for(None, Some(image.clone()), mounted),
            (image, Vec::new())
        );
        assert_eq!(
            launcher_for(
                Some("org.gameyfin.gameyfin-app".into()),
                None,
                "/app/bin/gameyfin-app".into()
            ),
            (
                PathBuf::from("/usr/bin/flatpak"),
                vec!["run".to_string(), "org.gameyfin.gameyfin-app".to_string()]
            )
        );
        // A deb or rpm install, or an empty variable left behind by a launcher.
        let installed = PathBuf::from("/usr/bin/gameyfin-app");
        assert_eq!(
            launcher_for(None, None, installed.clone()),
            (installed.clone(), Vec::new())
        );
        assert_eq!(
            launcher_for(Some(String::new()), Some(PathBuf::new()), installed.clone()),
            (installed, Vec::new())
        );
    }
}
