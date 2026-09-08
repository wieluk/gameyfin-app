//! Migrating between two folder stores, which is the case a user can actually check.

use std::path::{Path, PathBuf};

use gameyfin_api::saves::{UploadMetadata, UploadOutcome};
use gameyfin_core::save_migration::migrate;
use gameyfin_core::save_store::{FolderStore, SaveStore};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gameyfin-migrate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn meta(hash: &str) -> UploadMetadata {
    UploadMetadata {
        content_hash: hash.into(),
        platform: "WINDOWS".into(),
        installation_id: Some("device-a".into()),
        device_name: Some("desktop".into()),
        ludusavi_title: Some("Celeste".into()),
        base_save_id: None,
        force: true,
    }
}

/// Uploads one version and returns its id, waiting out the per-millisecond id granularity.
async fn put(store: &FolderStore, dir: &Path, game_id: i64, body: &[u8], hash: &str) -> String {
    let file = dir.join("upload.zip");
    std::fs::write(&file, body).unwrap();
    let outcome = store.upload(game_id, &file, &meta(hash)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    match outcome {
        UploadOutcome::Stored(v) => v.id,
        other => panic!("expected a stored version, got {other:?}"),
    }
}

#[tokio::test]
async fn only_the_newest_version_moves_by_default() {
    let dir = scratch("newest");
    let source = FolderStore::new(dir.join("from"), 10);
    let destination = FolderStore::new(dir.join("to"), 10);

    put(&source, &dir, 42, b"PK\x03\x04old", "aaa").await;
    put(&source, &dir, 42, b"PK\x03\x04new", "bbb").await;

    let summary = migrate(&source, &destination, &dir.join("tmp"), false, |_, _| {})
        .await
        .unwrap();

    assert_eq!(1, summary.games);
    assert_eq!(1, summary.copied);
    assert_eq!(0, summary.failed);

    let moved = destination.list(42).await.unwrap();
    assert_eq!(1, moved.len());
    assert_eq!("bbb", moved[0].content_hash);
}

#[tokio::test]
async fn the_full_history_moves_when_asked_newest_last() {
    let dir = scratch("history");
    let source = FolderStore::new(dir.join("from"), 10);
    let destination = FolderStore::new(dir.join("to"), 10);

    put(&source, &dir, 42, b"PK\x03\x04old", "aaa").await;
    put(&source, &dir, 42, b"PK\x03\x04new", "bbb").await;

    let summary = migrate(&source, &destination, &dir.join("tmp"), true, |_, _| {})
        .await
        .unwrap();

    assert_eq!(2, summary.copied);
    // Newest first at the destination too, so the order survived the copy.
    let hashes: Vec<_> = destination
        .list(42)
        .await
        .unwrap()
        .into_iter()
        .map(|v| v.content_hash)
        .collect();
    assert_eq!(vec!["bbb".to_string(), "aaa".to_string()], hashes);
}

#[tokio::test]
async fn nothing_is_removed_from_the_source() {
    let dir = scratch("intact");
    let source = FolderStore::new(dir.join("from"), 10);
    let destination = FolderStore::new(dir.join("to"), 10);
    put(&source, &dir, 42, b"PK\x03\x04only", "aaa").await;

    migrate(&source, &destination, &dir.join("tmp"), true, |_, _| {})
        .await
        .unwrap();

    assert_eq!(1, source.list(42).await.unwrap().len());
    assert_eq!(vec![42], source.games().await.unwrap());
}

#[tokio::test]
async fn running_it_twice_copies_nothing_the_second_time() {
    let dir = scratch("twice");
    let source = FolderStore::new(dir.join("from"), 10);
    let destination = FolderStore::new(dir.join("to"), 10);
    put(&source, &dir, 42, b"PK\x03\x04only", "aaa").await;

    migrate(&source, &destination, &dir.join("tmp"), true, |_, _| {})
        .await
        .unwrap();
    let again = migrate(&source, &destination, &dir.join("tmp"), true, |_, _| {})
        .await
        .unwrap();

    assert_eq!(0, again.copied);
    assert_eq!(1, again.skipped);
    assert_eq!(1, destination.list(42).await.unwrap().len());
}

#[tokio::test]
async fn every_game_is_visited_and_reported() {
    let dir = scratch("many");
    let source = FolderStore::new(dir.join("from"), 10);
    let destination = FolderStore::new(dir.join("to"), 10);
    put(&source, &dir, 1, b"PK\x03\x04a", "aaa").await;
    put(&source, &dir, 2, b"PK\x03\x04b", "bbb").await;

    let mut seen = 0;
    let summary = migrate(
        &source,
        &destination,
        &dir.join("tmp"),
        false,
        |_, total| {
            seen = total;
        },
    )
    .await
    .unwrap();

    assert_eq!(2, seen);
    assert_eq!(2, summary.games);
    assert_eq!(2, summary.copied);
    assert_eq!(vec![1, 2], destination.games().await.unwrap());
}

#[tokio::test]
async fn an_empty_source_is_a_successful_no_op() {
    let dir = scratch("empty");
    let source = FolderStore::new(dir.join("from"), 10);
    let destination = FolderStore::new(dir.join("to"), 10);

    let summary = migrate(&source, &destination, &dir.join("tmp"), true, |_, _| {})
        .await
        .unwrap();

    assert_eq!(0, summary.games);
    assert_eq!(0, summary.copied);
    assert!(summary.problems.is_empty());
}
