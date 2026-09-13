//! Save sync against a real HTTP server, covering the status codes the routes add:
//! 204 for unchanged content, 409 for a conflict, 405 when the feature is off, 413 over quota.

use std::path::PathBuf;
use std::sync::Arc;

use gameyfin_api::saves::{UploadMetadata, UploadOutcome};
use gameyfin_api::{ApiError, DeviceTokenAuth, GameyfinClient};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gameyfin-saves-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("save.zip")
}

fn client(url: &str) -> GameyfinClient {
    GameyfinClient::new(url, Arc::new(DeviceTokenAuth::new("token"))).unwrap()
}

fn version_json(id: i64) -> String {
    format!(
        r#"{{"id":{id},"gameId":42,"gameTitle":"Celeste","sizeBytes":1024,
            "contentHash":"abc","platform":"WINDOWS","locked":false,
            "createdAt":"2026-01-01T00:00:00Z"}}"#
    )
}

fn archive(name: &str) -> (PathBuf, UploadMetadata) {
    let path = scratch(name);
    std::fs::write(&path, b"PK\x03\x04payload").unwrap();
    let metadata = UploadMetadata {
        content_hash: "abc".into(),
        platform: "WINDOWS".into(),
        ..Default::default()
    };
    (path, metadata)
}

#[tokio::test]
async fn lists_versions() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/saves/game/42")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!("[{},{}]", version_json(2), version_json(1)))
        .create_async()
        .await;

    let versions = client(&server.url()).list_saves(42).await.unwrap();

    assert_eq!(
        vec!["2", "1"],
        versions.iter().map(|v| v.id.as_str()).collect::<Vec<_>>()
    );
    assert_eq!("Celeste", versions[0].game_title.as_deref().unwrap());
    mock.assert_async().await;
}

#[tokio::test]
async fn downloads_a_version_to_disk() {
    let payload = b"PK\x03\x04hello world";
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/saves/game/42/7")
        .with_status(200)
        .with_body(payload)
        .create_async()
        .await;
    let destination = scratch("download");

    let written = client(&server.url())
        .download_save(42, "7", &destination)
        .await
        .unwrap();

    assert_eq!(payload.len() as u64, written);
    assert_eq!(payload.to_vec(), std::fs::read(&destination).unwrap());
    mock.assert_async().await;
}

#[tokio::test]
async fn upload_sends_the_declared_hash_and_returns_the_stored_version() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/saves/game/42")
        .match_header("content-type", "application/zip")
        .match_header("x-content-hash", "abc")
        .match_header("x-save-platform", "WINDOWS")
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(version_json(9))
        .create_async()
        .await;
    let (path, metadata) = archive("stored");

    let outcome = client(&server.url())
        .upload_save(42, &path, &metadata)
        .await
        .unwrap();

    match outcome {
        UploadOutcome::Stored(version) => assert_eq!("9", version.id),
        other => panic!("expected a stored version, got {other:?}"),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn identical_content_reports_unchanged() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/saves/game/42")
        .with_status(204)
        .create_async()
        .await;
    let (path, metadata) = archive("unchanged");

    let outcome = client(&server.url())
        .upload_save(42, &path, &metadata)
        .await
        .unwrap();

    assert_eq!(UploadOutcome::Unchanged, outcome);
    mock.assert_async().await;
}

#[tokio::test]
async fn a_stale_base_reports_the_remote_version_rather_than_failing() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/saves/game/42")
        .match_header("x-base-save-id", "3")
        .with_status(409)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"remote":{},"baseSaveId":3}}"#,
            version_json(8)
        ))
        .create_async()
        .await;
    let (path, mut metadata) = archive("conflict");
    metadata.base_save_id = Some("3".into());

    let outcome = client(&server.url())
        .upload_save(42, &path, &metadata)
        .await
        .unwrap();

    match outcome {
        UploadOutcome::Conflict {
            remote,
            base_save_id,
        } => {
            assert_eq!("8", remote.id);
            assert_eq!(Some("3".to_string()), base_save_id);
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    mock.assert_async().await;
}

#[tokio::test]
async fn a_base_from_another_store_is_not_sent() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/saves/game/42")
        .match_header("x-base-save-id", mockito::Matcher::Missing)
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(version_json(11))
        .create_async()
        .await;
    let (path, mut metadata) = archive("foreign-base");
    metadata.base_save_id = Some("1788880138395_da8e4ec6-9d22-4d6b-92f1-f57d20773ab7".into());

    client(&server.url())
        .upload_save(42, &path, &metadata)
        .await
        .unwrap();

    mock.assert_async().await;
}

