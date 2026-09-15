//! Folders in a games folder that no game claims: copied in by hand, or left by a game the
//! server no longer has. Listed so they can be given to a game or deleted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gameyfin_core::InstallLayout;
use serde::Serialize;
use tauri::{AppHandle, State};

use super::{notify, LibraryFolder};
use crate::error::{blocking, CommandError, CommandResult, Context};
use crate::library_state::{Activity, EXTRACT_DIR};
use crate::state::AppState;

#[derive(Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UntrackedFolder {
    pub path: String,
    pub name: String,
    pub folder: LibraryFolder,
    pub bytes: u64,
    /// The id in its name, for a game the server no longer has.
    pub game_id: Option<i64>,
}

/// Largest first: disk space is the usual reason to look.
#[tauri::command]
pub async fn list_untracked_folders(
    state: State<'_, AppState>,
) -> CommandResult<Vec<UntrackedFolder>> {
    let roots = state.settings().library_roots();
    // Unknown without the catalogue, so a bad connection calls no game gone.
    let catalog: Option<HashSet<i64>> = state
        .games()
        .await
        .ok()
        .map(|games| games.iter().map(|g| g.id).collect());
    let tracked = state.library().tracked_dirs();
    let mut found = blocking("could not scan the games folders", move || {
        Ok::<_, std::convert::Infallible>(scan(&roots, &tracked, catalog.as_ref()))
    })
    .await?;
    found.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    Ok(found)
}

fn scan(
    roots: &[String],
    tracked: &[PathBuf],
    catalog: Option<&HashSet<i64>>,
) -> Vec<UntrackedFolder> {
    let tracked: Vec<PathBuf> = tracked.iter().map(|t| resolve(t)).collect();
    let mut found = Vec::new();
    for root in roots {
        let layout = InstallLayout::new(root);
        for folder in [LibraryFolder::Installations, LibraryFolder::Downloads] {
            let Ok(entries) = std::fs::read_dir(folder.dir(&layout)) else {
                continue;
            };
            // A linked folder points at something that lives elsewhere, so it is not a stray here.
            let dirs = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path());
            for path in dirs {
                let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                    continue;
                };
                // An install in progress sets files aside under dot names.
                if name.starts_with('.') || tracked.contains(&resolve(&path)) {
                    continue;
                }
                let game_id = InstallLayout::game_id_from_dir(&path);
                // A known id is the rescan's to adopt.
                let known = match (game_id, catalog) {
                    (Some(id), Some(catalog)) => catalog.contains(&id),
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                if known {
                    continue;
                }
                found.push(UntrackedFolder {
                    bytes: gameyfin_core::extract::directory_size(&path).unwrap_or(0),
                    path: path.to_string_lossy().into_owned(),
                    name,
                    folder,
                    game_id,
                });
            }
        }
    }
    found
}

