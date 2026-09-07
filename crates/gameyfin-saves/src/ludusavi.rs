//! The Ludusavi driver.
//!
//! Integration is by subprocess plus `--api` JSON rather than the `ludusavi` library
//! crate: `src/lib.rs` upstream warns that its Rust API is unstable, while the CLI's JSON
//! output is schema-documented and versioned.
//!
//! Two invariants hold for every call:
//!
//! * `--config <dir>` points at a directory this app owns, so the user's own
//!   `~/.config/ludusavi` is never read or written. This directory must be **persistent
//!   and shared** across operations: `--config` relocates the manifest cache too, so a
//!   fresh directory forces a full manifest download (tens of MB) before the command
//!   runs. With a stable directory the manifest is fetched once and then refreshed at
//!   most daily.
//! * `--force` is always passed. Without it Ludusavi falls back to an interactive
//!   `dialoguer` confirmation, which would hang a background sync forever.
//!
//! Deliberately *not* used: `ludusavi wrap`. It blocks until the wrapped game exits and
//! owns process launching, which conflicts with this app supervising the game itself.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::api::{ApiOutput, BackupsGame, FoundGame, ScanGame};
use crate::error::{SaveError, SaveResult};
use crate::runner::{CommandRunner, ProcessRunner};

/// Archive format for a backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupFormat {
    /// One zip per backup, the sensible unit to upload to a server.
    Zip,
    /// Plain files mirroring the original tree.
    Simple,
}

impl BackupFormat {
    fn as_str(self) -> &'static str {
        match self {
            BackupFormat::Zip => "zip",
            BackupFormat::Simple => "simple",
        }
    }
}

/// How a game was identified to Ludusavi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameQuery {
    /// Exact and deterministic. Preferred whenever the Steam AppID is known.
    SteamId(u32),
    GogId(u64),
    /// Matched on title, optionally with normalization or fuzzy matching.
    Title {
        title: String,
        normalized: bool,
        fuzzy: bool,
    },
}

pub struct Ludusavi {
    binary: PathBuf,
    config_dir: PathBuf,
    runner: Box<dyn CommandRunner>,
}

