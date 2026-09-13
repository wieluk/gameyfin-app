//! Types for Ludusavi's `--api` JSON output; the per-game payload varies by command, so
//! [`ApiOutput`] is generic over it.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The envelope every `--api` invocation prints on stdout.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiOutput<G> {
    #[serde(default)]
    pub overall: Option<Overall>,
    #[serde(default = "BTreeMap::new")]
    pub games: BTreeMap<String, G>,
    #[serde(default)]
    pub errors: Option<ApiErrors>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overall {
    #[serde(default)]
    pub total_games: u32,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub processed_games: u32,
    #[serde(default)]
    pub processed_bytes: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrors {
    #[serde(default)]
    pub some_games_failed: bool,
    #[serde(default)]
    pub unknown_games: Vec<String>,
    #[serde(default)]
    pub cloud_conflict: Option<serde_json::Value>,
    #[serde(default)]
    pub cloud_sync_failed: Option<serde_json::Value>,
}

/// Per-game payload of `find --api`.
#[derive(Debug, Clone, Deserialize)]
pub struct FoundGame {
    /// Match quality `0.0..=1.0`; optional because certainty is decided by how the lookup
    /// was made, not by this number.
    #[serde(default)]
    pub score: Option<f64>,
}

/// Per-game payload of `backups --api`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupsGame {
    #[serde(default)]
    pub backup_path: Option<String>,
    #[serde(default)]
    pub backups: Vec<BackupEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub name: String,
    pub when: String,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub comment: Option<String>,
}

/// Per-game payload of `backup --api` and `restore --api`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanGame {
    #[serde(default)]
    pub decision: Option<Decision>,
    #[serde(default)]
    pub change: Option<Change>,
    #[serde(default)]
    pub files: BTreeMap<String, ScannedFile>,
    #[serde(default)]
    pub registry: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Decision {
    Processed,
    Cancelled,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Change {
    New,
    Different,
    Removed,
    Same,
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub ignored: bool,
    #[serde(default)]
    pub change: Option<Change>,
    /// Restore only: the path in the backup, when a redirect moved the file elsewhere.
    #[serde(default)]
    pub original_path: Option<String>,
    /// Backup only: the path the file is stored under, when a redirect moved it.
    #[serde(default)]
    pub redirected_path: Option<String>,
    #[serde(default)]
    pub error: Option<FileError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileError {
    #[serde(default)]
    pub message: String,
}

impl<G> ApiOutput<G> {
    /// Games Ludusavi did not recognise; the reliable signal since the exit code alone
    /// cannot distinguish "unknown game" from any other failure.
    pub fn unknown_games(&self) -> &[String] {
        match &self.errors {
            Some(e) => &e.unknown_games,
            None => &[],
        }
    }
}

impl ScanGame {
    pub fn produced_data(&self) -> bool {
        self.decision == Some(Decision::Processed)
            && self.files.values().any(|f| !f.failed && !f.ignored)
    }

    /// Files a restore did not put anywhere real: failed ones, and ones left under `synthetic_root`
    /// with no redirect back, which silently succeed inside a Flatpak.
    pub fn misplaced<'a>(
        &'a self,
        synthetic_root: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a ScannedFile)> + 'a {
        self.files
            .iter()
            .filter(move |(path, file)| {
                !file.ignored && (file.failed || path.starts_with(synthetic_root))
            })
            .map(|(path, file)| (path.as_str(), file))
    }

    /// Whether a preview found live files that differ from the backup at its path. After a
    /// sync that path holds exactly what was synced; before one, every file there is new.
    pub fn changed(&self) -> bool {
        self.files.values().any(|file| {
            !file.failed
                && !file.ignored
                && matches!(file.change, Some(Change::New | Change::Different))
        })
    }

    /// Paths of the files a restore wrote or a backup read, leaving out failed and ignored ones.
    pub fn placed(&self) -> impl Iterator<Item = &str> {
        self.files
            .iter()
            .filter(|(_, file)| !file.failed && !file.ignored)
            .map(|(path, _)| path.as_str())
    }

    /// Total bytes of the files that were successfully handled.
    pub fn bytes(&self) -> u64 {
        self.files
            .values()
            .filter(|f| !f.failed && !f.ignored)
            .map(|f| f.bytes)
            .sum()
    }
}

