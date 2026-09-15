//! What the client knows about each game locally: durable records on disk, live activity in
//! memory. A finished download is an archive, not an install.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::progress::TransferProgress;
use crate::state::{read, write};

pub const RECORDS_FILE: &str = "library.json";

/// Subdirectory of a game's download folder holding its unpacked files.
pub const EXTRACT_DIR: &str = "extracted";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameRecord {
    pub archive_path: Option<PathBuf>,
    pub archive_bytes: u64,
    /// Unpacked files, still in Downloads until the user decides how to install them.
    pub extracted_dir: Option<PathBuf>,
    pub install_dir: Option<PathBuf>,
    /// Relative to `install_dir`.
    pub executable: Option<String>,
    pub installed_at: Option<String>,
    /// Setup programs inside `install_dir`, relative to it.
    pub setup_candidates: Vec<String>,
    /// Setup programs inside `extracted_dir`, cached because listing runs per game per refresh.
    pub staging_setups: Vec<String>,
    pub minutes_played: u32,
    pub last_played_at: Option<String>,
    /// Kept as typed, since that is what the options box shows back.
    pub launch_arguments: String,
    pub installer_arguments: String,
    /// One `KEY=value` per line.
    pub launch_environment: String,
    pub proton_build: Option<String>,
    pub launch_toggles: gameyfin_core::environment::LaunchToggles,
    /// Winetricks verbs the last failed start pointed at, offered ticked until installed.
    pub suggested_winetricks: Vec<String>,
    /// A setup program Windows refused without elevation, so the retry runs that exact file.
    pub elevation_program: Option<PathBuf>,
    pub saves: gameyfin_core::LocalSaveState,
    pub save_restore_strategy: SaveRestoreStrategy,
    /// Hand-entered `(source, target)` path mappings.
    pub save_redirects: Vec<(String, String)>,
}

/// Mirrors `gameyfin_saves::RestoreStrategy` with a wire form independent of that crate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SaveRestoreStrategy {
    #[default]
    Portable,
    CrossOs,
}

impl From<bool> for SaveRestoreStrategy {
    fn from(cross_os: bool) -> Self {
        if cross_os {
            Self::CrossOs
        } else {
            Self::Portable
        }
    }
}

impl From<SaveRestoreStrategy> for gameyfin_saves::RestoreStrategy {
    fn from(value: SaveRestoreStrategy) -> Self {
        match value {
            SaveRestoreStrategy::Portable => Self::Portable,
            SaveRestoreStrategy::CrossOs => Self::CrossOs,
        }
    }
}

impl GameRecord {
    /// A failed setup leaves an empty destination, which must not count as installed.
    pub fn is_installed(&self) -> bool {
        self.install_dir.as_deref().is_some_and(has_content)
    }

    pub fn existing_archive(&self) -> Option<PathBuf> {
        self.archive_path.clone().filter(|p| p.exists())
    }

    pub fn existing_staging(&self) -> Option<PathBuf> {
        self.extracted_dir.clone().filter(|d| d.exists())
    }
}

/// Setup programs inside a directory, relative to it. Walks the tree, so call it rarely.
pub fn scan_setups(dir: &Path) -> Vec<String> {
    gameyfin_core::executable::find_installers(dir)
        .unwrap_or_default()
        .iter()
        .filter_map(|p| relative_to(p, dir))
        .collect()
}

pub fn relative_to(path: &Path, base: &Path) -> Option<String> {
    path.strip_prefix(base)
        .ok()
        .map(|r| r.to_string_lossy().into_owned())
}

/// Work happening right now. Not persisted.
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
    /// A setup program or a move into place.
    Installing {
        progress: Option<TransferProgress>,
    },
    /// Anything else that takes a while, described for the row.
    Preparing {
        message: String,
        /// Set while it waits on a download that measures itself.
        progress: Option<TransferProgress>,
    },
    Running {
        since: String,
        executable: Option<String>,
        runtime: Option<String>,
    },
    Failed {
        message: String,
        stage: Stage,
    },
}

impl Activity {
    fn is_busy(&self) -> bool {
        !matches!(self, Activity::Failed { .. })
    }

