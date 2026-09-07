//! Integration tests against the real Ludusavi binary.
//!
//! The unit tests use canned JSON, which proves the parsing is self-consistent but not
//! that it matches what Ludusavi actually prints. These tests close that gap.
//!
//! They are skipped when the bundled binary is absent (`npm run fetch-ludusavi` puts it
//! there), so a checkout without it still passes.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::sync::{Mutex, MutexGuard};

use gameyfin_saves::ludusavi::GameQuery;
use gameyfin_saves::{resolve, GameIdentity, Ludusavi, TitleMatch};

/// Steam AppID for Celeste, a stable, long-standing manifest entry.
const CELESTE_APP_ID: u32 = 504230;

fn binary() -> Option<PathBuf> {
    let triple = if cfg!(windows) {
        "x86_64-pc-windows-msvc.exe"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/binaries")
        .join(format!("ludusavi-{triple}"));
    path.exists().then_some(path)
}

/// One config directory shared by every test, warmed exactly once.
///
/// `--config` relocates the manifest cache as well as the settings, so a per-test
/// directory would download the whole manifest per test, minutes of work and a needless
/// hammering of the manifest host. Keeping it under `target/` also means the manifest
/// survives between runs.
fn shared_config_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

    DIR.get_or_init(|| {
        let binary = binary()?;
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ludusavi-test-config");
        std::fs::create_dir_all(&dir).expect("create shared config dir");

        // Fetch the manifest once, before any test races for it. `--force` is not used:
        // an already-cached manifest should be left alone.
        let status = std::process::Command::new(&binary)
            .args(["--config", &dir.to_string_lossy(), "manifest", "update"])
            .output();
        match status {
            Ok(out) if out.status.success() => Some(dir),
            Ok(out) => {
                eprintln!(
                    "manifest update failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                None
            }
            Err(e) => {
                eprintln!("could not run ludusavi: {e}");
                None
            }
        }
    })
    .as_ref()
}

/// Serialises access to the shared Ludusavi config directory.
///
/// Two Ludusavi processes pointed at one config directory deadlock, verified against
/// 0.31, where a lone `find` returns in under a second and two together hang. The test
/// harness runs tests on parallel threads, so they must take turns.
///
/// An async mutex rather than a blocking one: the guard is held across `await` points
/// while Ludusavi runs, which is precisely what `std::sync::Mutex` must not do.
fn ludusavi_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn exclusive() -> MutexGuard<'static, ()> {
    ludusavi_lock().lock().await
}

fn harness() -> Option<Ludusavi> {
    Some(Ludusavi::new(binary()?, shared_config_dir()?))
}

/// A scratch directory for backup staging, cleaned up on drop.
struct Staging(PathBuf);

impl Staging {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("gameyfin-stage-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create staging dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn find_by_steam_id_returns_the_canonical_title() {
    let Some(lud) = harness() else {
        eprintln!("skipping: bundled ludusavi not present");
        return;
    };
    let _guard = exclusive().await;

    let out = lud.find(&GameQuery::SteamId(CELESTE_APP_ID)).await.unwrap();
    assert!(
        out.games.contains_key("Celeste"),
        "expected Celeste, got {:?}",
        out.games.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn resolve_treats_a_steam_id_match_as_certain() {
    let Some(lud) = harness() else {
        eprintln!("skipping: bundled ludusavi not present");
        return;
    };
    let _guard = exclusive().await;

    let identity = GameIdentity {
        // Deliberately messy, the way a Gameyfin title often is.
        title: "Celeste (2018) [GOG]".into(),
        steam_app_id: Some(CELESTE_APP_ID),
        gog_id: None,
    };

    let matched = resolve(&lud, &identity).await.unwrap();
    assert_eq!(matched, TitleMatch::Certain("Celeste".into()));
}

#[tokio::test]
async fn a_nonexistent_game_resolves_to_nothing() {
    let Some(lud) = harness() else {
        eprintln!("skipping: bundled ludusavi not present");
        return;
    };
    let _guard = exclusive().await;

    let identity = GameIdentity {
        title: "Zzzz Not A Real Game Qqq 12345".into(),
        ..Default::default()
    };
    assert_eq!(resolve(&lud, &identity).await.unwrap(), TitleMatch::None);
}

#[tokio::test]
async fn backing_up_an_unrecognised_game_is_an_error_not_a_silent_no_op() {
    let Some(lud) = harness() else {
        eprintln!("skipping: bundled ludusavi not present");
        return;
    };
    let _guard = exclusive().await;
    let staging = Staging::new("unknown-game");

    // Ludusavi exits 0 here; the driver must still surface it as a failure.
    let err = lud
        .backup(
            "Zzzz Not A Real Game Qqq 12345",
            staging.path(),
            gameyfin_saves::BackupFormat::Zip,
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, gameyfin_saves::SaveError::NoMatch { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn listing_backups_of_an_empty_directory_is_empty_not_an_error() {
    let Some(lud) = harness() else {
        eprintln!("skipping: bundled ludusavi not present");
        return;
    };
    let _guard = exclusive().await;
    let staging = Staging::new("empty-listing");

    let out = lud.backups(staging.path()).await.unwrap();
    assert!(out.games.is_empty());
}