/// The folders a set of files sits in, without the ones nested inside another, so a restore
/// can say where it put a save in a line rather than a file list.
pub fn folders_of<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut parents: Vec<&std::path::Path> = paths
        .into_iter()
        .filter_map(|path| std::path::Path::new(path).parent())
        .collect();
    parents.sort();
    parents.dedup();
    // Sorted, a folder comes before anything inside it, so comparing to the kept ones is enough.
    let mut kept: Vec<&std::path::Path> = Vec::new();
    for parent in parents {
        if !kept.iter().any(|outer| parent.starts_with(outer)) {
            kept.push(parent);
        }
    }
    kept.iter()
        .map(|folder| folder.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod folders_tests {
    use super::*;

    #[test]
    fn files_in_one_folder_and_below_it_name_that_folder_once() {
        let folders = folders_of([
            "/p/Saves/Save_1.sav",
            "/p/Saves/Save_2.sav",
            "/p/Saves/Slot/extra.sav",
        ]);
        assert_eq!(folders, vec!["/p/Saves".to_string()]);
    }

    #[test]
    fn unrelated_folders_are_each_named() {
        let folders = folders_of(["/a/x.sav", "/b/y.cfg"]);
        assert_eq!(folders, vec!["/a".to_string(), "/b".to_string()]);
    }

    #[test]
    fn placed_leaves_out_what_was_not_written() {
        let game: ScanGame = serde_json::from_str(
            r#"{"decision":"Processed","files":{
            "/p/a.sav":{"bytes":1},
            "/p/b.sav":{"bytes":1,"failed":true},
            "/p/c.sav":{"bytes":1,"ignored":true}}}"#,
        )
        .unwrap();
        assert_eq!(game.placed().collect::<Vec<_>>(), vec!["/p/a.sav"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_find_output() {
        let json = r#"{"games":{"Celeste":{"score":0.87},"Celeste Classic":{"score":0.42}}}"#;
        let out: ApiOutput<FoundGame> = serde_json::from_str(json).unwrap();
        assert_eq!(out.games.len(), 2);
        assert_eq!(out.games["Celeste"].score, Some(0.87));
    }

    #[test]
    fn exact_match_has_no_score() {
        let json = r#"{"games":{"Celeste":{}}}"#;
        let out: ApiOutput<FoundGame> = serde_json::from_str(json).unwrap();
        assert_eq!(out.games["Celeste"].score, None);
    }

    #[test]
    fn parses_backup_output_and_summarises() {
        let json = r#"{
            "overall": {"totalGames":1,"totalBytes":150,"processedGames":1,"processedBytes":150},
            "games": {"Game 1": {
                "decision":"Processed","change":"Different",
                "files":{
                    "/games/game1/save.json":{"bytes":100,"change":"Same","failed":false,"ignored":false},
                    "/games/game1/other.bin":{"bytes":50,"change":"New","failed":false,"ignored":false}
                },
                "registry":{}
            }}
        }"#;
        let out: ApiOutput<ScanGame> = serde_json::from_str(json).unwrap();
        let game = &out.games["Game 1"];
        assert!(game.produced_data());
        assert_eq!(game.bytes(), 150);
        assert_eq!(out.overall.unwrap().processed_bytes, 150);
    }

    #[test]
    fn ignored_and_failed_files_are_excluded_from_bytes() {
        let json = r#"{"games":{"G":{"decision":"Processed","files":{
            "a":{"bytes":10,"failed":false,"ignored":false},
            "b":{"bytes":99,"failed":true,"ignored":false},
            "c":{"bytes":99,"failed":false,"ignored":true}
        }}}}"#;
        let out: ApiOutput<ScanGame> = serde_json::from_str(json).unwrap();
        let game = &out.games["G"];
        assert_eq!(game.bytes(), 10);
    }

    #[test]
    fn parses_backups_listing() {
        let json = r#"{"games":{"Celeste":{"backupPath":"/b/Celeste","backups":[
            {"name":"2024-01-02T03-04-05.zip","when":"2024-01-02T03:04:05Z","os":"windows","locked":true,"comment":"pre-patch"}
        ]}}}"#;
        let out: ApiOutput<BackupsGame> = serde_json::from_str(json).unwrap();
        let g = &out.games["Celeste"];
        assert_eq!(g.backup_path.as_deref(), Some("/b/Celeste"));
        assert!(g.backups[0].locked);
        assert_eq!(g.backups[0].comment.as_deref(), Some("pre-patch"));
    }

    #[test]
    fn parses_error_envelope() {
        let json = r#"{"games":{},"errors":{"someGamesFailed":true,"unknownGames":["Nope"]}}"#;
        let out: ApiOutput<ScanGame> = serde_json::from_str(json).unwrap();
        let errors = out.errors.unwrap();
        assert!(errors.some_games_failed);
        assert_eq!(errors.unknown_games, vec!["Nope"]);
    }
}

#[cfg(test)]
mod misplaced_tests {
    use super::*;

    fn game(json: &str) -> ScanGame {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_restore_mapped_onto_this_machine_misplaced_nothing() {
        // Real output: the key is where the file went, `originalPath` where it came from.
        let restored = game(
            r#"{"decision":"Processed","change":"Different","files":{
            "/var/home/u/P/1/drive_c/users/steamuser/AppData/Save_1.sav":{
                "change":"New","bytes":14,
                "originalPath":"/gameyfin/home/AppData/Save_1.sav"}}}"#,
        );
        assert_eq!(restored.misplaced("/gameyfin/").count(), 0);
    }

    #[test]
    fn a_restore_with_no_redirect_is_misplaced_whether_or_not_it_failed() {
        // Outside a sandbox the write to a synthetic path fails outright.
        let refused = game(
            r#"{"decision":"Processed","files":{
            "/gameyfin/home/AppData/Save_1.sav":{"failed":true,
                "error":{"message":"Permission denied (os error 13)"},"bytes":14}}}"#,
        );
        assert_eq!(refused.misplaced("/gameyfin/").count(), 1);

        // Inside a Flatpak it succeeds into scratch space that vanishes with the process.
        let vanished = game(
            r#"{"decision":"Processed","files":{
            "/gameyfin/home/AppData/Save_1.sav":{"change":"New","bytes":585374},
            "/gameyfin/home/AppData/Save_2.sav":{"change":"New","bytes":64}}}"#,
        );
        assert_eq!(vanished.misplaced("/gameyfin/").count(), 2);
    }
}

#[cfg(test)]
mod changed_tests {
    use super::*;

    #[test]
    fn only_files_that_differ_from_the_last_sync_count_as_changed() {
        let same: ScanGame = serde_json::from_str(
            r#"{"decision":"Processed","files":{"/p/Save_1.sav":{"change":"Same","bytes":64}}}"#,
        )
        .unwrap();
        assert!(!same.changed());

        let played: ScanGame = serde_json::from_str(
            r#"{"decision":"Processed","files":{"/p/Save_1.sav":{"change":"Different","bytes":90}}}"#,
        )
        .unwrap();
        assert!(played.changed());
    }
}
