//! Downloader tests over a real socket, including a server that ignores `Range` and replies
//! `200` with the whole file, as Gameyfin 2.4 does.

use std::io::Write;
use std::path::PathBuf;

use gameyfin_core::checkpoint::Checkpoint;
use gameyfin_core::download::{Downloader, Outcome, StartMode};

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
    // Raw HTTP, since a mock will not send fewer bytes than it advertises: declare 8192, send
    // 4096, close.
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

/// A zip as Gameyfin builds one for a folder game: deflated, each entry followed by a data
/// descriptor, and no length for the whole.
fn zip_on_the_fly(entries: &[(&str, &[u8])]) -> Vec<u8> {
    fn put16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    fn put32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        let packed = encoder.finish().unwrap();
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let sizes = [crc.sum(), packed.len() as u32, data.len() as u32];

        put32(&mut out, 0x0403_4b50);
        put16(&mut out, 20);
        put16(&mut out, 0x0808);
        put16(&mut out, 8);
        for _ in 0..4 {
            put32(&mut out, 0);
        }
        put16(&mut out, name.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&packed);
        put32(&mut out, 0x0807_4b50);
        for value in sizes {
            put32(&mut out, value);
        }

        put32(&mut central, 0x0201_4b50);
        put16(&mut central, 20);
        put16(&mut central, 20);
        put16(&mut central, 0x0808);
        put16(&mut central, 8);
        put32(&mut central, 0);
        for value in sizes {
            put32(&mut central, value);
        }
        put16(&mut central, name.len() as u16);
        for _ in 0..4 {
            put16(&mut central, 0);
        }
        put32(&mut central, 0);
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let (at, size, count) = (out.len() as u32, central.len() as u32, entries.len() as u16);
    out.extend_from_slice(&central);
    put32(&mut out, 0x0605_4b50);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, count);
    put16(&mut out, count);
    put32(&mut out, size);
    put32(&mut out, at);
    put16(&mut out, 0);
    out
}

#[tokio::test]
async fn a_folder_zip_without_a_length_is_unpacked_while_it_downloads() {
    let game = body(200_000);
    let zip = zip_on_the_fly(&[("Game/game.bin", &game), ("Game/readme.txt", b"hello")]);
    let mut server = mockito::Server::new_async().await;
    let served = zip.clone();
    let mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_chunked_body(move |w| w.write_all(&served))
        .create_async()
        .await;

    let dest = scratch("unpack");
    let dir = dest.with_file_name("extracted");
    let mut samples = Vec::new();
    let outcome = Downloader::new(reqwest::Client::new())
        .download_or_unpack(
            &format!("{}/game", server.url()),
            &dest,
            Some(&dir),
            |r| r,
            |p| samples.push(p),
        )
        .await
        .unwrap();

    mock.assert_async().await;
    match outcome {
        Outcome::Unpacked {
            dir: unpacked,
            bytes,
        } => {
            assert_eq!(unpacked, dir);
            assert_eq!(bytes, game.len() as u64 + 5);
        }
        other => panic!("expected an unpacked folder, got {other:?}"),
    }
    assert_eq!(std::fs::read(dir.join("Game/game.bin")).unwrap(), game);
    assert_eq!(
        std::fs::read(dir.join("Game/readme.txt")).unwrap(),
        b"hello"
    );
    assert!(!dest.exists(), "nothing is saved as a file");
    assert!(!samples.is_empty(), "expected progress callbacks");
}

#[tokio::test]
async fn the_same_zip_with_a_length_is_saved_as_a_file() {
    // A single-file game that happens to be a zip: served raw, with its size.
    let zip = zip_on_the_fly(&[("Game/readme.txt", b"hello")]);
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_body(&zip)
        .create_async()
        .await;

    let dest = scratch("sized-zip");
    let dir = dest.with_file_name("extracted");
    let outcome = Downloader::new(reqwest::Client::new())
        .download_or_unpack(
            &format!("{}/game", server.url()),
            &dest,
            Some(&dir),
            |r| r,
            |_| {},
        )
        .await
        .unwrap();

    match outcome {
        Outcome::File(file) => assert_eq!(std::fs::read(file.path).unwrap(), zip),
        other => panic!("expected a saved file, got {other:?}"),
    }
    assert!(!dir.exists());
}

#[tokio::test]
async fn another_body_without_a_length_is_saved_as_a_file() {
    let mut payload = vec![0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
    payload.extend(body(4096));
    let mut server = mockito::Server::new_async().await;
    let served = payload.clone();
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_chunked_body(move |w| w.write_all(&served))
        .create_async()
        .await;

    let dest = scratch("chunked-7z");
    let dir = dest.with_file_name("extracted");
    let outcome = Downloader::new(reqwest::Client::new())
        .download_or_unpack(
            &format!("{}/game", server.url()),
            &dest,
            Some(&dir),
            |r| r,
            |_| {},
        )
        .await
        .unwrap();

    assert!(matches!(outcome, Outcome::File(_)), "{outcome:?}");
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert!(!dir.exists());
}

#[tokio::test]
async fn a_resumed_download_is_never_unpacked() {
    let payload = zip_on_the_fly(&[("Game/game.bin", &body(8192))]);
    // Half of whatever the zip compressed to.
    let already_have = payload.len() / 2;
    let mut server = mockito::Server::new_async().await;
    let served = payload[already_have..].to_vec();
    let _mock = server
        .mock("GET", "/game")
        .match_header("range", format!("bytes={already_have}-").as_str())
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
        // Chunked, so only the resume keeps it from being unpacked.
        .with_chunked_body(move |w| w.write_all(&served))
        .create_async()
        .await;

    let dest = scratch("resume-zip");
    let dir = dest.with_file_name("extracted");
    std::fs::write(&dest, &payload[..already_have]).unwrap();
    let mut cp = Checkpoint::new(Some(payload.len() as u64), Some("\"v1\"".into()), None);
    cp.received_bytes = already_have as u64;
    cp.save(&dest).await.unwrap();

    let outcome = Downloader::new(reqwest::Client::new())
        .download_or_unpack(
            &format!("{}/game", server.url()),
            &dest,
            Some(&dir),
            |r| r,
            |_| {},
        )
        .await
        .unwrap();

    match outcome {
        Outcome::File(file) => assert_eq!(file.mode, StartMode::Resumed),
        other => panic!("expected a saved file, got {other:?}"),
    }
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert!(!dir.exists());
}