fn resolve(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The resolved folder, which subfolder it is in, and its games folder. Anything but a stray
/// directly inside Installations or Downloads is refused, since it is renamed or deleted.
fn stray(state: &AppState, path: &str) -> CommandResult<(PathBuf, LibraryFolder, String)> {
    let refused = || CommandError::msg(format!("{path} is not a folder Gameyfin can change."));
    let resolved = Path::new(path).canonicalize().map_err(|_| refused())?;
    let parent = resolved.parent().ok_or_else(refused)?;
    for root in state.settings().library_roots() {
        let layout = InstallLayout::new(&root);
        for folder in [LibraryFolder::Installations, LibraryFolder::Downloads] {
            if folder.dir(&layout).canonicalize().ok().as_deref() != Some(parent) {
                continue;
            }
            let tracked = state.library().tracked_dirs();
            if tracked.iter().any(|t| resolve(t) == resolved) {
                return Err(CommandError::msg(
                    "That folder belongs to a game in your library.",
                ));
            }
            return Ok((resolved, folder, root));
        }
    }
    Err(refused())
}

#[tauri::command]
pub async fn delete_untracked_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<()> {
    let (dir, _, _) = stray(&state, &path)?;
    tracing::info!(?dir, "deleting an untracked folder");
    tokio::fs::remove_dir_all(&dir)
        .await
        .context(format!("could not delete {}", dir.display()))?;
    notify(&app);
    Ok(())
}

/// Renames the folder after the game, then adopts it as an install or a download.
#[tauri::command]
pub async fn assign_untracked_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    game_id: i64,
) -> CommandResult<()> {
    let (dir, folder, root) = stray(&state, &path)?;
    let game = state.game(game_id).await?;
    let layout = InstallLayout::new(&root);
    let record = state.library().record(game_id);
    tracing::info!(game_id, ?dir, ?folder, "assigning an untracked folder");

    match folder {
        LibraryFolder::Installations => {
            if record.is_installed() {
                return Err(CommandError::msg(format!(
                    "{} is already installed. Uninstall it first, or delete this folder.",
                    game.title
                )));
            }
            // Claimed before the rename, so a busy game leaves the folder untouched.
            let _claim = state
                .library()
                .claim(game_id, Activity::installing())
                .ok_or_else(|| CommandError::msg("Wait for what this game is doing to finish."))?;
            let target = layout.install_dir(game_id, &game.title);
            rename(&dir, &target).await?;
            super::install::finish_install(&state, game_id, &target).await;
        }
        LibraryFolder::Downloads => {
            if record.existing_archive().is_some() || record.existing_staging().is_some() {
                return Err(CommandError::msg(format!(
                    "{} already has a download. Delete it first.",
                    game.title
                )));
            }
            let target = layout.downloads_dir(game_id, &game.title);
            let source = dir.clone();
            let unpacked =
                blocking("could not read the folder", move || is_unpacked(&source)).await?;
            if unpacked {
                // Loose files are what an unpacked download looks like, so they go where one would.
                tokio::fs::create_dir_all(&target)
                    .await
                    .context(format!("could not create {}", target.display()))?;
                rename(&dir, &target.join(EXTRACT_DIR)).await?;
            } else {
                rename(&dir, &target).await?;
            }
            state.library().rescan(Path::new(&root)).await;
        }
    }
    notify(&app);
    Ok(())
}

async fn rename(from: &Path, to: &Path) -> CommandResult<()> {
    if to.exists() {
        return Err(CommandError::msg(format!(
            "{} already exists. Move or delete it first.",
            to.display()
        )));
    }
    tokio::fs::rename(from, to)
        .await
        .context("could not rename the folder")
}

/// A single file is a download waiting to be installed; anything more is its unpacked files.
fn is_unpacked(dir: &Path) -> std::io::Result<bool> {
    let mut files = 0;
    for entry in std::fs::read_dir(dir)? {
        let kind = entry?.file_type()?;
        if kind.is_dir() {
            return Ok(true);
        }
        files += 1;
    }
    Ok(files != 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gameyfin-untracked-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(found: &[UntrackedFolder]) -> Vec<&str> {
        let mut names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        names.sort();
        names
    }

    #[test]
    fn only_folders_no_game_claims_are_listed() {
        let root = scratch("scan");
        let layout = InstallLayout::new(&root);
        let installs = layout.installs_root();
        for name in [
            "(1) Known",
            "(2) Gone",
            "Copied In",
            ".incoming-(3) Busy",
            "Adopted",
        ] {
            std::fs::create_dir_all(installs.join(name)).unwrap();
        }
        std::fs::create_dir_all(layout.downloads_root().join("Loose")).unwrap();
        let roots = vec![root.display().to_string()];
        let tracked = vec![installs.join("Adopted")];
        let catalog: HashSet<i64> = [1].into();

        let found = scan(&roots, &tracked, Some(&catalog));
        assert_eq!(names(&found), ["(2) Gone", "Copied In", "Loose"]);
        let gone = found.iter().find(|f| f.name == "(2) Gone").unwrap();
        assert_eq!(gone.game_id, Some(2));
        assert!(matches!(gone.folder, LibraryFolder::Installations));
        assert!(matches!(
            found.iter().find(|f| f.name == "Loose").unwrap().folder,
            LibraryFolder::Downloads
        ));

        // Without the catalogue, no id is assumed gone.
        let offline = scan(&roots, &tracked, None);
        assert_eq!(names(&offline), ["Copied In", "Loose"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn one_file_is_a_download_and_more_is_unpacked() {
        let dir = scratch("unpacked");
        std::fs::write(dir.join("game.zip"), b"x").unwrap();
        assert!(!is_unpacked(&dir).unwrap());
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        assert!(is_unpacked(&dir).unwrap());

        let nested = scratch("nested");
        std::fs::create_dir_all(nested.join("bin")).unwrap();
        assert!(is_unpacked(&nested).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&nested).unwrap();
    }
}
