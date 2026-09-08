//! What the client knows about each game locally. A download and an install are separate:
//! a finished download is an archive on disk, not a playable game.
//!
//! ```text
//! NotInstalled → Downloading → Downloaded → Installing → Installed
//!                     ↓             ↓            ↓
//!                   Failed        Failed       Failed
//! ```
//!
//! `Downloaded` is a resting state with an Install action against it. Durable facts (paths,
//! chosen executable) are persisted; in-flight progress is not (the download's own
//! checkpoint handles resume).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub const RECORDS_FILE: &str = "library.json";

/// Subdirectory of a game's download folder holding its unpacked files.
pub const EXTRACT_DIR: &str = "extracted";

/// The durable part of a game's local state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameRecord {
    /// A completed download, awaiting installation.
    pub archive_path: Option<PathBuf>,
    pub archive_bytes: u64,
    /// Where the archive was unpacked, inside the Downloads folder.
    ///
    /// Extraction is staging, not installation: the files sit next to the archive until
    /// the user decides what should happen to them, run a setup program, or move them
    /// into the installations folder as a portable game.
    pub extracted_dir: Option<PathBuf>,
    /// Where the game was installed.
    pub install_dir: Option<PathBuf>,
    /// Chosen launch executable, relative to `install_dir`.
    pub executable: Option<String>,
    pub installed_at: Option<String>,
    /// The setup program already run for this game, relative to the staging directory.
    ///
    /// Recorded so it is not offered again as "another installer", the one that
    /// installed the game is not a DLC waiting to be added.
    pub used_setup: Option<String>,
    /// Setup programs found after extracting, relative to `install_dir`.
    ///
    /// An archive sometimes contains an installer rather than a ready-to-run game, so
    /// finishing extraction is not always finishing installation.
    pub setup_candidates: Vec<String>,
    /// Setup programs still sitting in the staging directory, relative to it.
    ///
    /// Cached rather than scanned on demand: this is read for every game every time the
    /// library is listed, and the scan is a recursive directory walk.
    pub staging_setups: Vec<String>,
    pub minutes_played: u32,
    pub last_played_at: Option<String>,
    /// Extra options passed when the game starts, as the user typed them.
    ///
    /// Stored as one string rather than a parsed list because that is what is shown back
    /// in the settings box, and a round trip through a list loses how it was written.
    #[serde(default)]
    pub launch_arguments: String,
    /// Extra options passed to the game's setup program.
    #[serde(default)]
    pub installer_arguments: String,
    /// A setup program Windows refused to start without administrator rights.
    ///
    /// Remembered so the offer to try again elevated survives a restart, and so the retry
    /// runs the program that actually needed it rather than re-deriving a guess. Cleared
    /// as soon as an install attempt gets past the spawn.
    #[serde(default)]
    pub elevation_program: Option<PathBuf>,
}

impl GameRecord {
    pub fn is_installed(&self) -> bool {
        // The directory must exist *and* contain something. A setup program that failed
        // leaves the destination behind empty, and treating that as installed moves the
        // game out of Downloads and offers a Play button that cannot work.
        self.install_dir.as_ref().is_some_and(|d| has_content(d))
    }

    pub fn is_downloaded(&self) -> bool {
        self.archive_path.as_ref().is_some_and(|p| p.exists())
    }

    pub fn is_extracted(&self) -> bool {
        self.extracted_dir.as_ref().is_some_and(|d| d.exists())
    }

    /// The downloaded archive, if it is still on disk.
    pub fn existing_archive(&self) -> Option<PathBuf> {
        self.archive_path.clone().filter(|p| p.exists())
    }

    /// The unpacked staging directory, if it is still on disk.
    pub fn existing_staging(&self) -> Option<PathBuf> {
        self.extracted_dir.clone().filter(|d| d.exists())
    }
}