    fn percent(&self) -> Option<f64> {
        match self {
            Activity::Downloading {
                received_bytes,
                total_bytes,
                ..
            } => Some(if *total_bytes > 0 {
                *received_bytes as f64 / *total_bytes as f64 * 100.0
            } else {
                0.0
            }),
            Activity::Extracting { percent } => Some(*percent),
            Activity::Preparing {
                progress: Some(p), ..
            }
            | Activity::Installing { progress: Some(p) }
                if p.total_bytes > 0 =>
            {
                Some(p.received_bytes as f64 / p.total_bytes as f64 * 100.0)
            }
            _ => None,
        }
    }

    pub fn installing() -> Self {
        Activity::Installing { progress: None }
    }

    pub fn preparing(message: impl Into<String>) -> Self {
        Activity::Preparing {
            message: message.into(),
            progress: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Stage {
    Download,
    Extract,
    Install,
    Launch,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Download => "Download",
            Stage::Extract => "Extract",
            Stage::Install => "Install",
            Stage::Launch => "Launch",
        }
    }
}

/// Work on an installed game, shown on its installed row rather than moving it to Downloads.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum InstalledBusy {
    Preparing {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        progress: Option<TransferProgress>,
    },
    Installing {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        progress: Option<TransferProgress>,
    },
    Failed {
        message: String,
    },
}

/// What the UI renders for a game.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum GameState {
    NotInstalled,
    Downloading {
        received_bytes: u64,
        total_bytes: u64,
        bytes_per_second: f64,
    },
    Downloaded {
        archive_path: String,
        bytes: u64,
    },
    Extracting {
        percent: f64,
    },
    Extracted {
        path: String,
        setup_candidates: Vec<String>,
        archive_present: bool,
    },
    Installing {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        progress: Option<TransferProgress>,
    },
    Preparing {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        progress: Option<TransferProgress>,
    },
    Installed {
        path: String,
        executable: Option<String>,
        setup_candidates: Vec<String>,
        staging_setups: Vec<String>,
        staging_present: bool,
        busy: Option<InstalledBusy>,
    },
    Running {
        since: String,
        executable: Option<String>,
        runtime: Option<String>,
    },
    Failed {
        message: String,
        stage: Stage,
        /// Kept so a failed launch still offers Play and the executable picker.
        path: Option<String>,
        executable: Option<String>,
        /// The retry that helps is "as administrator", not a plain retry.
        elevation_required: bool,
    },
}

struct Slot {
    claim: u64,
    activity: Activity,
}

#[derive(Default)]
pub struct LibraryState {
    records: RwLock<HashMap<i64, GameRecord>>,
    activity: RwLock<HashMap<i64, Slot>>,
    config_dir: OnceLock<PathBuf>,
    writer: tokio::sync::Mutex<()>,
    claims: AtomicU64,
}

pub type SharedLibraryState = Arc<LibraryState>;

/// A game's busy state. Dropping it clears the activity unless it has become a failure or
/// someone else has claimed the game since.
pub struct Claim {
    library: SharedLibraryState,
    game_id: i64,
    id: u64,
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut activity = write(&self.library.activity);
        if activity
            .get(&self.game_id)
            .is_some_and(|slot| slot.claim == self.id && slot.activity.is_busy())
        {
            activity.remove(&self.game_id);
        }
    }
}

impl LibraryState {
    pub async fn load(&self, config_dir: PathBuf) {
        let records = crate::persist::read_json_or_default(&config_dir.join(RECORDS_FILE)).await;
        let _ = self.config_dir.set(config_dir);
        *write(&self.records) = records;
    }

