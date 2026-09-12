//! The WebDAV store against a real HTTP server, since its correctness is mostly about
//! which verbs go where and how a multi-status body is read.

use std::path::PathBuf;

use gameyfin_api::saves::{UploadMetadata, UploadOutcome};
use gameyfin_core::save_store::SaveStore;
use gameyfin_core::WebDavStore;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gameyfin-dav-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn store(url: &str) -> WebDavStore {
    WebDavStore::new(
        url.to_string(),
        Some("alice".into()),
        Some("secret".into()),
        reqwest::Client::new(),
        5,
    )
}

fn multistatus(names: &[&str]) -> String {
    let entries: String = names
        .iter()
        .map(|n| format!("<d:response><d:href>/dav/42/{n}</d:href></d:response>"))
        .collect();
    format!(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">{entries}</d:multistatus>"#)
}

fn sidecar(id: &str, hash: &str) -> String {
    format!(
        r#"{{"id":"{id}","gameId":42,"sizeBytes":10,"contentHash":"{hash}",
            "platform":"WINDOWS","locked":false,"createdAt":"2026-01-01T00:00:00Z"}}"#
    )
}

fn meta(hash: &str, base: Option<&str>, force: bool) -> UploadMetadata {
    UploadMetadata {
        content_hash: hash.into(),
        platform: "WINDOWS".into(),
        installation_id: Some("device-a".into()),
        device_name: Some("desktop".into()),
        ludusavi_title: None,
        base_save_id: base.map(str::to_string),
        force,
    }
}

#[tokio::test]
async fn a_missing_collection_lists_nothing() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("PROPFIND", "/42")
        .with_status(404)
        .create_async()
        .await;

    assert!(store(&server.url()).list(42).await.unwrap().is_empty());
}

#[tokio::test]
async fn versions_are_read_from_their_sidecars_newest_first() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("PROPFIND", "/42")
        .with_status(207)
        .with_body(multistatus(&[
            "",
            "0000000000001_device-a.zip",
            "0000000000001_device-a.json",
            "0000000000002_device-b.zip",
            "0000000000002_device-b.json",
        ]))
        .create_async()
        .await;
    server
        .mock("GET", "/42/0000000000001_device-a.json")
        .with_body(sidecar("0000000000001_device-a", "older"))
        .create_async()
        .await;
    server
        .mock("GET", "/42/0000000000002_device-b.json")
        .with_body(sidecar("0000000000002_device-b", "newer"))
        .create_async()
        .await;

    let versions = store(&server.url()).list(42).await.unwrap();

    assert_eq!(2, versions.len());
    assert_eq!("newer", versions[0].content_hash, "newest first");
    assert_eq!("older", versions[1].content_hash);
}

#[tokio::test]
async fn credentials_are_sent() {
    let mut server = mockito::Server::new_async().await;
    // "alice:secret" base64 encoded.
    let mock = server
        .mock("PROPFIND", "/42")
        .match_header("authorization", "Basic YWxpY2U6c2VjcmV0")
        .with_status(207)
        .with_body(multistatus(&[""]))
        .create_async()
        .await;

    store(&server.url()).list(42).await.unwrap();

    mock.assert_async().await;
}

#[tokio::test]
async fn an_unauthorised_share_is_reported_as_such() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("PROPFIND", "/42")
        .with_status(401)
        .create_async()
        .await;

    let error = store(&server.url()).list(42).await.unwrap_err();

    assert!(error.is_auth(), "got {error:?}");
}

#[tokio::test]
async fn identical_content_is_not_uploaded_again() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("PROPFIND", "/42")
        .with_status(207)
        .with_body(multistatus(&["0000000000001_device-a.json"]))
        .create_async()
        .await;
    server
        .mock("GET", "/42/0000000000001_device-a.json")
        .with_body(sidecar("0000000000001_device-a", "same"))
        .create_async()
        .await;
    let put = server
        .mock("PUT", mockito::Matcher::Any)
        .expect(0)
        .create_async()
        .await;

    let dir = scratch("same");
    let file = dir.join("a.zip");
    std::fs::write(&file, b"PK\x03\x04").unwrap();

    let outcome = store(&server.url())
        .upload(
            42,
            &file,
            &meta("same", Some("0000000000001_device-a"), false),
        )
        .await
        .unwrap();

    assert_eq!(UploadOutcome::Unchanged, outcome);
    put.assert_async().await;
}

#[tokio::test]
async fn a_stale_base_is_a_conflict_and_writes_nothing() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("PROPFIND", "/42")
        .with_status(207)
        .with_body(multistatus(&["0000000000009_device-b.json"]))
        .create_async()
        .await;
    server
        .mock("GET", "/42/0000000000009_device-b.json")
        .with_body(sidecar("0000000000009_device-b", "theirs"))
        .create_async()
        .await;
    let put = server
        .mock("PUT", mockito::Matcher::Any)
        .expect(0)
        .create_async()
        .await;

    let dir = scratch("conflict");
    let file = dir.join("a.zip");
    std::fs::write(&file, b"PK\x03\x04").unwrap();

    let outcome = store(&server.url())
        .upload(
            42,
            &file,
            &meta("mine", Some("0000000000001_device-a"), false),
        )
        .await
        .unwrap();

    assert!(
        matches!(outcome, UploadOutcome::Conflict { .. }),
        "got {outcome:?}"
    );
    put.assert_async().await;
}

#[tokio::test]
async fn an_upload_writes_the_archive_then_the_sidecar() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("PROPFIND", "/42")
        .with_status(207)
        .with_body(multistatus(&[""]))
        .create_async()
        .await;
    server
        .mock("MKCOL", "/42")
        .with_status(201)
        .create_async()
        .await;
    let puts = server
        .mock(
            "PUT",
            mockito::Matcher::Regex(r"^/42/\d{13}_device-a\.(zip|json)$".into()),
        )
        .with_status(201)
        .expect(2)
        .create_async()
        .await;

    let dir = scratch("upload");
    let file = dir.join("a.zip");
    std::fs::write(&file, b"PK\x03\x04payload").unwrap();

    let outcome = store(&server.url())
        .upload(42, &file, &meta("fresh", None, false))
        .await
        .unwrap();

    assert!(
        matches!(outcome, UploadOutcome::Stored(_)),
        "got {outcome:?}"
    );
    puts.assert_async().await;
}

#[tokio::test]
async fn fetching_writes_the_archive_to_disk() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("GET", "/42/0000000000001_device-a.zip")
        .with_body(b"PK\x03\x04payload")
        .create_async()
        .await;

    let dir = scratch("fetch");
    let destination = dir.join("out.zip");
    store(&server.url())
        .fetch(42, "0000000000001_device-a", &destination)
        .await
        .unwrap();

    assert_eq!(
        b"PK\x03\x04payload".to_vec(),
        std::fs::read(&destination).unwrap()
    );
}

#[tokio::test]
async fn games_are_the_numeric_collections_at_the_root() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("PROPFIND", "/")
        .with_status(207)
        .with_body(
            r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">
                 <d:response><d:href>/dav/</d:href></d:response>
                 <d:response><d:href>/dav/42/</d:href></d:response>
                 <d:response><d:href>/dav/7/</d:href></d:response>
                 <d:response><d:href>/dav/notes.txt</d:href></d:response>
               </d:multistatus>"#,
        )
        .create_async()
        .await;

    let games = store(&server.url()).games().await.unwrap();

    assert_eq!(vec![7, 42], games, "numeric collections only, sorted");
}