impl Ludusavi {
    pub fn new(binary: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            config_dir: config_dir.into(),
            runner: Box::new(ProcessRunner),
        }
    }

    pub fn with_runner(
        binary: impl Into<PathBuf>,
        config_dir: impl Into<PathBuf>,
        runner: Box<dyn CommandRunner>,
    ) -> Self {
        Self {
            binary: binary.into(),
            config_dir: config_dir.into(),
            runner,
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Global flags that precede every subcommand.
    fn base_args(&self) -> Vec<String> {
        vec![
            "--config".into(),
            self.config_dir.to_string_lossy().into_owned(),
        ]
    }

    async fn run_api<T: DeserializeOwned>(
        &self,
        command: &str,
        args: Vec<String>,
    ) -> SaveResult<T> {
        let output = self
            .runner
            .run(&self.binary.to_string_lossy(), &args)
            .await?;

        // A non-zero exit does not mean there is no result. Ludusavi 0.31 exits 1 for an
        // unrecognised game while still printing a complete JSON document describing it
        // under `errors.unknownGames`. Rejecting on the exit code alone would throw that
        // detail away and report a generic failure, so the payload is preferred whenever
        // one parses; the exit code only decides how to describe an unusable response.
        let stdout = output.stdout.trim();
        let parse_error = if stdout.is_empty() {
            None
        } else {
            match serde_json::from_str::<T>(stdout) {
                Ok(parsed) => return Ok(parsed),
                Err(source) => Some(source),
            }
        };

        if !output.success() {
            return Err(SaveError::CommandFailed {
                command: command.to_string(),
                status: output.status,
                stderr: output.stderr.trim().to_string(),
            });
        }

        Err(match parse_error {
            Some(source) => SaveError::Parse {
                command: command.to_string(),
                source,
            },
            None => SaveError::EmptyOutput {
                command: command.to_string(),
            },
        })
    }

    /// Look a game up in the manifest.
    pub async fn find(&self, query: &GameQuery) -> SaveResult<ApiOutput<FoundGame>> {
        let mut args = self.base_args();
        args.push("find".into());
        args.push("--api".into());

        match query {
            GameQuery::SteamId(id) => {
                args.push("--steam-id".into());
                args.push(id.to_string());
            }
            GameQuery::GogId(id) => {
                args.push("--gog-id".into());
                args.push(id.to_string());
            }
            GameQuery::Title {
                title,
                normalized,
                fuzzy,
            } => {
                if *normalized {
                    args.push("--normalized".into());
                }
                if *fuzzy {
                    args.push("--fuzzy".into());
                    // Fuzzy matching without candidates to compare is not useful: the
                    // caller needs the alternatives to ask the user about.
                    args.push("--multiple".into());
                }
                args.push(title.clone());
            }
        }

        self.run_api("find", args).await
    }

    /// Back a single game up into `staging`.
    ///
    /// Retention is left at one full backup with no differentials: the server owns
    /// version history (see the save-sync plan), so keeping a second copy locally would
    /// only inflate what has to be uploaded.
    pub async fn backup(
        &self,
        title: &str,
        staging: &Path,
        format: BackupFormat,
    ) -> SaveResult<ApiOutput<ScanGame>> {
        let mut args = self.base_args();
        args.extend([
            "backup".into(),
            "--api".into(),
            "--force".into(),
            "--no-cloud-sync".into(),
            "--format".into(),
            format.as_str().into(),
            "--full-limit".into(),
            "1".into(),
            "--differential-limit".into(),
            "0".into(),
            "--path".into(),
            staging.to_string_lossy().into_owned(),
            title.to_string(),
        ]);

        if format == BackupFormat::Zip {
            // zstd gives the best ratio-to-time for save data, which is small and
            // highly compressible.
            args.insert(args.len() - 1, "--compression".into());
            args.insert(args.len() - 1, "zstd".into());
        }

        let out: ApiOutput<ScanGame> = self.run_api("backup", args).await?;
        reject_unknown_game("backup", title, &out)?;
        Ok(out)
    }

    /// Restore a single game from a previously downloaded backup directory.
    pub async fn restore(&self, title: &str, source: &Path) -> SaveResult<ApiOutput<ScanGame>> {
        let mut args = self.base_args();
        args.extend([
            "restore".into(),
            "--api".into(),
            "--force".into(),
            "--no-cloud-sync".into(),
            "--path".into(),
            source.to_string_lossy().into_owned(),
            title.to_string(),
        ]);

        let out: ApiOutput<ScanGame> = self.run_api("restore", args).await?;
        reject_unknown_game("restore", title, &out)?;
        Ok(out)
    }

    /// List the backups present in a directory.
    pub async fn backups(&self, path: &Path) -> SaveResult<ApiOutput<BackupsGame>> {
        let mut args = self.base_args();
        args.extend([
            "backups".into(),
            "--api".into(),
            "--path".into(),
            path.to_string_lossy().into_owned(),
        ]);

        self.run_api("backups", args).await
    }
}

/// Turn Ludusavi's "unknown game" reporting into a specific error.
///
/// Asked for a title it cannot find, Ludusavi writes nothing, exits 1, and names the game
/// under `errors.unknownGames`. Mapping that to [`SaveError::NoMatch`] tells the caller the
/// game needs identifying, a custom-game entry or a different title, rather than leaving
/// it as an opaque command failure it cannot act on.
fn reject_unknown_game(command: &str, title: &str, out: &ApiOutput<ScanGame>) -> SaveResult<()> {
    if out.unknown_games().iter().any(|g| g == title) {
        return Err(SaveError::NoMatch {
            title: title.to_string(),
        });
    }
    let _ = command;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;
    use std::sync::Arc;

    fn ludusavi(runner: Arc<FakeRunner>) -> Ludusavi {
        struct Shared(Arc<FakeRunner>);
        #[async_trait::async_trait]
        impl CommandRunner for Shared {
            async fn run(
                &self,
                program: &str,
                args: &[String],
            ) -> SaveResult<crate::runner::CommandOutput> {
                self.0.run(program, args).await
            }
        }
        Ludusavi::with_runner("/opt/ludusavi", "/cfg", Box::new(Shared(runner)))
    }

    #[tokio::test]
    async fn find_by_steam_id_is_exact() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok(
            r#"{"games":{"Celeste":{}}}"#,
        )]));
        let lud = ludusavi(runner.clone());
        let out = lud.find(&GameQuery::SteamId(504230)).await.unwrap();

        assert!(out.games.contains_key("Celeste"));
        let args = runner.call(0);
        assert_eq!(
            args,
            vec!["--config", "/cfg", "find", "--api", "--steam-id", "504230"]
        );
    }

    #[tokio::test]
    async fn fuzzy_find_requests_multiple_candidates() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok(
            r#"{"games":{"Celeste":{"score":0.91}}}"#,
        )]));
        let lud = ludusavi(runner.clone());
        lud.find(&GameQuery::Title {
            title: "celeste".into(),
            normalized: false,
            fuzzy: true,
        })
        .await
        .unwrap();

        let args = runner.call(0);
        assert!(args.contains(&"--fuzzy".to_string()));
        // Without --multiple the caller cannot show the user alternatives.
        assert!(args.contains(&"--multiple".to_string()));
        assert_eq!(args.last().unwrap(), "celeste");
    }

    #[tokio::test]
    async fn backup_is_non_interactive_and_scoped() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok(
            r#"{"games":{"Celeste":{"decision":"Processed","files":{"a":{"bytes":5}}}}}"#,
        )]));
        let lud = ludusavi(runner.clone());
        let out = lud
            .backup("Celeste", Path::new("/tmp/stage"), BackupFormat::Zip)
            .await
            .unwrap();

        assert!(out.games["Celeste"].produced_data());
        let args = runner.call(0);
        // --force is what stops Ludusavi blocking on an interactive confirmation.
        assert!(args.contains(&"--force".to_string()));
        assert!(args.contains(&"--no-cloud-sync".to_string()));
        assert!(args.windows(2).any(|w| w == ["--format", "zip"]));
        assert!(args.windows(2).any(|w| w == ["--compression", "zstd"]));
        assert!(args.windows(2).any(|w| w == ["--path", "/tmp/stage"]));
        // The server owns version history, so only one full backup is kept locally.
        assert!(args.windows(2).any(|w| w == ["--full-limit", "1"]));
        assert!(args.windows(2).any(|w| w == ["--differential-limit", "0"]));
        assert_eq!(args.last().unwrap(), "Celeste");
    }

    #[tokio::test]
    async fn simple_format_omits_compression() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok(r#"{"games":{}}"#)]));
        let lud = ludusavi(runner.clone());
        lud.backup("G", Path::new("/s"), BackupFormat::Simple)
            .await
            .unwrap();

        let args = runner.call(0);
        assert!(args.windows(2).any(|w| w == ["--format", "simple"]));
        assert!(!args.contains(&"--compression".to_string()));
    }

    #[tokio::test]
    async fn restore_targets_the_downloaded_directory() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok(
            r#"{"games":{"Celeste":{"decision":"Processed","files":{}}}}"#,
        )]));
        let lud = ludusavi(runner.clone());
        lud.restore("Celeste", Path::new("/tmp/dl")).await.unwrap();

        let args = runner.call(0);
        assert!(args.contains(&"restore".to_string()));
        assert!(args.contains(&"--force".to_string()));
        assert!(args.windows(2).any(|w| w == ["--path", "/tmp/dl"]));
    }

    #[tokio::test]
    async fn unknown_game_is_an_error_despite_a_zero_exit() {
        // Ludusavi 0.31 exits 0 here and writes nothing, reporting only via errors.
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok(
            r#"{"errors":{"unknownGames":["Nope"]},"overall":{"totalGames":0},"games":{}}"#,
        )]));
        let lud = ludusavi(runner);
        let err = lud
            .backup("Nope", Path::new("/s"), BackupFormat::Zip)
            .await
            .unwrap_err();
        assert!(matches!(err, SaveError::NoMatch { ref title } if title == "Nope"));
    }

    #[tokio::test]
    async fn unknown_other_game_does_not_fail_this_backup() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok(
            r#"{"errors":{"unknownGames":["Other"]},"games":{"Celeste":{"decision":"Processed","files":{"a":{"bytes":1}}}}}"#,
        )]));
        let lud = ludusavi(runner);
        let out = lud
            .backup("Celeste", Path::new("/s"), BackupFormat::Zip)
            .await
            .unwrap();
        assert!(out.games["Celeste"].produced_data());
    }

    #[tokio::test]
    async fn non_zero_exit_is_reported_with_stderr() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::failure(1, "boom")]));
        let lud = ludusavi(runner);
        let err = lud
            .backup("G", Path::new("/s"), BackupFormat::Zip)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            SaveError::CommandFailed { status: 1, ref stderr, .. } if stderr == "boom"
        ));
    }

    #[tokio::test]
    async fn empty_stdout_is_distinguished_from_bad_json() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok("   ")]));
        let lud = ludusavi(runner);
        let err = lud.find(&GameQuery::SteamId(1)).await.unwrap_err();
        assert!(matches!(err, SaveError::EmptyOutput { .. }));
    }

    #[tokio::test]
    async fn malformed_json_is_a_parse_error() {
        let runner = Arc::new(FakeRunner::new(vec![FakeRunner::ok("not json")]));
        let lud = ludusavi(runner);
        let err = lud.find(&GameQuery::SteamId(1)).await.unwrap_err();
        assert!(matches!(err, SaveError::Parse { .. }));
    }
}