/// Setup programs inside a staging directory, relative to it.
///
/// Blocking: it walks the directory. Call it where staging changes, not per read.
pub fn scan_staging_setups(staging: &Path) -> Vec<String> {
    gameyfin_core::executable::find_installers(staging)
        .unwrap_or_default()
        .iter()
        .filter_map(|p| {
            p.strip_prefix(staging)
                .ok()
                .map(|r| r.to_string_lossy().into_owned())
        })
        .collect()
}

/// Progress of something happening right now. Not persisted.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Activity {
    Downloading {
        received_bytes: u64,
        total_bytes: u64,
        bytes_per_second: f64,
    },
    Extracting {
        percent: f64,
    },
    Installing {
        percent: f64,
    },
    /// Setting up the compatibility layer before a game or installer can run.
    ///
    /// Its own state because the first run downloads a Proton build, hundreds of
    /// megabytes, and without saying so the app simply appears to do nothing.
    Preparing {
        message: String,
    },
    Running {
        since: String,
    },
    Failed {
        message: String,
        /// Which step failed, so the UI can offer the right retry.
        stage: Stage,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Download,
    Extract,
    Install,
    Launch,
}

/// What the UI should render for a game.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum GameState {
    NotInstalled,
    Downloading {
        received_bytes: u64,
        total_bytes: u64,
        bytes_per_second: f64,
    },
    /// Archive on disk, not yet unpacked.
    Downloaded {
        archive_path: String,
        bytes: u64,
    },
    Extracting {
        percent: f64,
    },
    /// Unpacked into the Downloads folder, awaiting a decision about installing.
    Extracted {
        path: String,
        /// Setup programs found in the unpacked files, relative to `path`.
        setup_candidates: Vec<String>,
        /// True when the archive is still on disk and could be reclaimed.
        archive_present: bool,
    },
    Installing {
        percent: f64,
    },
    /// Setting up the compatibility layer. See [`Activity::Preparing`].
    Preparing {
        message: String,
    },
    Installed {
        path: String,
        executable: Option<String>,
        /// Setup programs found in the installed files.
        setup_candidates: Vec<String>,
        /// Setup programs still sitting in the unpacked download, such as a DLC installer.
        staging_setups: Vec<String>,
        /// Whether unpacked files are still taking up space in Downloads.
        staging_present: bool,
    },
    Running {
        since: String,
    },
    Failed {
        message: String,
        stage: Stage,
        /// Whether the step failed only because Windows wants it run as administrator.
        ///
        /// Kept apart from the message so the UI can offer the retry that actually helps,
        /// rather than a plain "Retry install" that will fail again the same way.
        elevation_required: bool,
    },
}

#[derive(Default)]
pub struct LibraryState {
    records: RwLock<HashMap<i64, GameRecord>>,
    activity: RwLock<HashMap<i64, Activity>>,
    config_dir: RwLock<PathBuf>,
}

