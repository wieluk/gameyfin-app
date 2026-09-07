//! Downloader tests against a real HTTP server.
//!
//! The unit tests cover the decision logic in isolation. These exercise the actual
//! transfer loop over a socket, including the case that matters most in practice: a
//! server that ignores `Range` and replies `200` with the whole file, which is exactly
//! what Gameyfin 2.4 does today.

use std::path::PathBuf;

use gameyfin_core::checkpoint::Checkpoint;
use gameyfin_core::download::{Downloader, StartMode};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gameyfin-dl-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("game.bin")
}

fn body(len: usize) -> Vec<u8> {
    // Position-dependent bytes, so a mis-spliced file is detectable rather than
    // accidentally matching.
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[tokio::test]
async fn downloads_a_whole_file() {
    let payload = body(4096);
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_header(
            "content-disposition",
            "attachment; filename=\"Celeste.zip\"",
        )
        .with_body(&payload)
        .create_async()
        .await;

    let dest = scratch("whole");
    let downloader = Downloader::new(reqwest::Client::new());
    let outcome = downloader
        .download(&format!("{}/game", server.url()), &dest, |r| r, |_| {})
        .await
        .unwrap();

    mock.assert_async().await;
    assert_eq!(outcome.mode, StartMode::Fresh);
    assert_eq!(outcome.bytes, payload.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    // The checkpoint is removed once the transfer completes.
    assert!(Checkpoint::load(&dest).await.is_none());
}

#[tokio::test]
async fn resumes_from_a_checkpoint_when_the_server_honours_range() {
    let payload = body(4096);
    let already_have = 1024usize;

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/game")
        .match_header("range", "bytes=1024-")
        .with_status(206)
        .with_header(
            "content-range",
            &format!(
                "bytes {}-{}/{}",
                already_have,
                payload.len() - 1,
                payload.len()
            ),
        )
        .with_header("etag", "\"v1\"")
        .with_body(&payload[already_have..])
        .create_async()
        .await;

    let dest = scratch("resume");
    std::fs::write(&dest, &payload[..already_have]).unwrap();
    let mut cp = Checkpoint::new(Some(payload.len() as u64), Some("\"v1\"".into()), None);
    cp.received_bytes = already_have as u64;
    cp.save(&dest).await.unwrap();

    let downloader = Downloader::new(reqwest::Client::new());
    let outcome = downloader
        .download(&format!("{}/game", server.url()), &dest, |r| r, |_| {})
        .await
        .unwrap();

    mock.assert_async().await;
    assert_eq!(outcome.mode, StartMode::Resumed);
    // The whole file must be byte-identical, not merely the right length.
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
}

#[tokio::test]
async fn restarts_cleanly_when_the_server_ignores_range() {
    // Gameyfin 2.4's behaviour: no Accept-Ranges, so a Range request gets 200 and the
    // entire body. Appending that to the partial file would corrupt it.
    let payload = body(4096);
    let already_have = 1024usize;

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_body(&payload)
        .create_async()
        .await;

    let dest = scratch("ignored-range");
    std::fs::write(&dest, &payload[..already_have]).unwrap();
    let mut cp = Checkpoint::new(Some(payload.len() as u64), None, None);
    cp.received_bytes = already_have as u64;
    cp.save(&dest).await.unwrap();

    let downloader = Downloader::new(reqwest::Client::new());
    let outcome = downloader
        .download(&format!("{}/game", server.url()), &dest, |r| r, |_| {})
        .await
        .unwrap();

    mock.assert_async().await;
    assert_eq!(outcome.mode, StartMode::RestartedByServer);
    assert_eq!(outcome.bytes, payload.len() as u64);
    // Correct content, not 5120 bytes of partial + whole.
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
}

#[tokio::test]
async fn a_206_for_the_wrong_offset_is_not_trusted() {
    let payload = body(2048);
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/game")
        .with_status(206)
        // We asked for bytes=512- but the server answers from 0.
        .with_header(
            "content-range",
            &format!("bytes 0-{}/{}", payload.len() - 1, payload.len()),
        )
        .with_body(&payload)
        .create_async()
        .await;

    let dest = scratch("wrong-offset");
    std::fs::write(&dest, &payload[..512]).unwrap();
    let mut cp = Checkpoint::new(Some(payload.len() as u64), None, None);
    cp.received_bytes = 512;
    cp.save(&dest).await.unwrap();

    let downloader = Downloader::new(reqwest::Client::new());
    let outcome = downloader
        .download(&format!("{}/game", server.url()), &dest, |r| r, |_| {})
        .await
        .unwrap();

    mock.assert_async().await;
    assert_eq!(outcome.mode, StartMode::RestartedByServer);
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
}

#[tokio::test]
async fn a_truncated_transfer_keeps_the_checkpoint_for_a_later_resume() {
    // A mock server will not send fewer bytes than it advertises, so this speaks HTTP
    // directly: declare 8192 bytes, send 4096, then close. That is what a dropped
    // connection mid-download actually looks like to the client.
    let payload = body(4096);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let served = payload.clone();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Read and discard the request head.
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await;

        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
            served.len() * 2
        );
        socket.write_all(header.as_bytes()).await.unwrap();
        socket.write_all(&served).await.unwrap();
        socket.flush().await.unwrap();
        // Close early, leaving the body short of what was promised.
    });

    let dest = scratch("truncated");
    let downloader = Downloader::new(reqwest::Client::new());
    let result = downloader
        .download(&format!("http://{addr}/game"), &dest, |r| r, |_| {})
        .await;

    assert!(
        result.is_err(),
        "a short transfer must not be reported as success"
    );
    // The partial file and its checkpoint survive, so the next attempt can continue.
    let checkpoint = Checkpoint::load(&dest).await.expect("checkpoint retained");
    assert!(checkpoint.received_bytes > 0);
}

#[tokio::test]
async fn progress_is_reported_and_reaches_the_total() {
    let payload = body(64 * 1024);
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_body(&payload)
        .create_async()
        .await;

    let dest = scratch("progress");
    let mut samples = Vec::new();
    let downloader = Downloader::new(reqwest::Client::new());
    downloader
        .download(
            &format!("{}/game", server.url()),
            &dest,
            |r| r,
            |p| samples.push(p),
        )
        .await
        .unwrap();

    assert!(!samples.is_empty(), "expected progress callbacks");
    let last = samples.last().unwrap();
    assert_eq!(last.received_bytes, payload.len() as u64);
    assert_eq!(last.fraction(), Some(1.0));
}

#[tokio::test]
async fn authorization_closure_is_applied() {
    let payload = body(16);
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/game")
        .match_header("authorization", "Bearer secret")
        .with_status(200)
        .with_body(&payload)
        .create_async()
        .await;

    let dest = scratch("auth");
    let downloader = Downloader::new(reqwest::Client::new());
    downloader
        .download(
            &format!("{}/game", server.url()),
            &dest,
            |r| r.bearer_auth("secret"),
            |_| {},
        )
        .await
        .unwrap();

    mock.assert_async().await;
}
