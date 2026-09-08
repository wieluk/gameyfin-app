//! Where downloaded files go. Path policy only (state lives in [`crate::library_state`]);
//! the `(id) Title` directory naming is what re-attaches an install to its game after the
//! local records are lost.

use std::path::PathBuf;

/// Destination for a game's download, given the configured library root.
pub fn download_path(library_root: &str, game_id: i64, title: &str, filename: &str) -> PathBuf {
    gameyfin_core::InstallLayout::new(library_root)
        .downloads_dir(game_id, title)
        .join(filename)
}

/// A filesystem-safe filename for a game whose server filename is not yet known.
pub fn provisional_filename(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if gameyfin_core::prefix::is_illegal_filename_char(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_path_uses_the_shared_layout() {
        let path = download_path("/library", 12, "Celeste", "Celeste.zip");
        assert_eq!(
            path,
            PathBuf::from("/library/Gameyfin/Downloads/(12) Celeste/Celeste.zip")
        );
    }

    #[test]
    fn provisional_filenames_are_filesystem_safe() {
        assert_eq!(
            provisional_filename("Where/Are: My *Saves?"),
            "Where_Are_ My _Saves_"
        );
        assert_eq!(provisional_filename("   "), "download");
    }
}