impl LibraryState {
    pub async fn load(&self, config_dir: PathBuf) {
        *self.config_dir.write().await = config_dir.clone();
        let path = config_dir.join(RECORDS_FILE);

        let records: HashMap<i64, GameRecord> = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                // Losing local bookkeeping is recoverable, a rescan rebuilds it, so a
                // corrupt file must not stop the app starting.
                tracing::warn!("ignoring unreadable library records at {path:?}: {e}");
                HashMap::new()
            }),
            Err(_) => HashMap::new(),
        };

        *self.records.write().await = records;
    }

    async fn persist(&self) {
        let dir = self.config_dir.read().await.clone();
        if dir.as_os_str().is_empty() {
            return;
        }
        let records = self.records.read().await.clone();

        if let Err(e) = async {
            tokio::fs::create_dir_all(&dir).await?;
            let json = serde_json::to_vec_pretty(&records)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            tokio::fs::write(dir.join(RECORDS_FILE), json).await
        }
        .await
        {
            tracing::error!("could not save library records: {e}");
        }
    }

    pub async fn record(&self, game_id: i64) -> GameRecord {
        self.records
            .read()
            .await
            .get(&game_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Update a game's durable record and write it out.
    pub async fn update_record(&self, game_id: i64, f: impl FnOnce(&mut GameRecord)) {
        {
            let mut records = self.records.write().await;
            f(records.entry(game_id).or_default());
        }
        self.persist().await;
    }

    pub async fn set_activity(&self, game_id: i64, activity: Activity) {
        self.activity.write().await.insert(game_id, activity);
    }

    pub async fn clear_activity(&self, game_id: i64) {
        self.activity.write().await.remove(&game_id);
    }

    /// True when something is already running for this game.
    pub async fn is_busy(&self, game_id: i64) -> bool {
        matches!(
            self.activity.read().await.get(&game_id),
            Some(
                Activity::Downloading { .. }
                    | Activity::Installing { .. }
                    | Activity::Running { .. }
            )
        )
    }

    /// Resolve the state the UI should show: live activity wins, otherwise what is on disk.
    pub async fn state_of(&self, game_id: i64) -> GameState {
        let record = self.record(game_id).await;
        self.state_from(game_id, record).await
    }

    /// Same, for a caller that already holds the record.
    pub async fn state_from(&self, game_id: i64, record: GameRecord) -> GameState {
        if let Some(activity) = self.activity.read().await.get(&game_id) {
            return match activity {
                Activity::Downloading {
                    received_bytes,
                    total_bytes,
                    bytes_per_second,
                } => GameState::Downloading {
                    received_bytes: *received_bytes,
                    total_bytes: *total_bytes,
                    bytes_per_second: *bytes_per_second,
                },
                Activity::Extracting { percent } => GameState::Extracting { percent: *percent },
                Activity::Installing { percent } => GameState::Installing { percent: *percent },
                Activity::Preparing { message } => GameState::Preparing {
                    message: message.clone(),
                },
                Activity::Running { since } => GameState::Running {
                    since: since.clone(),
                },
                Activity::Failed { message, stage } => GameState::Failed {
                    message: message.clone(),
                    stage: *stage,
                    // Recorded against the game rather than carried in the activity: the
                    // program that needed elevation has to outlive the failure, because
                    // it is what the retry runs.
                    elevation_required: *stage == Stage::Install
                        && record.elevation_program.is_some(),
                },
            };
        }

        if record.is_installed() {
            let staging_present = record.extracted_dir.as_ref().is_some_and(|d| d.exists());
            return GameState::Installed {
                path: record
                    .install_dir
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                executable: record.executable,
                setup_candidates: record.setup_candidates,
                // Kept after installing so leftover setup programs, a DLC installer for
                // instance, stay reachable and the space they occupy stays visible.
                staging_setups: if staging_present {
                    record.staging_setups
                } else {
                    Vec::new()
                },
                staging_present,
            };
        }
        if record.is_extracted() {
            return GameState::Extracted {
                path: record
                    .extracted_dir
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                setup_candidates: record.setup_candidates,
                archive_present: record.archive_path.as_ref().is_some_and(|p| p.exists()),
            };
        }
        if record.is_downloaded() {
            return GameState::Downloaded {
                archive_path: record
                    .archive_path
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                bytes: record.archive_bytes,
            };
        }
        GameState::NotInstalled
    }

    /// Rebuild records by looking at the library folder.
    ///
    /// The `(id) Title` directory naming means an install can be re-attached to its game
    /// after the records file is lost, or when a folder is copied from another machine.
    pub async fn rescan(&self, library_root: &Path) -> usize {
        let layout = gameyfin_core::InstallLayout::new(library_root);
        let mut found = 0;

        for (dir, installed) in [
            (layout.installs_root(), true),
            (layout.downloads_root(), false),
        ] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(game_id) = gameyfin_core::InstallLayout::game_id_from_dir(&path) else {
                    continue;
                };

                if installed {
                    // An empty folder is the residue of a failed install, not an install.
                    if !has_content(&path) {
                        continue;
                    }
                    self.update_record(game_id, |r| r.install_dir = Some(path.clone()))
                        .await;
                } else {
                    let staged = path.join(EXTRACT_DIR);
                    let archive = first_file(&path);
                    let bytes = archive
                        .as_ref()
                        .and_then(|a| std::fs::metadata(a).ok())
                        .map(|m| m.len())
                        .unwrap_or(0);
                    let staged_exists = staged.is_dir();

                    let staging_setups = if staged_exists {
                        scan_staging_setups(&staged)
                    } else {
                        Vec::new()
                    };
                    self.update_record(game_id, |r| {
                        r.archive_path = archive.clone();
                        r.archive_bytes = bytes;
                        if staged_exists {
                            r.extracted_dir = Some(staged.clone());
                            r.staging_setups = staging_setups;
                        }
                    })
                    .await;
                }
                found += 1;
            }
        }

        found
    }
}

