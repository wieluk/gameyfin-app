//! Where games live on disk. The game id is encoded in the directory name (`(123) Celeste`)
//! so a library can be reconstructed from bare folders after a lost config.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct InstallLayout {
    root: PathBuf,
}

impl InstallLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The folder every download lands in, one directory per game beneath it.
    pub fn downloads_root(&self) -> PathBuf {
        self.root.join("Gameyfin").join("Downloads")
    }

    pub fn installs_root(&self) -> PathBuf {
        self.root.join("Gameyfin").join("Installations")
    }

    pub fn downloads_dir(&self, game_id: i64, title: &str) -> PathBuf {
        self.downloads_root()
            .join(Self::folder_name(game_id, title))
    }

    /// In the library folder, not the config directory, so a Wine outside the sandbox sees the
    /// same path and a user can find a broken prefix.
    pub fn prefix_dir(&self, game_id: i64) -> PathBuf {
        self.prefixes_root().join(game_id.to_string())
    }

    pub fn prefixes_root(&self) -> PathBuf {
        self.root.join("Gameyfin").join("Prefixes")
    }

    pub fn install_dir(&self, game_id: i64, title: &str) -> PathBuf {
        self.installs_root().join(Self::folder_name(game_id, title))
    }

    /// Save backup staging, one directory per game. In the library folder for the reason
    /// prefixes are: it is user data, and a Wine outside the sandbox has to reach it.
    pub fn saves_root(&self) -> PathBuf {
        self.root.join("Gameyfin").join("Saves")
    }

    pub fn saves_dir(&self, game_id: i64) -> PathBuf {
        self.saves_root().join(game_id.to_string())
    }

    /// `(<id>) <title>`, with characters no filesystem will accept removed.
    fn folder_name(game_id: i64, title: &str) -> String {
        format!("({game_id}) {}", sanitize(title))
    }

    /// The game id in a folder name, which is what re-attaches an install to its game after
    /// the local records are lost or a folder is copied from another machine.
    pub fn game_id_from_dir(dir: &Path) -> Option<i64> {
        let name = dir.file_name()?.to_str()?;
        let inner = name.strip_prefix('(')?;
        let (id, _) = inner.split_once(')')?;
        id.parse().ok()
    }
}

/// Strips characters no filename may hold. Windows' rules everywhere, so a library on a
/// shared drive produces identical names on both systems.
fn sanitize(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if crate::prefix::is_illegal_filename_char(c) {
                ' '
            } else {
                c
            }
        })
        .collect();

    // Windows also refuses names ending in a dot or space.
    let trimmed = cleaned.trim().trim_end_matches('.').trim_end();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roots_are_the_parents_of_a_game_folder() {
        let l = InstallLayout::new("/library");
        assert_eq!(
            l.downloads_dir(12, "Celeste").parent(),
            Some(l.downloads_root().as_path())
        );
        assert_eq!(
            l.install_dir(12, "Celeste").parent(),
            Some(l.installs_root().as_path())
        );
        assert_eq!(l.prefix_dir(12).parent(), Some(l.prefixes_root().as_path()));
        assert_eq!(l.saves_dir(12).parent(), Some(l.saves_root().as_path()));
    }

    #[test]
    fn save_staging_is_separate_from_installs_and_prefixes() {
        let l = InstallLayout::new("/library");
        assert_eq!(l.saves_dir(12), PathBuf::from("/library/Gameyfin/Saves/12"));
        assert_ne!(l.saves_root(), l.prefixes_root());
        assert_ne!(l.saves_root(), l.installs_root());
    }

    #[test]
    fn download_and_install_dirs_are_separate() {
        let l = InstallLayout::new("/library");
        assert_eq!(
            l.downloads_dir(12, "Celeste"),
            PathBuf::from("/library/Gameyfin/Downloads/(12) Celeste")
        );
        assert_eq!(
            l.install_dir(12, "Celeste"),
            PathBuf::from("/library/Gameyfin/Installations/(12) Celeste")
        );
    }

    #[test]
    fn prefixes_live_beside_the_games() {
        let l = InstallLayout::new("/library");
        assert_eq!(
            l.prefix_dir(93),
            PathBuf::from("/library/Gameyfin/Prefixes/93")
        );
    }

    #[test]
    fn game_id_round_trips_through_the_directory_name() {
        let l = InstallLayout::new("/library");
        let dir = l.install_dir(4567, "Return of the Obra Dinn");
        assert_eq!(InstallLayout::game_id_from_dir(&dir), Some(4567));
    }

    #[test]
    fn unrelated_directories_yield_no_id() {
        assert_eq!(
            InstallLayout::game_id_from_dir(Path::new("/library/Some Game")),
            None
        );
        assert_eq!(
            InstallLayout::game_id_from_dir(Path::new("/library/(abc) Game")),
            None
        );
    }

    #[test]
    fn invalid_filename_characters_are_replaced() {
        let l = InstallLayout::new("/library");
        let dir = l.install_dir(1, "Where/Are: My *Saves?");
        assert_eq!(
            dir.file_name().unwrap().to_str().unwrap(),
            "(1) Where Are  My  Saves"
        );
        // Still recoverable afterwards.
        assert_eq!(InstallLayout::game_id_from_dir(&dir), Some(1));
    }

    #[test]
    fn titles_that_sanitize_to_nothing_get_a_placeholder() {
        let l = InstallLayout::new("/library");
        let dir = l.install_dir(9, "///");
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), "(9) Untitled");
    }

    #[test]
    fn trailing_dots_are_removed_for_windows() {
        let l = InstallLayout::new("/library");
        let dir = l.install_dir(3, "Portal 2...");
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), "(3) Portal 2");
    }
}
