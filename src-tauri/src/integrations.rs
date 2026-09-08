//! Commands for the places Gameyfin reaches outside its own window: Steam shortcuts,
//! desktop and menu entries, per-game compatibility prefixes, the umu fix database. Each
//! touches something another program also owns, so each is careful and reports what it did.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// The user's home directory.
///
/// Everything here writes relative to it, and getting it wrong means scattering files in
/// someone else's profile, so it is resolved once and fails loudly.
fn home() -> CommandResult<PathBuf> {
    #[cfg(windows)]
    let candidate = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let candidate = std::env::var_os("HOME");

    candidate
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| CommandError::Message("could not find your home directory".into()))
}

/// How to invoke Gameyfin from outside it: the program, and any arguments it needs first.
///
/// Inside a Flatpak the binary's own path is meaningless on the host, which is where a
/// desktop entry or Steam will run it from, so the invocation has to go through
/// `flatpak run <app-id>`. Both shortcut kinds share this, because getting them out of
/// step would mean one of them silently launching nothing.
fn launcher_invocation() -> CommandResult<(PathBuf, Vec<String>)> {
    if let Ok(app_id) = std::env::var("FLATPAK_ID") {
        return Ok((
            PathBuf::from("/usr/bin/flatpak"),
            vec!["run".to_string(), app_id],
        ));
    }
    let exe = std::env::current_exe().map_err(|e| {
        CommandError::Message(format!("could not find the Gameyfin executable: {e}"))
    })?;
    Ok((exe, Vec::new()))
}

/// A game's title, from the catalogue.
async fn title_of(state: &State<'_, AppState>, game_id: i64) -> CommandResult<String> {
    state
        .games()
        .await?
        .into_iter()
        .find(|g| g.id == game_id)
        .map(|g| g.title)
        .ok_or_else(|| CommandError::Message("That game is no longer in the library.".into()))
}

// --- Desktop and menu shortcuts ------------------------------------------------------

/// Which shortcuts exist for a game, so the UI can show them as toggles.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
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
    let title = title_of(&state, game_id).await.unwrap_or_default();

    // Reading Steam's file is cheap, but only worth doing when Steam exists.
    let steam_available = gameyfin_core::steam::is_installed(&home);
    let steam = steam_available && steam_entry_for(&home, game_id, &title).is_some();

    Ok(ShortcutStatus {
        desktop: present.contains(&Location::Desktop),
        menu: present.contains(&Location::Menu),
        steam_available,
        steam,
    })
}

/// Create or remove a game's desktop or menu shortcut.
#[tauri::command]
pub async fn set_shortcut(
    state: State<'_, AppState>,
    game_id: i64,
    location: gameyfin_core::shortcuts::Location,
    enabled: bool,
) -> CommandResult<()> {
    let home = home()?;
    let title = title_of(&state, game_id).await?;

    if !enabled {
        gameyfin_core::shortcuts::remove(&home, location, game_id, &title);
        tracing::info!(game_id, ?location, "removed a shortcut");
        return Ok(());
    }

    let (launcher, launcher_args) = launcher_invocation()?;
    let target = gameyfin_core::shortcuts::Target {
        game_id,
        title: title.clone(),
        launcher,
        launcher_args,
        icon: None,
    };
    let path = gameyfin_core::shortcuts::create(&home, location, &target)
        .map_err(|e| CommandError::Message(format!("could not create the shortcut: {e}")))?;
    tracing::info!(game_id, ?location, ?path, "created a shortcut");
    Ok(())
}

// --- Steam ---------------------------------------------------------------------------

/// Build the shortcut this app would write for a game.
fn steam_shortcut_for(game_id: i64, title: &str) -> CommandResult<gameyfin_core::steam::Shortcut> {
    let (launcher, launcher_args) = launcher_invocation()?;
    // Steam stores the program quoted, and runs it with the arguments that follow.
    let exe = format!("\"{}\"", launcher.display());

    let launch_options = launcher_args
        .iter()
        .cloned()
        .chain(["--launch".to_string(), game_id.to_string()])
        .collect::<Vec<_>>()
        .join(" ");

    let start_dir = launcher
        .parent()
        .map(|p| format!("\"{}\"", p.display()))
        .unwrap_or_default();

    Ok(gameyfin_core::steam::Shortcut {
        app_name: title.to_string(),
        exe,
        start_dir,
        icon: String::new(),
        launch_options,
        tags: vec![gameyfin_core::steam::OWNER_TAG.to_string()],
    })
}

