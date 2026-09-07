//! Types for Ludusavi's `--api` JSON output.
//!
//! Shapes follow `docs/schema/general-output.yaml` in the Ludusavi repository. The
//! top-level envelope is shared across commands, but the per-game payload differs, so
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
    /// Match quality in `0.0..=1.0`. Ludusavi 0.31 reports 1.0 for exact and ID-based
    /// matches, but the field is optional here because the schema allows its absence and
    /// certainty is decided by *how* the lookup was made, not by this number.
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
}

impl<G> ApiOutput<G> {
    /// Games Ludusavi did not recognise.
    ///
    /// Ludusavi exits 1 in this case but still prints a full JSON document, so this is
    /// the reliable signal, the exit code alone cannot distinguish "unknown game" from
    /// any other failure.
    pub fn unknown_games(&self) -> &[String] {
        match &self.errors {
            Some(e) => &e.unknown_games,
            None => &[],
        }
    }
}

impl ScanGame {
    /// Whether the scan actually captured something, as opposed to running but finding
    /// nothing worth writing.
    pub fn produced_data(&self) -> bool {
        self.decision == Some(Decision::Processed)
            && self.files.values().any(|f| !f.failed && !f.ignored)
    }

    /// Total bytes of the files that were successfully handled.
    pub fn bytes(&self) -> u64 {
        self.files
            .values()
            .filter(|f| !f.failed && !f.ignored)
            .map(|f| f.bytes)
            .sum()
    }

    pub fn any_failed(&self) -> bool {
        self.files.values().any(|f| f.failed)
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
        assert!(!game.any_failed());
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
        assert!(game.any_failed());
    }

    #[test]
    fn scan_with_no_usable_files_produced_nothing() {
        let json = r#"{"games":{"G":{"decision":"Processed","files":{
            "a":{"bytes":10,"failed":true,"ignored":false}
        }}}}"#;
        let out: ApiOutput<ScanGame> = serde_json::from_str(json).unwrap();
        assert!(!out.games["G"].produced_data());
    }

    #[test]
    fn cancelled_scan_produced_nothing() {
        let json = r#"{"games":{"G":{"decision":"Cancelled","files":{
            "a":{"bytes":10,"failed":false,"ignored":false}
        }}}}"#;
        let out: ApiOutput<ScanGame> = serde_json::from_str(json).unwrap();
        assert!(!out.games["G"].produced_data());
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
