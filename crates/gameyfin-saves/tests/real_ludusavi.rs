//! Tests against the real Ludusavi binary, since canned JSON cannot prove the parsing matches
//! its output. Skipped when the bundled binary is absent (`npm run fetch-sidecars`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::sync::{Mutex, MutexGuard};

use gameyfin_saves::ludusavi::GameQuery;
use gameyfin_saves::runner::ProcessRunner;
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

/// One config directory shared by every test: `--config` relocates the manifest cache, so a
/// per-test directory would download the whole manifest each time.
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

/// Two Ludusavi processes on one config directory deadlock, so tests take turns. Two `cargo test`
/// runs over this crate still collide. Async, since the guard is held across awaits.
fn ludusavi_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn exclusive() -> MutexGuard<'static, ()> {
    ludusavi_lock().lock().await
}

fn harness() -> Option<Ludusavi> {
    // Far shorter than the app's ten minutes, so a wedged run fails instead of looking hung.
    Some(Ludusavi::with_runner(
        binary()?,
        shared_config_dir()?,
        Box::new(ProcessRunner::with_timeout(std::time::Duration::from_secs(
            60,
        ))),
    ))
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

/// On ostree `/home` links to `/var/home`, and Ludusavi only applies a redirect to a file's real
/// path. Pinned against the binary, since the app works around this behaviour.
#[cfg(unix)]
#[tokio::test]
async fn a_redirect_only_applies_to_the_path_a_file_really_has() {
    let (Some(binary), Some(shared)) = (binary(), shared_config_dir()) else {
        eprintln!("skipping: bundled ludusavi not present");
        return;
    };
    let _guard = exclusive().await;

    let root = std::env::temp_dir().join(format!("gameyfin-real-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let saves = root
        .join("var/home/u/prefix/drive_c/users/steamuser/AppData/LocalLow/TeamSoda/Duckov/Saves");
    std::fs::create_dir_all(&saves).unwrap();
    std::fs::write(saves.join("Save_1.sav"), b"duck").unwrap();
    std::os::unix::fs::symlink(root.join("var/home"), root.join("home")).unwrap();

    let prefix_through_link = root.join("home/u/prefix");
    let home_through_link = prefix_through_link.join("drive_c/users/steamuser");
    let home_as_resolved = std::fs::canonicalize(&home_through_link).unwrap();

    for (name, source, travels) in [
        ("through-link", home_through_link, false),
        ("as-resolved", home_as_resolved, true),
    ] {
        let config = root.join(format!("config-{name}"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::copy(shared.join("manifest.yaml"), config.join("manifest.yaml")).unwrap();
        let staging = Staging::new(name);
        gameyfin_saves::ConfigBuilder::new(staging.path())
            .wine_prefix(&prefix_through_link)
            .portable_home(&source)
            .write(&config)
            .await
            .unwrap();

        let lud = Ludusavi::with_runner(
            &binary,
            &config,
            Box::new(ProcessRunner::with_timeout(std::time::Duration::from_secs(
                60,
            ))),
        )
        .auto_update_manifest(false);
        let scan = lud
            .preview("Escape From Duckov", staging.path())
            .await
            .unwrap();

        let redirected = scan
            .games
            .values()
            .flat_map(|game| game.files.values())
            .any(|file| {
                file.redirected_path
                    .as_deref()
                    .is_some_and(|path| path.starts_with("/gameyfin/home/"))
            });
        assert_eq!(redirected, travels, "redirect written {name}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}
