//! Downloader tests over a real socket, including a server that ignores `Range` and replies
//! `200` with the whole file, as Gameyfin 2.4 does.

use std::io::Write;
use std::path::PathBuf;

use gameyfin_core::checkpoint::{Checkpoint, UnpackPoint};
use gameyfin_core::download::{Cancel, Downloader, Outcome, StartMode};
use gameyfin_core::CoreError;

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
async fn a_zip_file_with_a_length_is_unpacked_too() {
    // A single-file game stored as a zip: served raw, with its size.
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

    assert!(matches!(outcome, Outcome::Unpacked { .. }), "{outcome:?}");
    assert_eq!(
        std::fs::read(dir.join("Game/readme.txt")).unwrap(),
        b"hello"
    );
    assert!(!dest.exists());
}

#[tokio::test]
async fn a_7z_is_saved_as_a_file() {
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

#[tokio::test]
async fn a_download_retried_after_a_cancel_is_still_unpacked() {
    // The partial file asks for a resume, which Gameyfin answers with the whole body again.
    let zip = zip_on_the_fly(&[("Game/readme.txt", b"hello")]);
    let mut server = mockito::Server::new_async().await;
    let served = zip.clone();
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_chunked_body(move |w| w.write_all(&served))
        .create_async()
        .await;

    let dest = scratch("retry");
    let dir = dest.with_file_name("extracted");
    std::fs::write(&dest, &zip[..10]).unwrap();
    let mut cp = Checkpoint::new(Some(zip.len() as u64), None, None);
    cp.received_bytes = 10;
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

    assert!(matches!(outcome, Outcome::Unpacked { .. }), "{outcome:?}");
    assert!(!dest.exists());
}

fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        builder.append_data(&mut header, name, *data).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

#[tokio::test]
async fn a_tar_gz_is_unpacked_while_it_downloads() {
    let archive = tar_gz(&[("Game/game.bin", &body(50_000))]);
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_body(&archive)
        .create_async()
        .await;

    let dest = scratch("tar-gz");
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

    assert!(matches!(outcome, Outcome::Unpacked { .. }), "{outcome:?}");
    assert_eq!(
        std::fs::read(dir.join("Game/game.bin")).unwrap(),
        body(50_000)
    );
}

#[tokio::test]
async fn a_gzip_holding_no_tar_reports_it_cannot_be_unpacked_on_the_way() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&body(4096)).unwrap();
    let payload = encoder.finish().unwrap();
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_body(&payload)
        .create_async()
        .await;

    let dest = scratch("gzip-no-tar");
    let dir = dest.with_file_name("extracted");
    let result = Downloader::new(reqwest::Client::new())
        .download_or_unpack(
            &format!("{}/game", server.url()),
            &dest,
            Some(&dir),
            |r| r,
            |_| {},
        )
        .await;

    assert!(
        matches!(result, Err(gameyfin_core::CoreError::CannotStream(_))),
        "{result:?}"
    );
    assert!(!dir.exists());
}

/// Bytes that barely compress, so a zip of them is as long as they are.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

/// A body sent in small pieces, so a pause lands part way through.
fn slowly(bytes: Vec<u8>) -> impl Fn(&mut dyn std::io::Write) -> std::io::Result<()> {
    move |w| {
        for piece in bytes.chunks(2048) {
            w.write_all(piece)?;
            w.flush()?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(())
    }
}

fn three_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("Game/a.bin", noise(60_000, 1)),
        ("Game/data/b.bin", noise(60_000, 2)),
        ("Game/c.bin", noise(60_000, 3)),
    ]
}

