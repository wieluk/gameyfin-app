//! Tauri commands. Thin on purpose: logic that can be tested without a GUI lives in the crates.

pub mod install;
pub mod launch;
pub mod library;
pub mod session;
pub mod untracked;

use std::path::{Path, PathBuf};

use gameyfin_core::InstallLayout;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Asks the UI for a full refresh. For structural changes, not progress.
pub fn notify(app: &AppHandle) {
    let _ = app.emit("library-changed", ());
    crate::taskbar::refresh(app);
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GameStateEvent {
    game_id: i64,
    state: crate::library_state::GameState,
}

/// Pushes one game's state, cheaper than a full `list_entries` refresh.
pub fn notify_state(app: &AppHandle, game_id: i64) {
    let state = app.state::<AppState>().library().state_of(game_id);
    let _ = app.emit("game-state", GameStateEvent { game_id, state });
    crate::taskbar::refresh(app);
}

/// The games folder a game lives in, from its recorded paths, else the default.
pub fn root_for_game(state: &AppState, game_id: i64) -> CommandResult<String> {
    let record = state.library().record(game_id);
    let settings = state.settings();
    let known = settings.library_roots();
    record
        .install_dir
        .iter()
        .chain(record.archive_path.iter())
        .find_map(|path| known.iter().find(|root| path.starts_with(root)).cloned())
        .map_or_else(|| settings.require_root(None), Ok)
        .map_err(CommandError::Message)
}

pub fn layout_for(state: &AppState, game_id: i64) -> CommandResult<InstallLayout> {
    Ok(InstallLayout::new(root_for_game(state, game_id)?))
}

/// Where a game installs to by default, named after its title.
pub async fn install_dir_for(state: &AppState, game_id: i64) -> CommandResult<PathBuf> {
    let layout = layout_for(state, game_id)?;
    Ok(layout.install_dir(game_id, &state.title(game_id).await))
}

pub async fn downloads_dir_for(state: &AppState, game_id: i64) -> CommandResult<PathBuf> {
    let layout = layout_for(state, game_id)?;
    Ok(layout.downloads_dir(game_id, &state.title(game_id).await))
}

/// RFC 3339, which the webview's `Date` parses.
pub fn now_iso8601() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// An unreadable directory counts as non-empty, so nothing is deleted on a failed read.
pub async fn dir_is_empty(dir: &Path) -> bool {
    match tokio::fs::read_dir(dir).await {
        Ok(mut entries) => entries.next_entry().await.ok().flatten().is_none(),
        Err(_) => false,
    }
}

pub fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Resolves `relative` under `base`, refusing anything that escapes it, symlinks included.
pub fn contained(base: &Path, relative: &str) -> CommandResult<PathBuf> {
    let refused = || CommandError::msg(format!("{relative} is not inside this game's folder."));
    let candidate = Path::new(relative);
    if candidate.is_absolute() || relative.is_empty() {
        return Err(refused());
    }
    let joined = base.join(candidate);
    match (joined.canonicalize(), base.canonicalize()) {
        (Ok(resolved), Ok(root)) if resolved.starts_with(&root) => Ok(joined),
        (Err(_), _) => Err(CommandError::msg(format!("{relative} could not be found."))),
        _ => Err(refused()),
    }
}

/// Whether a folder may be adopted as, or deleted as, a game's install folder.
pub fn safe_game_folder(state: &AppState, dir: &Path) -> CommandResult<()> {
    let refused = || {
        CommandError::msg(format!(
            "{} cannot be used as a game's folder.",
            dir.display()
        ))
    };
    let resolved = dir.canonicalize().map_err(|_| refused())?;
    if resolved.parent().is_none() {
        return Err(refused());
    }
    let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    let mut protected: Vec<PathBuf> = [crate::integrations::home().ok(), Some(state.config_dir())]
        .into_iter()
        .flatten()
        .map(|p| resolve(&p))
        .collect();
    for root in state.settings().library_roots() {
        let layout = InstallLayout::new(&root);
        protected.extend(
            [root.into(), layout.installs_root(), layout.downloads_root()]
                .map(|p: PathBuf| resolve(&p)),
        );
    }
    // Neither a protected folder nor anything containing one.
    if protected.iter().any(|p| p.starts_with(&resolved)) {
        return Err(refused());
    }
    Ok(())
}

pub fn ensure_windows_program(program: &Path, what: &str) -> CommandResult<()> {
    if gameyfin_core::needs_proton(program) && !gameyfin_core::looks_like_windows_program(program) {
        tracing::error!(?program, "not a Windows program");
        return Err(CommandError::msg(format!(
            "{} is not a Windows program. The {what} may be incomplete or corrupt.",
            file_label(program)
        )));
    }
    Ok(())
}

/// Which of a games folder's two subfolders.
#[derive(Debug, Clone, Copy, serde::Deserialize, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum LibraryFolder {
    Downloads,
    Installations,
}

impl LibraryFolder {
    pub fn dir(self, layout: &InstallLayout) -> PathBuf {
        match self {
            LibraryFolder::Downloads => layout.downloads_root(),
            LibraryFolder::Installations => layout.installs_root(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contained_refuses_escapes() {
        let base = std::env::temp_dir().join(format!("gameyfin-contained-{}", std::process::id()));
        std::fs::create_dir_all(base.join("bin")).unwrap();
        std::fs::write(base.join("bin/game.exe"), b"x").unwrap();

        assert!(contained(&base, "bin/game.exe").is_ok());
        assert!(contained(&base, "../").is_err());
        assert!(contained(&base, "bin/../../etc").is_err());
        assert!(contained(&base, "/etc/passwd").is_err());
        assert!(contained(&base, "").is_err());
        assert!(contained(&base, "missing.exe").is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn protected_folders_cannot_be_adopted() {
        let state = AppState::default();
        let root = std::env::temp_dir().join(format!("gameyfin-safe-{}", std::process::id()));
        let game = InstallLayout::new(&root).install_dir(1, "Game");
        std::fs::create_dir_all(&game).unwrap();
        state.set_config_dir(root.join("config"));
        crate::state::write_settings_for_test(&state, |s| {
            s.library_root = Some(root.display().to_string())
        });

        assert!(safe_game_folder(&state, &game).is_ok());
        assert!(safe_game_folder(&state, &root).is_err());
        assert!(safe_game_folder(&state, &InstallLayout::new(&root).installs_root()).is_err());
        assert!(safe_game_folder(&state, root.parent().unwrap()).is_err());
        assert!(safe_game_folder(&state, Path::new("/")).is_err());
        if let Ok(home) = crate::integrations::home() {
            assert!(safe_game_folder(&state, &home).is_err());
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