/// Whether a directory exists and holds anything at all.
fn has_content(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// The first regular file in a directory, ignoring our own sidecars.
fn first_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().into_owned();
        // Skip the resume checkpoint left beside a partial download.
        (path.is_file() && !name.ends_with(".gameyfin-part")).then_some(path)
    })
}

pub type SharedLibraryState = Arc<LibraryState>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_refused_installer_offers_the_administrator_retry() {
        // Windows returning "needs elevation" is not a broken download, and a plain
        // "Retry install" would be refused identically. The record is what remembers it.
        let state = LibraryState::default();
        state
            .update_record(7, |r| {
                r.elevation_program = Some(PathBuf::from("C:/Downloads/(7) Game/setup.exe"))
            })
            .await;
        state
            .set_activity(
                7,
                Activity::Failed {
                    message: "setup.exe needs to run as administrator.".into(),
                    stage: Stage::Install,
                },
            )
            .await;

        match state.state_of(7).await {
            GameState::Failed {
                elevation_required, ..
            } => assert!(elevation_required),
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_ordinary_failure_offers_no_administrator_retry() {
        let state = LibraryState::default();
        state
            .set_activity(
                8,
                Activity::Failed {
                    message: "The installer exited with code 1.".into(),
                    stage: Stage::Install,
                },
            )
            .await;

        match state.state_of(8).await {
            GameState::Failed {
                elevation_required, ..
            } => assert!(!elevation_required),
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_failed_download_never_offers_the_administrator_retry() {
        // A leftover program from a previous install attempt must not turn an unrelated
        // download failure into an offer to run something as administrator.
        let state = LibraryState::default();
        state
            .update_record(9, |r| {
                r.elevation_program = Some(PathBuf::from("setup.exe"))
            })
            .await;
        state
            .set_activity(
                9,
                Activity::Failed {
                    message: "connection reset".into(),
                    stage: Stage::Download,
                },
            )
            .await;

        match state.state_of(9).await {
            GameState::Failed {
                elevation_required, ..
            } => assert!(!elevation_required),
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_game_is_not_installed() {
        let state = LibraryState::default();
        assert!(matches!(state.state_of(1).await, GameState::NotInstalled));
    }

    #[tokio::test]
    async fn extracted_files_are_staged_not_installed() {
        // Extraction stages files in Downloads; installing is a separate decision.
        let dir = std::env::temp_dir().join(format!("gameyfin-ls-ex-{}", std::process::id()));
        let staged = dir.join(EXTRACT_DIR);
        std::fs::create_dir_all(&staged).unwrap();

        let state = LibraryState::default();
        state
            .update_record(9, |r| {
                r.extracted_dir = Some(staged.clone());
                r.setup_candidates = vec!["setup.exe".into()];
            })
            .await;

        match state.state_of(9).await {
            GameState::Extracted {
                setup_candidates, ..
            } => assert_eq!(setup_candidates, vec!["setup.exe"]),
            other => panic!("expected extracted, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn installed_state_reports_the_cached_staging_setups() {
        // The list is cached on the record rather than rescanned, because state_of runs
        // once per game on every library listing.
        let dir = std::env::temp_dir().join(format!("gameyfin-ls-cache-{}", std::process::id()));
        let staged = dir.join(EXTRACT_DIR);
        let installed = dir.join("installed");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("game.exe"), b"x").unwrap();

        let state = LibraryState::default();
        state
            .update_record(11, |r| {
                r.install_dir = Some(installed.clone());
                r.extracted_dir = Some(staged.clone());
                r.staging_setups = vec!["dlc/setup.exe".into()];
            })
            .await;

        match state.state_of(11).await {
            GameState::Installed {
                staging_setups,
                staging_present,
                ..
            } => {
                assert_eq!(staging_setups, vec!["dlc/setup.exe"]);
                assert!(staging_present);
            }
            other => panic!("expected installed, got {other:?}"),
        }

        // Once the staging directory is gone the cache must not be reported.
        std::fs::remove_dir_all(&staged).unwrap();
        match state.state_of(11).await {
            GameState::Installed {
                staging_setups,
                staging_present,
                ..
            } => {
                assert!(staging_setups.is_empty());
                assert!(!staging_present);
            }
            other => panic!("expected installed, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_completed_download_is_downloaded_not_installed() {
        // The distinction that was previously wrong: an archive on disk is not a game
        // that can be played.
        let dir = std::env::temp_dir().join(format!("gameyfin-ls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("game.zip");
        std::fs::write(&archive, b"x").unwrap();

        let state = LibraryState::default();
        state
            .update_record(1, |r| {
                r.archive_path = Some(archive.clone());
                r.archive_bytes = 1;
            })
            .await;

        assert!(matches!(
            state.state_of(1).await,
            GameState::Downloaded { .. }
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_empty_install_directory_is_not_an_install() {
        // A setup program that failed leaves the destination behind, empty.
        let dir = std::env::temp_dir().join(format!("gameyfin-ls-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let state = LibraryState::default();
        state
            .update_record(11, |r| r.install_dir = Some(dir.clone()))
            .await;
        assert!(matches!(state.state_of(11).await, GameState::NotInstalled));

        // With a file in it, it counts.
        std::fs::write(dir.join("game.exe"), b"x").unwrap();
        assert!(matches!(
            state.state_of(11).await,
            GameState::Installed { .. }
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_install_whose_directory_vanished_is_not_installed() {
        let state = LibraryState::default();
        state
            .update_record(2, |r| {
                r.install_dir = Some(PathBuf::from("/definitely/not/here"))
            })
            .await;
        assert!(matches!(state.state_of(2).await, GameState::NotInstalled));
    }

    #[tokio::test]
    async fn live_activity_wins_over_what_is_on_disk() {
        let state = LibraryState::default();
        state
            .update_record(3, |r| r.install_dir = Some(std::env::temp_dir()))
            .await;
        state
            .set_activity(3, Activity::Installing { percent: 40.0 })
            .await;

        assert!(matches!(
            state.state_of(3).await,
            GameState::Installing { .. }
        ));

        state.clear_activity(3).await;
        assert!(matches!(
            state.state_of(3).await,
            GameState::Installed { .. }
        ));
    }

    #[tokio::test]
    async fn a_failure_is_not_busy_so_it_can_be_retried() {
        let state = LibraryState::default();
        state
            .set_activity(
                4,
                Activity::Failed {
                    message: "boom".into(),
                    stage: Stage::Download,
                },
            )
            .await;
        assert!(!state.is_busy(4).await);
    }

    #[tokio::test]
    async fn downloading_counts_as_busy() {
        let state = LibraryState::default();
        state
            .set_activity(
                5,
                Activity::Downloading {
                    received_bytes: 1,
                    total_bytes: 2,
                    bytes_per_second: 1.0,
                },
            )
            .await;
        assert!(state.is_busy(5).await);
    }

    #[tokio::test]
    async fn records_round_trip_through_disk() {
        let dir = std::env::temp_dir().join(format!("gameyfin-ls-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let state = LibraryState::default();
        state.load(dir.clone()).await;
        state
            .update_record(7, |r| {
                r.executable = Some("Celeste.exe".into());
                r.minutes_played = 42;
            })
            .await;

        let reloaded = LibraryState::default();
        reloaded.load(dir.clone()).await;
        let record = reloaded.record(7).await;
        assert_eq!(record.executable.as_deref(), Some("Celeste.exe"));
        assert_eq!(record.minutes_played, 42);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