#[tokio::test]
async fn forcing_an_upload_sets_the_header() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/saves/game/42")
        .match_header("x-force", "true")
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(version_json(10))
        .create_async()
        .await;
    let (path, mut metadata) = archive("forced");
    metadata.force = true;

    client(&server.url())
        .upload_save(42, &path, &metadata)
        .await
        .unwrap();

    mock.assert_async().await;
}

#[tokio::test]
async fn a_server_with_the_feature_off_is_reported_as_disabled() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/saves/game/42")
        .with_status(405)
        .create_async()
        .await;

    let error = client(&server.url()).list_saves(42).await.unwrap_err();

    assert!(matches!(error, ApiError::SaveSyncDisabled), "got {error:?}");
}

#[tokio::test]
async fn a_server_predating_the_feature_is_told_apart_from_one_with_it_switched_off() {
    // A stock server has no route at all and answers 404. The remedy differs from a 405,
    // so the two must not collapse into one error.
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/saves/game/42")
        .with_status(404)
        .create_async()
        .await;

    let error = client(&server.url()).list_saves(42).await.unwrap_err();

    assert!(
        matches!(error, ApiError::SaveSyncUnsupported { .. }),
        "got {error:?}"
    );
}

#[tokio::test]
async fn a_missing_version_is_still_an_ordinary_not_found() {
    // Listing cannot legitimately 404 on a supporting server, but downloading a version
    // that has been pruned can, and that must not read as "the server lacks the feature".
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/saves/game/42/7")
        .with_status(404)
        .create_async()
        .await;
    let destination = scratch("gone");

    let error = client(&server.url())
        .download_save(42, "7", &destination)
        .await
        .unwrap_err();

    assert!(
        !matches!(error, ApiError::SaveSyncUnsupported { .. }),
        "a pruned version should not look like an unsupported server, got {error:?}"
    );
}

#[tokio::test]
async fn an_over_quota_upload_is_reported_as_such() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/saves/game/42")
        .with_status(413)
        .create_async()
        .await;
    let (path, metadata) = archive("quota");

    let error = client(&server.url())
        .upload_save(42, &path, &metadata)
        .await
        .unwrap_err();

    assert!(matches!(error, ApiError::QuotaExceeded), "got {error:?}");
}

#[tokio::test]
async fn a_refused_download_leaves_no_file_behind() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/saves/game/42/7")
        .with_status(403)
        .create_async()
        .await;
    let destination = scratch("forbidden");

    let error = client(&server.url())
        .download_save(42, "7", &destination)
        .await
        .unwrap_err();

    assert!(error.is_auth(), "got {error:?}");
    assert!(!destination.exists());
}

#[tokio::test]
async fn hashing_matches_a_known_digest() {
    let path = scratch("hash");
    std::fs::write(&path, b"gameyfin").unwrap();

    let digest = gameyfin_api::hash_file(&path).await.unwrap();

    assert_eq!(
        "816af297f426f89f0779586c13ce225431c607e71a5499346a0bf8a760e00da0",
        digest
    );
}

#[tokio::test]
async fn lists_every_save_the_user_has_over_hilla() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/connect/SaveSyncEndpoint/getMySaves")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!("[{},{}]", version_json(2), version_json(1)))
        .create_async()
        .await;

    let saves = client(&server.url()).my_saves().await.unwrap();

    // A migration off the server starts here, so an empty answer copied nothing at all.
    assert_eq!(
        vec!["2", "1"],
        saves.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()
    );
    assert!(saves.iter().all(|s| s.game_id == 42));
    mock.assert_async().await;
}

#[tokio::test]
async fn deletes_several_saves_by_their_numeric_ids() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/connect/SaveSyncEndpoint/deleteSaves")
        .match_body(mockito::Matcher::Json(
            serde_json::json!({ "saveIds": [7, 9] }),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("null")
        .create_async()
        .await;

    client(&server.url())
        .delete_saves(&["7", "9"])
        .await
        .unwrap();
    mock.assert_async().await;
}