/// Whether this game is already in one of the user's Steam accounts.
fn steam_entry_for(home: &std::path::Path, game_id: i64, title: &str) -> Option<u32> {
    let shortcut = steam_shortcut_for(game_id, title).ok()?;
    let wanted = shortcut.app_id();
    gameyfin_core::steam::shortcut_files(home)
        .into_iter()
        .any(|path| {
            let document = gameyfin_core::steam::read_document(&path);
            let Some(gameyfin_core::steam::Value::Map(list)) = document.get("shortcuts") else {
                return false;
            };
            list.iter().any(|(_, entry)| {
                matches!(
                    entry.get("appid"),
                    Some(gameyfin_core::steam::Value::Int(id)) if *id == wanted as i32
                )
            })
        })
        .then_some(wanted)
}

/// Add or remove a game in the user's Steam library.
///
/// Written to every signed-in account on the machine, because there is no way to tell from
/// here which one belongs to the person at the keyboard, and adding it to one at random
/// would look like the button had failed.
#[tauri::command]
pub async fn set_steam_shortcut(
    state: State<'_, AppState>,
    game_id: i64,
    enabled: bool,
) -> CommandResult<String> {
    let home = home()?;
    let title = title_of(&state, game_id).await?;
    let shortcut = steam_shortcut_for(game_id, &title)?;

    let files = gameyfin_core::steam::shortcut_files(&home);
    if files.is_empty() {
        return Err(CommandError::Message(
            "No Steam account was found on this machine. Sign in to Steam once, then try again."
                .into(),
        ));
    }

    let mut written = 0usize;
    let mut failures = Vec::new();
    for path in &files {
        let mut document = gameyfin_core::steam::read_document(path);
        if enabled {
            gameyfin_core::steam::upsert(&mut document, &shortcut);
        } else if !gameyfin_core::steam::remove(&mut document, shortcut.app_id()) {
            // Nothing to remove in this account, which is not a failure.
            continue;
        }

        match gameyfin_core::steam::write_document(path, &document) {
            Ok(()) => written += 1,
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }

    if written == 0 && !failures.is_empty() {
        return Err(CommandError::Message(format!(
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

// --- Compatibility prefixes ----------------------------------------------------------

/// One game's Wine prefix.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefixEntry {
    pub game_id: i64,
    /// The game's title, when it is still in the catalogue.
    pub title: Option<String>,
    pub path: String,
    pub bytes: u64,
}

/// Every prefix on disk, largest first.
///
/// Listed from the folder rather than from the library records, so a prefix left behind by
/// a game that has since been removed from the server still shows up. Those are exactly
/// the ones worth reclaiming.
#[tauri::command]
pub async fn list_prefixes(state: State<'_, AppState>) -> CommandResult<Vec<PrefixEntry>> {
    if cfg!(windows) {
        return Ok(Vec::new());
    }
    let Some(root) = state.settings().await.library_root else {
        return Ok(Vec::new());
    };
    let dir = gameyfin_core::InstallLayout::new(&root).prefixes_root();

    let titles: std::collections::HashMap<i64, String> = state
        .games()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|g| (g.id, g.title))
        .collect();

    let mut entries = tokio::task::spawn_blocking(move || {
        let Ok(read) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        read.flatten()
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

/// Delete one game's prefix.
///
/// Safe in the sense that it is rebuilt on the next launch, but not free: anything the
/// game wrote inside the prefix, including saves that do not live in the install folder,
/// goes with it. The UI says so before calling this.
#[tauri::command]
pub async fn delete_prefix(state: State<'_, AppState>, game_id: i64) -> CommandResult<()> {
    let root = state
        .settings()
        .await
        .library_root
        .ok_or_else(|| CommandError::Message("No games folder configured yet.".into()))?;
    let dir = gameyfin_core::InstallLayout::new(&root).prefix_dir(game_id);

    if !dir.is_dir() {
        return Ok(());
    }
    tokio::fs::remove_dir_all(&dir)
        .await
        .map_err(|e| CommandError::Message(format!("could not delete {}: {e}", dir.display())))?;
    tracing::info!(game_id, ?dir, "deleted a compatibility prefix");
    Ok(())
}

/// A Wine tool that can be run against one prefix.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrefixTool {
    /// Wine's own configuration dialog.
    Winecfg,
    /// The prefix's C: drive, in a file manager.
    Explorer,
    /// A registry editor, for the fixes forum posts ask for.
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

/// Open a Wine tool against one game's prefix.
///
/// This is what makes a broken prefix fixable rather than only deletable: most of the
/// advice that exists for a misbehaving Windows game is a change in `winecfg` or a key in
/// the registry.
#[tauri::command]
pub async fn open_prefix_tool(
    app: AppHandle,
    state: State<'_, AppState>,
    game_id: i64,
    tool: PrefixTool,
) -> CommandResult<()> {
    let _ = app;
    let settings = state.settings().await;
    let root = settings
        .library_root
        .clone()
        .ok_or_else(|| CommandError::Message("No games folder configured yet.".into()))?;
    let prefix = gameyfin_core::InstallLayout::new(&root).prefix_dir(game_id);

    if !prefix.is_dir() {
        return Err(CommandError::Message(
            "This game has no compatibility prefix yet. Run it once first.".into(),
        ));
    }

    let config_dir = state.config_dir().await;
    let runtime = tokio::task::spawn_blocking(move || {
        gameyfin_core::detect_windows_runtime_in(Some(&config_dir))
    })
    .await
    .map_err(|e| CommandError::Message(format!("could not look for Wine: {e}")))?
    .ok_or_else(|| {
        CommandError::Message(
            "No Wine was found. Download it from Settings, under Compatibility.".into(),
        )
    })?;

    let program = tool.program();
    let command = gameyfin_core::prefix::tool_command(&runtime, &prefix, program);

    tracing::info!(game_id, program, ?prefix, "opening a Wine tool");
    tokio::process::Command::new(&command.program)
        .args(&command.args)
        .envs(&command.env)
        .spawn()
        .map_err(|e| CommandError::Message(format!("could not start {program}: {e}")))?;
    Ok(())
}

// --- The umu fix database ------------------------------------------------------------

/// What the app knows about per-title Proton fixes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UmuStatus {
    /// How many rows the cached database holds. Zero means it has never been fetched.
    pub entries: usize,
    pub enabled: bool,
    /// The id that would be used for a given game, when one was asked about.
    pub resolved: Option<String>,
}

#[tauri::command]
pub async fn umu_status(
    state: State<'_, AppState>,
    game_id: Option<i64>,
) -> CommandResult<UmuStatus> {
    let settings = state.settings().await;

    // Resolved through the state rather than against a cloned database, so what is shown
    // here is exactly what a launch would use, the `umuFixes` setting included.
    let resolved = match game_id {
        Some(id) => {
            let game = state
                .games()
                .await
                .unwrap_or_default()
                .into_iter()
                .find(|g| g.id == id);
            match game {
                Some(game) => Some(state.umu_id_for(&game.title, game.steam_app_id()).await),
                None => None,
            }
        }
        None => None,
    };

    Ok(UmuStatus {
        entries: state.umu_entry_count().await,
        enabled: settings.umu_fixes,
        resolved,
    })
}

/// Fetch the umu database now, rather than waiting for the next startup.
#[tauri::command]
pub async fn refresh_umu_database(state: State<'_, AppState>) -> CommandResult<usize> {
    let count = state
        .refresh_umu_database()
        .await
        .map_err(|e| CommandError::Message(e.to_string()))?;
    Ok(count)
}