    pub fn record(&self, game_id: i64) -> GameRecord {
        read(&self.records)
            .get(&game_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn update_record(&self, game_id: i64, f: impl FnOnce(&mut GameRecord)) {
        f(write(&self.records).entry(game_id).or_default());
        self.persist().await;
    }

    /// Every folder a record points into, to tell adopted folders from strays.
    pub fn tracked_dirs(&self) -> Vec<PathBuf> {
        let parent = |p: &PathBuf| p.parent().map(Path::to_path_buf);
        read(&self.records)
            .values()
            .flat_map(|r| {
                [
                    r.install_dir.clone(),
                    r.extracted_dir.as_ref().and_then(parent),
                    r.archive_path.as_ref().and_then(parent),
                ]
            })
            .flatten()
            .collect()
    }

    /// Forgets every game's last synced version, for a store that never held those ids.
    pub async fn forget_synced_save_ids(&self) {
        for record in write(&self.records).values_mut() {
            record.saves.last_synced_save_id = None;
        }
        self.persist().await;
    }

    /// Snapshots inside the writer lock, so the last write always carries every change.
    async fn persist(&self) {
        let Some(dir) = self.config_dir.get() else {
            return;
        };
        let _writer = self.writer.lock().await;
        let records = read(&self.records).clone();
        if let Err(e) = crate::persist::write_json(&dir.join(RECORDS_FILE), &records, false).await {
            tracing::error!("could not save library records: {e}");
        }
    }

    /// Marks a game busy. `None` when it already is.
    pub fn claim(self: &Arc<Self>, game_id: i64, activity: Activity) -> Option<Claim> {
        let mut slots = write(&self.activity);
        if slots
            .get(&game_id)
            .is_some_and(|slot| slot.activity.is_busy())
        {
            return None;
        }
        let id = self.claims.fetch_add(1, Ordering::Relaxed) + 1;
        slots.insert(
            game_id,
            Slot {
                claim: id,
                activity,
            },
        );
        Some(Claim {
            library: self.clone(),
            game_id,
            id,
        })
    }

    /// Replaces the activity, keeping whichever claim holds the game.
    pub fn set_activity(&self, game_id: i64, activity: Activity) {
        let mut slots = write(&self.activity);
        let claim = slots.get(&game_id).map_or(0, |slot| slot.claim);
        slots.insert(game_id, Slot { claim, activity });
    }

    pub fn fail(&self, game_id: i64, stage: Stage, message: impl Into<String>) {
        self.set_activity(
            game_id,
            Activity::Failed {
                message: message.into(),
                stage,
            },
        );
    }

    /// False when the game is neither preparing nor installing.
    pub fn show_progress(&self, game_id: i64, progress: TransferProgress) -> bool {
        let mut slots = write(&self.activity);
        match slots.get_mut(&game_id).map(|slot| &mut slot.activity) {
            Some(
                Activity::Preparing {
                    progress: shown, ..
                }
                | Activity::Installing { progress: shown },
            ) => {
                *shown = Some(progress);
                true
            }
            _ => false,
        }
    }

    pub fn clear_activity(&self, game_id: i64) {
        write(&self.activity).remove(&game_id);
    }

    /// Mean progress of everything that measures itself, for the taskbar.
    pub fn overall_progress(&self) -> Option<f64> {
        let slots = read(&self.activity);
        let percents: Vec<f64> = slots
            .values()
            .filter_map(|slot| slot.activity.percent())
            .collect();
        (!percents.is_empty()).then(|| percents.iter().sum::<f64>() / percents.len() as f64)
    }

    /// Live activity wins over what is on disk. Touches the filesystem.
    pub fn state_of(&self, game_id: i64) -> GameState {
        self.state_from(game_id, self.record(game_id))
    }

    pub fn state_from(&self, game_id: i64, record: GameRecord) -> GameState {
        let activity = read(&self.activity)
            .get(&game_id)
            .map(|slot| slot.activity.clone());
        let Some(activity) = activity else {
            return resting_state(record);
        };

        if record.is_installed() {
            if let Some(busy) = busy_for(&activity) {
                return installed_state(record, Some(busy));
            }
        }
        match activity {
            Activity::Downloading {
                received_bytes,
                total_bytes,
                bytes_per_second,
            } => GameState::Downloading {
                received_bytes,
                total_bytes,
                bytes_per_second,
            },
            Activity::Extracting { percent } => GameState::Extracting { percent },
            Activity::Installing { progress } => GameState::Installing { progress },
            Activity::Preparing { message, progress } => GameState::Preparing { message, progress },
            Activity::Running {
                since,
                executable,
                runtime,
            } => GameState::Running {
                since,
                executable,
                runtime,
            },
            Activity::Failed { message, stage } => GameState::Failed {
                message,
                stage,
                path: record.install_dir.as_deref().map(display),
                executable: record.executable,
                elevation_required: stage == Stage::Install && record.elevation_program.is_some(),
            },
        }
    }

    /// Re-attaches `(id) Title` folders found under a games folder. Returns how many.
    pub async fn rescan(&self, library_root: &Path) -> usize {
        let root = library_root.to_path_buf();
        let found = tokio::task::spawn_blocking(move || scan_root(&root))
            .await
            .unwrap_or_default();
        let count = found.len();
        if count == 0 {
            return 0;
        }
        {
            let mut records = write(&self.records);
            for (game_id, found) in found {
                let record = records.entry(game_id).or_default();
                match found {
                    Found::Installed(dir) => record.install_dir = Some(dir),
                    Found::Download {
                        archive,
                        bytes,
                        staging,
                    } => {
                        record.archive_path = archive;
                        record.archive_bytes = bytes;
                        if let Some((dir, setups)) = staging {
                            record.extracted_dir = Some(dir);
                            record.staging_setups = setups;
                        }
                    }
                }
            }
        }
        self.persist().await;
        count
    }
}

enum Found {
    Installed(PathBuf),
    Download {
        archive: Option<PathBuf>,
        bytes: u64,
        staging: Option<(PathBuf, Vec<String>)>,
    },
}

fn scan_root(root: &Path) -> Vec<(i64, Found)> {
    let layout = gameyfin_core::InstallLayout::new(root);
    let mut found = Vec::new();
    for (dir, installed) in [
        (layout.installs_root(), true),
        (layout.downloads_root(), false),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let Some(game_id) = gameyfin_core::InstallLayout::game_id_from_dir(&path) else {
                continue;
            };
            if installed {
                if has_content(&path) {
                    found.push((game_id, Found::Installed(path)));
                }
                continue;
            }
            let archive = finished_download(&path);
            let bytes = archive
                .as_ref()
                .and_then(|a| std::fs::metadata(a).ok())
                .map_or(0, |m| m.len());
            let staged = path.join(EXTRACT_DIR);
            let staging = staged.is_dir().then(|| {
                let setups = scan_setups(&staged);
                (staged, setups)
            });
            found.push((
                game_id,
                Found::Download {
                    archive,
                    bytes,
                    staging,
                },
            ));
        }
    }
    found
}

fn resting_state(record: GameRecord) -> GameState {
    if record.is_installed() {
        return installed_state(record, None);
    }
    if let Some(dir) = record.existing_staging() {
        return GameState::Extracted {
            path: display(&dir),
            archive_present: record.existing_archive().is_some(),
            setup_candidates: record.setup_candidates,
        };
    }
    match record.existing_archive() {
        Some(archive) => GameState::Downloaded {
            archive_path: display(&archive),
            bytes: record.archive_bytes,
        },
        None => GameState::NotInstalled,
    }
}

/// Downloading and extracting stay Downloads-side work even for an installed game.
fn busy_for(activity: &Activity) -> Option<InstalledBusy> {
    match activity {
        Activity::Preparing { message, progress } => Some(InstalledBusy::Preparing {
            message: message.clone(),
            progress: progress.clone(),
        }),
        Activity::Installing { progress } => Some(InstalledBusy::Installing {
            progress: progress.clone(),
        }),
        Activity::Failed {
            message,
            stage: Stage::Install | Stage::Launch,
        } => Some(InstalledBusy::Failed {
            message: message.clone(),
        }),
        _ => None,
    }
}

fn installed_state(record: GameRecord, busy: Option<InstalledBusy>) -> GameState {
    let staging_present = record.existing_staging().is_some();
    GameState::Installed {
        path: record
            .install_dir
            .as_deref()
            .map(display)
            .unwrap_or_default(),
        executable: record.executable,
        setup_candidates: record.setup_candidates,
        staging_setups: if staging_present {
            record.staging_setups
        } else {
            Vec::new()
        },
        staging_present,
        busy,
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn has_content(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// The archive in a download folder, skipping partial transfers and torrent metainfo.
fn finished_download(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy();
        let partial = name.ends_with(".gameyfin-part")
            || gameyfin_core::Checkpoint::sidecar_path(&path).exists();
        (path.is_file() && !partial && !gameyfin_core::extract::is_torrent_metainfo(&path))
            .then_some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gameyfin-ls-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn downloading(received_bytes: u64, total_bytes: u64) -> Activity {
        Activity::Downloading {
            received_bytes,
            total_bytes,
            bytes_per_second: 0.0,
        }
    }

    fn elevation_offered(state: &LibraryState, game_id: i64) -> bool {
        match state.state_of(game_id) {
            GameState::Failed {
                elevation_required, ..
            } => elevation_required,
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn only_a_refused_install_offers_the_administrator_retry() {
        let state = LibraryState::default();
        state
            .update_record(7, |r| {
                r.elevation_program = Some(PathBuf::from("setup.exe"))
            })
            .await;
        state.fail(7, Stage::Install, "needs administrator");
        assert!(elevation_offered(&state, 7));

        state.fail(7, Stage::Download, "connection reset");
        assert!(!elevation_offered(&state, 7));

        state.fail(8, Stage::Install, "exit code 1");
        assert!(!elevation_offered(&state, 8));
    }

    #[tokio::test]
    async fn work_on_an_installed_game_keeps_it_installed() {
        let dir = scratch("busy");
        std::fs::write(dir.join("game.exe"), b"x").unwrap();
        let state = LibraryState::default();
        state
            .update_record(4, |r| {
                r.install_dir = Some(dir.clone());
                r.executable = Some("game.exe".into());
            })
            .await;

        state.set_activity(4, Activity::preparing("Setting up Wine"));
        assert!(matches!(
            state.state_of(4),
            GameState::Installed {
                busy: Some(InstalledBusy::Preparing { .. }),
                executable: Some(_),
                ..
            }
        ));
        state.set_activity(4, Activity::installing());
        assert!(matches!(
            state.state_of(4),
            GameState::Installed {
                busy: Some(InstalledBusy::Installing { .. }),
                ..
            }
        ));
        state.fail(4, Stage::Install, "exit code 1");
        assert!(matches!(
            state.state_of(4),
            GameState::Installed {
                busy: Some(InstalledBusy::Failed { .. }),
                ..
            }
        ));
        state.set_activity(4, downloading(1, 9));
        assert!(matches!(state.state_of(4), GameState::Downloading { .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn resting_states_follow_what_is_on_disk() {
        let dir = scratch("resting");
        let state = LibraryState::default();
        assert!(matches!(state.state_of(1), GameState::NotInstalled));

        let archive = dir.join("game.zip");
        std::fs::write(&archive, b"x").unwrap();
        state
            .update_record(1, |r| r.archive_path = Some(archive.clone()))
            .await;
        assert!(matches!(state.state_of(1), GameState::Downloaded { .. }));

        let staged = dir.join(EXTRACT_DIR);
        std::fs::create_dir_all(&staged).unwrap();
        state
            .update_record(1, |r| r.extracted_dir = Some(staged.clone()))
            .await;
        assert!(matches!(
            state.state_of(1),
            GameState::Extracted {
                archive_present: true,
                ..
            }
        ));

        let installed = dir.join("installed");
        std::fs::create_dir_all(&installed).unwrap();
        state
            .update_record(1, |r| {
                r.install_dir = Some(installed.clone());
                r.staging_setups = vec!["dlc/setup.exe".into()];
            })
            .await;
        assert!(
            matches!(state.state_of(1), GameState::Extracted { .. }),
            "an empty install folder is not an install"
        );

        std::fs::write(installed.join("game.exe"), b"x").unwrap();
        assert!(matches!(
            state.state_of(1),
            GameState::Installed { staging_present: true, ref staging_setups, .. } if staging_setups.len() == 1
        ));

        std::fs::remove_dir_all(&staged).unwrap();
        assert!(matches!(
            state.state_of(1),
            GameState::Installed { staging_present: false, ref staging_setups, .. } if staging_setups.is_empty()
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_activity_but_a_failure_is_busy() {
        let state = Arc::new(LibraryState::default());
        for activity in [
            downloading(1, 2),
            Activity::Extracting { percent: 1.0 },
            Activity::installing(),
            Activity::preparing("x"),
        ] {
            state.set_activity(5, activity);
            assert!(state.claim(5, Activity::installing()).is_none());
        }
        state.fail(5, Stage::Download, "boom");
        assert!(state.claim(5, Activity::installing()).is_some());
    }

    #[test]
    fn a_dropped_claim_clears_busy_state_but_keeps_a_failure() {
        let state = Arc::new(LibraryState::default());
        drop(state.claim(1, Activity::installing()).unwrap());
        assert!(matches!(state.state_of(1), GameState::NotInstalled));

        let claim = state.claim(1, Activity::installing()).unwrap();
        state.set_activity(1, Activity::preparing("still mine"));
        state.fail(1, Stage::Install, "boom");
        drop(claim);
        assert!(matches!(state.state_of(1), GameState::Failed { .. }));
    }

    #[test]
    fn a_stale_claim_does_not_clear_a_newer_one() {
        let state = Arc::new(LibraryState::default());
        let old = state.claim(1, Activity::installing()).unwrap();
        state.clear_activity(1);
        let _new = state.claim(1, downloading(0, 0)).unwrap();
        drop(old);
        assert!(
            state.claim(1, Activity::installing()).is_none(),
            "the newer claim still holds"
        );
    }

    #[tokio::test]
    async fn records_round_trip_through_disk() {
        let dir = scratch("roundtrip");
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
        assert_eq!(reloaded.record(7).minutes_played, 42);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_install_shows_progress_only_while_it_runs() {
        let state = LibraryState::default();
        let progress = |received_bytes, total_bytes| TransferProgress {
            received_bytes,
            total_bytes,
            bytes_per_second: 1.0,
        };
        state.set_activity(3, Activity::installing());
        assert!(state.show_progress(3, progress(30, 0)));
        assert_eq!(
            state.overall_progress(),
            None,
            "a setup program knows no total"
        );
        assert!(matches!(
            state.state_of(3),
            GameState::Installing { progress: Some(_) }
        ));

        assert!(state.show_progress(3, progress(30, 120)));
        assert_eq!(state.overall_progress(), Some(25.0));

        state.fail(3, Stage::Install, "boom");
        assert!(!state.show_progress(3, progress(60, 120)));
    }

    #[test]
    fn taskbar_progress_averages_what_measures_itself() {
        let state = LibraryState::default();
        assert_eq!(state.overall_progress(), None);
        state.set_activity(1, downloading(25, 100));
        state.set_activity(2, Activity::Extracting { percent: 75.0 });
        state.set_activity(3, Activity::installing());
        assert_eq!(state.overall_progress(), Some(50.0));

        state.set_activity(1, downloading(900, 0));
        assert_eq!(
            state.overall_progress(),
            Some(37.5),
            "unknown size reads as 0%"
        );

        state.clear_activity(1);
        state.clear_activity(2);
        assert_eq!(state.overall_progress(), None);
    }

    #[tokio::test]
    async fn a_rescan_skips_partial_downloads_and_empty_installs() {
        let root = scratch("rescan");
        let layout = gameyfin_core::InstallLayout::new(&root);

        let finished = layout.downloads_dir(1, "Done");
        std::fs::create_dir_all(&finished).unwrap();
        std::fs::write(finished.join("done.zip"), b"zip").unwrap();

        let partial = layout.downloads_dir(2, "Partial");
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("half.zip"), b"zi").unwrap();
        std::fs::write(partial.join("half.zip.gameyfin-part"), b"{}").unwrap();

        std::fs::create_dir_all(layout.install_dir(3, "Empty")).unwrap();

        let state = LibraryState::default();
        assert_eq!(state.rescan(&root).await, 2);
        assert!(state.record(1).archive_path.is_some());
        assert!(state.record(2).archive_path.is_none());
        assert!(state.record(3).install_dir.is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