fn zip_of(files: &[(&'static str, Vec<u8>)]) -> Vec<u8> {
    let entries: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    zip_on_the_fly(&entries)
}

#[tokio::test]
async fn a_paused_unpack_resumes_from_the_entry_it_stopped_in() {
    let files = three_files();
    let zip = zip_of(&files);
    let mut server = mockito::Server::new_async().await;
    let _whole = server
        .mock("GET", "/game")
        .match_header("range", mockito::Matcher::Missing)
        .with_status(200)
        .with_header("etag", "\"v1\"")
        .with_header("accept-ranges", "bytes")
        .with_header("content-length", &zip.len().to_string())
        .with_chunked_body(slowly(zip.clone()))
        .create_async()
        .await;

    let dest = scratch("pause-unpack");
    let dir = dest.with_file_name("extracted");
    let url = format!("{}/game", server.url());
    let cancel = Cancel::new();
    let pause_at = zip.len() * 2 / 3;
    let result = Downloader::new(reqwest::Client::new())
        .with_cancel(cancel.clone())
        .download_or_unpack(
            &url,
            &dest,
            Some(&dir),
            |r| r,
            |p| {
                assert!(p.resumable, "an ETag and Accept-Ranges make it resumable");
                if p.received_bytes as usize >= pause_at {
                    cancel.pause();
                }
            },
        )
        .await;

    assert!(matches!(result, Err(CoreError::Paused)), "{result:?}");
    assert!(dir.exists(), "a pause keeps what was unpacked");
    let point = Checkpoint::load(&dest)
        .await
        .and_then(|cp| cp.unpack)
        .expect("the entry to resume from");
    let at = point.next_entry_at as usize;
    assert!(at > 0, "the first entry was finished before the pause");
    assert_eq!(&zip[at..at + 4], b"PK\x03\x04", "it resumes at an entry");

    let resumed = server
        .mock("GET", "/game")
        .match_header("range", format!("bytes={at}-").as_str())
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header(
            "content-range",
            &format!("bytes {at}-{}/{}", zip.len() - 1, zip.len()),
        )
        .with_header("etag", "\"v1\"")
        .with_body(&zip[at..])
        .create_async()
        .await;

    let mut samples = Vec::new();
    let outcome = Downloader::new(reqwest::Client::new())
        .download_or_unpack(&url, &dest, Some(&dir), |r| r, |p| samples.push(p))
        .await
        .unwrap();

    resumed.assert_async().await;
    match outcome {
        Outcome::Unpacked { bytes, .. } => assert_eq!(bytes, 180_000),
        other => panic!("expected an unpacked folder, got {other:?}"),
    }
    for (name, data) in &files {
        assert_eq!(&std::fs::read(dir.join(name)).unwrap(), data, "{name}");
    }
    assert!(Checkpoint::load(&dest).await.is_none());
    assert!(samples[0].received_bytes > at as u64, "progress carries on");
    assert_eq!(samples.last().unwrap().received_bytes, zip.len() as u64);
}

#[tokio::test]
async fn an_unpack_from_an_old_server_starts_over_after_a_pause() {
    // Gameyfin 2.4 sends no ETag, so nothing says the zip is unchanged when asked again.
    let files = three_files();
    let zip = zip_of(&files);
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_chunked_body(slowly(zip.clone()))
        .expect(2)
        .create_async()
        .await;

    let dest = scratch("pause-old-server");
    let dir = dest.with_file_name("extracted");
    let url = format!("{}/game", server.url());
    let cancel = Cancel::new();
    let result = Downloader::new(reqwest::Client::new())
        .with_cancel(cancel.clone())
        .download_or_unpack(
            &url,
            &dest,
            Some(&dir),
            |r| r,
            |p| {
                assert!(!p.resumable);
                if p.received_bytes > 70_000 {
                    cancel.pause();
                }
            },
        )
        .await;
    assert!(matches!(result, Err(CoreError::Paused)), "{result:?}");
    assert!(Checkpoint::load(&dest).await.is_none());

    let outcome = Downloader::new(reqwest::Client::new())
        .download_or_unpack(&url, &dest, Some(&dir), |r| r, |_| {})
        .await
        .unwrap();

    assert!(matches!(outcome, Outcome::Unpacked { bytes: 180_000, .. }));
    for (name, data) in &files {
        assert_eq!(&std::fs::read(dir.join(name)).unwrap(), data, "{name}");
    }
}

#[tokio::test]
async fn a_changed_archive_unpacks_again_from_the_start() {
    let files = three_files();
    let zip = zip_of(&files);
    let mut server = mockito::Server::new_async().await;
    // Answering a stale If-Range as the server does: the whole new file.
    let mock = server
        .mock("GET", "/game")
        .match_header("if-range", "\"v1\"")
        .with_status(200)
        .with_header("etag", "\"v2\"")
        .with_header("accept-ranges", "bytes")
        .with_body(&zip)
        .create_async()
        .await;

    let dest = scratch("changed");
    let dir = dest.with_file_name("extracted");
    std::fs::create_dir_all(dir.join("Game")).unwrap();
    std::fs::write(dir.join("Game/old.bin"), b"from the previous version").unwrap();
    let mut cp = Checkpoint::new(Some(999), Some("\"v1\"".into()), None);
    cp.unpack = Some(UnpackPoint {
        next_entry_at: 500,
        unpacked_bytes: 400,
    });
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

    mock.assert_async().await;
    assert!(matches!(outcome, Outcome::Unpacked { bytes: 180_000, .. }));
    assert!(!dir.join("Game/old.bin").exists(), "the old unpack is gone");
    for (name, data) in &files {
        assert_eq!(&std::fs::read(dir.join(name)).unwrap(), data, "{name}");
    }
}

#[tokio::test]
async fn a_cancelled_unpack_is_removed() {
    let zip = zip_of(&three_files());
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/game")
        .with_status(200)
        .with_header("etag", "\"v1\"")
        .with_header("accept-ranges", "bytes")
        .with_chunked_body(slowly(zip.clone()))
        .create_async()
        .await;

    let dest = scratch("cancel-unpack");
    let dir = dest.with_file_name("extracted");
    let cancel = Cancel::new();
    let result = Downloader::new(reqwest::Client::new())
        .with_cancel(cancel.clone())
        .download_or_unpack(
            &format!("{}/game", server.url()),
            &dest,
            Some(&dir),
            |r| r,
            |p| {
                if p.received_bytes > 70_000 {
                    cancel.cancel();
                }
            },
        )
        .await;

    assert!(matches!(result, Err(CoreError::Cancelled)), "{result:?}");
    assert!(!dir.exists());
    assert!(Checkpoint::load(&dest).await.is_none());
}

#[tokio::test]
async fn a_resume_names_the_version_it_continues() {
    let payload = body(4096);
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/game")
        .match_header("range", "bytes=1024-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", "bytes 1024-4095/4096")
        .with_header("etag", "\"v1\"")
        .with_body(&payload[1024..])
        .create_async()
        .await;

    let dest = scratch("if-range");
    std::fs::write(&dest, &payload[..1024]).unwrap();
    let mut cp = Checkpoint::new(Some(4096), Some("\"v1\"".into()), None);
    cp.received_bytes = 1024;
    cp.save(&dest).await.unwrap();

    let mut resumable = Vec::new();
    let outcome = Downloader::new(reqwest::Client::new())
        .download(
            &format!("{}/game", server.url()),
            &dest,
            |r| r,
            |p| resumable.push(p.resumable),
        )
        .await
        .unwrap();

    mock.assert_async().await;
    assert_eq!(outcome.mode, StartMode::Resumed);
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert!(resumable.iter().all(|r| *r));
}

#[tokio::test]
async fn a_pause_during_a_stall_returns_promptly() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await;
        let header = "HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nETag: \"v1\"\r\nAccept-Ranges: bytes\r\n\r\n";
        socket.write_all(header.as_bytes()).await.unwrap();
        socket.write_all(&[7u8; 1000]).await.unwrap();
        socket.flush().await.unwrap();
        // Then nothing, holding the connection open.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    });

    let dest = scratch("stall");
    let cancel = Cancel::new();
    // Paused from elsewhere while no bytes are arriving, as the Pause button does.
    let pauser = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        pauser.pause();
    });
    let started = std::time::Instant::now();
    let result = Downloader::new(reqwest::Client::new())
        .with_cancel(cancel)
        .download(&format!("http://{addr}/game"), &dest, |r| r, |_| {})
        .await;

    assert!(matches!(result, Err(CoreError::Paused)), "{result:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(std::fs::read(&dest).unwrap(), vec![7u8; 1000]);
    let cp = Checkpoint::load(&dest).await.expect("checkpoint kept");
    assert_eq!(cp.received_bytes, 1000);
    assert_eq!(cp.etag.as_deref(), Some("\"v1\""));
}
