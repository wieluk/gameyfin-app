//! Resumable HTTP downloads. Gameyfin 2.4 ignores `Range`, so only a `206` with a matching
//! `Content-Range` is trusted as a resume; a `200` restarts rather than corrupting. A server
//! that sends an `ETag` and `Accept-Ranges` can also resume a zip being unpacked as it arrives.

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::checkpoint::{Checkpoint, UnpackPoint, FLUSH_INTERVAL};
use crate::error::{CoreError, CoreResult};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub bytes_per_second: f64,
    /// The server can continue from where the transfer stops, so a pause loses nothing.
    pub resumable: bool,
}

impl Progress {
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total_bytes?;
        (total > 0).then(|| (self.received_bytes as f64 / total as f64).clamp(0.0, 1.0))
    }
}

/// How a transfer started, which the caller may want to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    /// No usable partial file; transferring from zero.
    Fresh,
    /// Continued from a checkpoint via `206 Partial Content`.
    Resumed,
    /// A resume was attempted but the server sent the whole file, so it restarted.
    RestartedByServer,
}

#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    pub path: PathBuf,
    pub bytes: u64,
    pub mode: StartMode,
}

/// Decide how to begin, given a stored checkpoint and a probe of the server. Split out so
/// it is testable without a network.
pub fn plan_start(
    checkpoint: Option<&Checkpoint>,
    current_etag: Option<&str>,
    partial_len: u64,
) -> u64 {
    match checkpoint {
        Some(cp) if partial_len > 0 && cp.is_resumable_against(current_etag) => partial_len,
        _ => 0,
    }
}

/// What a stopped download left behind, and so where the next attempt continues from.
struct Leftover {
    checkpoint: Option<Checkpoint>,
    /// Set when a zip unpacked part way can continue from the entry it stopped in.
    unpack_point: Option<UnpackPoint>,
    resume_from: u64,
}

async fn leftover(destination: &Path, unpack_into: Option<&Path>) -> Leftover {
    let partial_len = tokio::fs::metadata(destination)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let mut checkpoint = Checkpoint::load(destination).await;
    let folder_kept = match unpack_into {
        Some(dir) => tokio::fs::try_exists(dir).await.unwrap_or(false),
        None => false,
    };
    // A zip unpacked part way continues from the entry it stopped in, if it can be told unchanged.
    let unpack_point = checkpoint
        .as_ref()
        .filter(|cp| folder_kept && cp.etag.is_some())
        .and_then(|cp| cp.unpack);
    let resume_from = match unpack_point {
        Some(point) => point.next_entry_at,
        None => {
            // The file is written before its sidecar, so it can be ahead after a crash.
            let file_checkpoint = checkpoint
                .as_mut()
                .filter(|cp| cp.unpack.is_none())
                .map(|cp| {
                    cp.reconcile(partial_len);
                    &*cp
                });
            plan_start(
                file_checkpoint,
                file_checkpoint.and_then(|c| c.etag.as_deref()),
                partial_len,
            )
        }
    };
    Leftover {
        checkpoint,
        unpack_point,
        resume_from,
    }
}

/// Where a stopped download will continue from, worked out as the next attempt will.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumePoint {
    pub at: u64,
    /// The server vouched for its bytes with an `ETag`, so the next attempt continues.
    /// `None` when nothing was saved to say.
    pub resumable: Option<bool>,
}

pub async fn resume_point(destination: &Path, unpack_into: Option<&Path>) -> ResumePoint {
    let left = leftover(destination, unpack_into).await;
    ResumePoint {
        at: left.resume_from,
        resumable: left.checkpoint.map(|cp| cp.etag.is_some()),
    }
}

/// Verify a `206` continues from where we asked: a server may answer with a different
/// range, and appending it blindly would corrupt the file.
pub fn content_range_starts_at(header: &str, expected_start: u64) -> bool {
    // Format: `bytes <start>-<end>/<total>`
    let Some(rest) = header.trim().strip_prefix("bytes ") else {
        return false;
    };
    let Some((range, _total)) = rest.split_once('/') else {
        return false;
    };
    let Some((start, _end)) = range.split_once('-') else {
        return false;
    };
    start.trim().parse::<u64>() == Ok(expected_start)
}

/// Filename from a `Content-Disposition` header, preferring the RFC 5987 `filename*` form
/// that carries non-ASCII titles correctly.
pub fn filename_from_disposition(header: &str) -> Option<String> {
    if let Some(idx) = header.find("filename*=") {
        let value = &header[idx + "filename*=".len()..];
        let value = value.split(';').next()?.trim();
        // `UTF-8''<percent-encoded>`
        if let Some((_charset, encoded)) = value.split_once("''") {
            return Some(percent_decode(encoded));
        }
    }

    let idx = header.find("filename=")?;
    let value = &header[idx + "filename=".len()..];
    let value = value.split(';').next()?.trim();
    let value = value.trim_matches('"');
    (!value.is_empty()).then(|| value.to_string())
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Space available on the filesystem holding `path`, when it can be determined.
#[cfg(unix)]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // Walk up to the nearest existing ancestor: the target directory may not exist yet.
    let mut probe = path;
    loop {
        if probe.exists() {
            break;
        }
        probe = probe.parent()?;
    }

    let c_path = CString::new(probe.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c_path` is a valid NUL-terminated string; `stat` is read only after success.
    unsafe {
        let mut stat: libc_statvfs = std::mem::zeroed();
        if statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return None;
        }
        Some(stat.f_bavail.saturating_mul(stat.f_frsize))
    }
}

#[cfg(windows)]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    // Walk up to the nearest existing ancestor: the target directory may not exist yet.
    let mut probe = path;
    loop {
        if probe.exists() {
            break;
        }
        probe = probe.parent()?;
    }

    let wide: Vec<u16> = probe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free_to_caller: u64 = 0;

    // SAFETY: `wide` is NUL-terminated; out-param read only after success, and null is allowed for the totals we skip.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free_to_caller)
}

#[cfg(windows)]
extern "system" {
    fn GetDiskFreeSpaceExW(
        directory: *const u16,
        free_bytes_available_to_caller: *mut u64,
        total_number_of_bytes: *mut u64,
        total_number_of_free_bytes: *mut u64,
    ) -> i32;
}

#[cfg(not(any(unix, windows)))]
pub fn available_space(_path: &Path) -> Option<u64> {
    // Callers treat `None` as "unknown" and skip the preflight rather than blocking.
    None
}

#[cfg(unix)]
#[repr(C)]
#[allow(non_camel_case_types)]
struct libc_statvfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    __spare: [i32; 6],
}

#[cfg(unix)]
extern "C" {
    #[link_name = "statvfs64"]
    fn statvfs(path: *const std::ffi::c_char, buf: *mut libc_statvfs) -> i32;
}

pub fn check_space(target: &Path, needed: u64) -> CoreResult<()> {
    let Some(available) = available_space(target) else {
        // Unknown is not the same as insufficient; let the transfer try.
        return Ok(());
    };
    if available < needed {
        return Err(CoreError::InsufficientSpace {
            path: target.display().to_string(),
            needed,
            available,
        });
    }
    Ok(())
}

/// A speed cap that can be changed while a transfer is running, shared between the settings
/// command and every running download so a change takes effect on the next chunk.
#[derive(Debug, Clone, Default)]
pub struct RateLimit(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl RateLimit {
    /// A limit in bytes per second. Zero means unlimited.
    pub fn new(bytes_per_second: u64) -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
            bytes_per_second,
        )))
    }

    pub fn set(&self, bytes_per_second: u64) {
        self.0
            .store(bytes_per_second, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A cooperative stop signal: the loop flushes and checkpoints on notice, leaving a
/// consistent partial file rather than one truncated mid-write. A pause keeps what arrived;
/// after a cancel the caller discards it.
#[derive(Debug, Clone, Default)]
pub struct Cancel(std::sync::Arc<std::sync::atomic::AtomicU8>);

const RUNNING: u8 = 0;
const CANCELLED: u8 = 1;
const PAUSED: u8 = 2;

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0
            .store(CANCELLED, std::sync::atomic::Ordering::Relaxed);
    }

    /// Does not soften a cancel already asked for.
    pub fn pause(&self) {
        let _ = self.0.compare_exchange(
            RUNNING,
            PAUSED,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Whether the transfer was asked to stop, by a cancel or a pause.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed) != RUNNING
    }

    pub fn is_paused(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed) == PAUSED
    }

    /// The error a stopped transfer ends with.
    pub fn stop_error(&self) -> CoreError {
        if self.is_paused() {
            CoreError::Paused
        } else {
            CoreError::Cancelled
        }
    }
}

/// How often a transfer waiting on a stalled connection looks for a stop.
const STOP_POLL: std::time::Duration = std::time::Duration::from_millis(250);

enum Next<T> {
    Chunk(T),
    End,
    Stopped,
}

/// Paces a transfer to a byte-per-second budget by reading more slowly, letting TCP
/// backpressure genuinely reduce bandwidth rather than merely delay it.
#[derive(Debug)]
struct RateLimiter {
    limit: RateLimit,
    /// The limit the current budget window was measured against, kept so a rate change can be noticed.
    window_limit: u64,
    started: std::time::Instant,
    transferred: u64,
}

const MIN_DELAY: std::time::Duration = std::time::Duration::from_millis(2);

impl RateLimiter {
    /// A limiter with a fixed rate, to keep the pacing arithmetic testable without a shared handle.
    #[cfg(test)]
    fn new(bytes_per_second: u64) -> Self {
        Self::shared(RateLimit::new(bytes_per_second))
    }

    fn shared(limit: RateLimit) -> Self {
        Self {
            window_limit: limit.get(),
            limit,
            started: std::time::Instant::now(),
            transferred: 0,
        }
    }

    /// How long to wait before accepting more, having just taken `bytes`. Delays below
    /// [`MIN_DELAY`] are skipped as not worth the scheduling cost.
    fn delay_after(&mut self, bytes: u64) -> Option<std::time::Duration> {
        let current = self.limit.get();

        // A changed limit restarts the accounting window, else a lowered cap would demand one enormous catch-up sleep.
        if current != self.window_limit {
            self.window_limit = current;
            self.started = std::time::Instant::now();
            self.transferred = 0;
        }

        if current == 0 {
            return None;
        }
        self.transferred += bytes;

        // Measured from the transfer's own start, so a burst is repaid over following chunks.
        let earned = std::time::Duration::from_secs_f64(self.transferred as f64 / current as f64);
        earned
            .checked_sub(self.started.elapsed())
            .filter(|delay| *delay >= MIN_DELAY)
    }
}

/// What a download produced: a file saved as sent, or a folder unpacked while it arrived.
#[derive(Debug, Clone)]
pub enum Outcome {
    File(DownloadOutcome),
    Unpacked { dir: PathBuf, bytes: u64 },
}

/// Enough of a body to see a zip header, or a plain tar's `ustar` marker at byte 257.
const SNIFF_BYTES: usize = 262;

pub struct Downloader {
    http: reqwest::Client,
    /// Shared so the cap can be changed while a transfer is running.
    rate_limit: RateLimit,
    /// Shared so the transfer can be stopped from outside.
    cancel: Cancel,
}

impl Downloader {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            rate_limit: RateLimit::default(),
            cancel: Cancel::default(),
        }
    }

    pub fn with_shared_rate_limit(mut self, limit: RateLimit) -> Self {
        self.rate_limit = limit;
        self
    }

    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    /// Transfer `url` into `destination`, reporting progress through `on_progress`.
    /// `authorize` is a closure so this crate stays independent of how the caller authenticates.
    pub async fn download<F, A>(
        &self,
        url: &str,
        destination: &Path,
        authorize: A,
        on_progress: F,
    ) -> CoreResult<DownloadOutcome>
    where
        F: FnMut(Progress) + Send,
        A: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder + Send + Sync,
    {
        match self
            .download_or_unpack(url, destination, None, authorize, on_progress)
            .await?
        {
            Outcome::File(outcome) => Ok(outcome),
            Outcome::Unpacked { .. } => unreachable!("nothing is unpacked without a folder for it"),
        }
    }

    /// As [`Self::download`], but a zip or tar is unpacked into `unpack_into` as it arrives.
    pub async fn download_or_unpack<F, A>(
        &self,
        url: &str,
        destination: &Path,
        unpack_into: Option<&Path>,
        authorize: A,
        mut on_progress: F,
    ) -> CoreResult<Outcome>
    where
        F: FnMut(Progress) + Send,
        A: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder + Send + Sync,
    {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let Leftover {
            checkpoint,
            unpack_point,
            resume_from,
        } = leftover(destination, unpack_into).await;

        let mut request = authorize(self.http.get(url));
        if resume_from > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
            // A changed file comes back whole instead of being continued.
            if let Some(etag) = checkpoint.as_ref().and_then(|c| c.etag.as_deref()) {
                request = request.header(reqwest::header::IF_RANGE, etag);
            }
        }

        let response = request.send().await?.error_for_status()?;
        let status = response.status();
        let headers = response.headers().clone();

        // A weak validator cannot vouch for a byte range.
        let etag = headers
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.starts_with("W/"))
            .map(str::to_string);
        let accepts_ranges = status == reqwest::StatusCode::PARTIAL_CONTENT
            || headers
                .get(reqwest::header::ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("bytes"));
        let resumable = etag.is_some() && accepts_ranges;
        let filename = headers
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(filename_from_disposition);

        let (mode, mut written) =
            if resume_from > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
                let honoured = headers
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .map(|h| content_range_starts_at(h, resume_from))
                    .unwrap_or(false);
                if honoured {
                    (StartMode::Resumed, resume_from)
                } else {
                    // A 206 for a range we did not ask for: safer to start again.
                    (StartMode::RestartedByServer, 0)
                }
            } else if resume_from > 0 {
                // Whole file despite the Range header: Gameyfin without Range support, or a changed file.
                (StartMode::RestartedByServer, 0)
            } else {
                (StartMode::Fresh, 0)
            };

        // `Content-Length` is the remaining bytes, so a resumed transfer adds the offset back for the true total.
        let total_bytes = response.content_length().map(|len| len + written);

        if let (Some(point), Some(dir), StartMode::Resumed) = (unpack_point, unpack_into, mode) {
            let checkpoint = Checkpoint::new(total_bytes, etag, filename);
            return self
                .unpack(
                    response.bytes_stream(),
                    destination,
                    dir,
                    crate::extract::ArchiveKind::Zip,
                    checkpoint,
                    Some(point),
                    &mut on_progress,
                )
                .await;
        }

        let mut stream = response.bytes_stream();
        let mut head = Vec::new();
        // A resumed transfer continues a file, so only a whole body is unpacked.
        if let Some(dir) = unpack_into.filter(|_| mode != StartMode::Resumed) {
            let mut seen = 0;
            while seen < SNIFF_BYTES {
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        seen += chunk.len();
                        head.push(chunk);
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                }
            }
            let first: Vec<u8> = head
                .iter()
                .flat_map(|chunk| chunk.iter().copied())
                .take(SNIFF_BYTES)
                .collect();
            if let Some(kind) = crate::extract::streamable_kind(&first) {
                let stream = futures_util::stream::iter(head.into_iter().map(Ok)).chain(stream);
                // Only a zip records where it can resume.
                let checkpoint = Checkpoint::new(
                    total_bytes,
                    etag.filter(|_| resumable && kind == crate::extract::ArchiveKind::Zip),
                    filename,
                );
                return self
                    .unpack(
                        stream,
                        destination,
                        dir,
                        kind,
                        checkpoint,
                        None,
                        &mut on_progress,
                    )
                    .await;
            }
        }
        // Whatever was sniffed is the start of the file.
        let mut stream = futures_util::stream::iter(head.into_iter().map(Ok)).chain(stream);

        let mut file = if written > 0 {
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(destination)
                .await?;
            // Discard anything past the resume point; it was never acknowledged.
            f.set_len(written).await?;
            f.seek_end().await?;
            f
        } else {
            tokio::fs::File::create(destination).await?
        };

        let mut cp = Checkpoint::new(total_bytes, etag, filename);
        cp.received_bytes = written;

        let started = std::time::Instant::now();
        let mut last_flush = std::time::Instant::now();
        let baseline = written;

        // Record the transfer before any bytes move, so even an immediate failure can be resumed from.
        cp.save(destination).await?;

        let mut limiter = RateLimiter::shared(self.rate_limit.clone());

        loop {
            let chunk = match self.next_unless_stopped(&mut stream).await {
                Next::Chunk(chunk) => chunk,
                Next::End => break,
                Next::Stopped => {
                    persist(&mut file, &mut cp, destination, written).await;
                    return Err(self.cancel.stop_error());
                }
            };

            // A dropped connection must not lose progress already on disk; the periodic flush can be behind.
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    persist(&mut file, &mut cp, destination, written).await;
                    return Err(e.into());
                }
            };

            if let Err(e) = file.write_all(&chunk).await {
                persist(&mut file, &mut cp, destination, written).await;
                return Err(e.into());
            }
            written += chunk.len() as u64;

            let elapsed = started.elapsed().as_secs_f64();
            let rate = if elapsed > 0.0 {
                (written - baseline) as f64 / elapsed
            } else {
                0.0
            };
            on_progress(Progress {
                received_bytes: written,
                total_bytes,
                bytes_per_second: rate,
                resumable,
            });

            if let Some(delay) = limiter.delay_after(chunk.len() as u64) {
                tokio::time::sleep(delay).await;
            }

            if last_flush.elapsed() >= FLUSH_INTERVAL {
                file.flush().await?;
                cp.received_bytes = written;
                cp.save(destination).await?;
                last_flush = std::time::Instant::now();
            }
        }

        file.flush().await?;
        drop(file);

        if let Some(expected) = total_bytes {
            if written != expected {
                // Leave the partial file and checkpoint so the next attempt can resume.
                cp.received_bytes = written;
                cp.save(destination).await?;
                return Err(CoreError::SizeMismatch {
                    expected,
                    actual: written,
                });
            }
        }

        Checkpoint::clear(destination).await?;

        Ok(Outcome::File(DownloadOutcome {
            path: destination.to_path_buf(),
            bytes: written,
            mode,
        }))
    }

    /// The next chunk, polling for a stop meanwhile so a stalled connection still pauses promptly.
    async fn next_unless_stopped<S: futures_util::Stream + Unpin>(
        &self,
        stream: &mut S,
    ) -> Next<S::Item> {
        loop {
            if self.cancel.is_cancelled() {
                return Next::Stopped;
            }
            match tokio::time::timeout(STOP_POLL, stream.next()).await {
                Ok(Some(item)) => return Next::Chunk(item),
                Ok(None) => return Next::End,
                Err(_) => {}
            }
        }
    }

    /// Unpacks an archive into `dir` as it downloads. A zip with an `ETag` in `checkpoint`
    /// records the entry it is in, so a pause or a dropped connection continues from there.
    /// Anything else that stops removes the folder rather than leaving half a game, except
    /// a pause, whose next attempt starts over.
    #[allow(clippy::too_many_arguments)]
    async fn unpack<S, B, F>(
        &self,
        mut stream: S,
        destination: &Path,
        dir: &Path,
        kind: crate::extract::ArchiveKind,
        mut checkpoint: Checkpoint,
        resume: Option<UnpackPoint>,
        on_progress: &mut F,
    ) -> CoreResult<Outcome>
    where
        S: futures_util::Stream<Item = reqwest::Result<B>> + Unpin,
        B: AsRef<[u8]>,
        F: FnMut(Progress),
    {
        if resume.is_none() {
            // Left by an earlier attempt that was saved as a file.
            let _ = tokio::fs::remove_file(destination).await;
            let _ = Checkpoint::clear(destination).await;
            if tokio::fs::try_exists(dir).await.unwrap_or(false) {
                tokio::fs::remove_dir_all(dir).await?;
            }
        }
        let resumable = checkpoint.etag.is_some();
        let start = resume.unwrap_or_default();
        let mark = crate::zip_stream::ResumeMark::new(start);
        let record = |checkpoint: &mut Checkpoint| {
            let point = mark.get();
            checkpoint.received_bytes = point.next_entry_at;
            checkpoint.unpack = Some(point);
        };

        let (chunks, receiver) = tokio::sync::mpsc::channel::<std::io::Result<Vec<u8>>>(16);
        let unpacker = {
            let (dir, cancel, mark) = (dir.to_path_buf(), self.cancel.clone(), mark.clone());
            tokio::task::spawn_blocking(move || {
                crate::extract::unpack_stream(
                    kind,
                    ChunkReader::new(receiver),
                    &dir,
                    &cancel,
                    &mark,
                )
            })
        };

        // Recorded before any bytes move, so even an immediate failure can be resumed from.
        if resumable {
            record(&mut checkpoint);
            checkpoint.save(destination).await?;
        }

        let started = std::time::Instant::now();
        let mut last_flush = std::time::Instant::now();
        let mut received = start.next_entry_at;
        let mut limiter = RateLimiter::shared(self.rate_limit.clone());
        let pumped: CoreResult<()> = loop {
            let chunk = match self.next_unless_stopped(&mut stream).await {
                Next::Chunk(Ok(chunk)) => chunk.as_ref().to_vec(),
                Next::Chunk(Err(e)) => break Err(e.into()),
                Next::End => break Ok(()),
                Next::Stopped => break Err(self.cancel.stop_error()),
            };
            let len = chunk.len() as u64;
            // A closed channel means the unpacker stopped, and its own error says why.
            if chunks.send(Ok(chunk)).await.is_err() {
                break Ok(());
            }
            received += len;
            let elapsed = started.elapsed().as_secs_f64();
            on_progress(Progress {
                received_bytes: received,
                total_bytes: checkpoint.total_bytes,
                bytes_per_second: if elapsed > 0.0 {
                    (received - start.next_entry_at) as f64 / elapsed
                } else {
                    0.0
                },
                resumable,
            });
            if let Some(delay) = limiter.delay_after(len) {
                tokio::time::sleep(delay).await;
            }
            if resumable && last_flush.elapsed() >= FLUSH_INTERVAL {
                record(&mut checkpoint);
                if let Err(e) = checkpoint.save(destination).await {
                    break Err(e);
                }
                last_flush = std::time::Instant::now();
            }
        };

        if let Err(e) = &pumped {
            let _ = chunks.send(Err(std::io::Error::other(e.to_string()))).await;
        }
        drop(chunks);
        let unpacked = unpacker
            .await
            .map_err(|e| CoreError::Other(format!("unpacking stopped unexpectedly: {e}")))?;

        let result = match (pumped, unpacked) {
            // The unpacker reports a pause as the cancel it saw.
            (Err(_), _) | (Ok(()), Err(_)) if self.cancel.is_cancelled() => {
                Err(self.cancel.stop_error())
            }
            (Err(e), _) | (Ok(()), Err(e)) => Err(e),
            (Ok(()), Ok(bytes)) => Ok(Outcome::Unpacked {
                dir: dir.to_path_buf(),
                bytes,
            }),
        };

        let keep = match &result {
            Ok(_) => {
                Checkpoint::clear(destination).await?;
                return result;
            }
            Err(CoreError::Paused) => true,
            Err(CoreError::Cancelled | CoreError::CannotStream(_)) => false,
            Err(_) => resumable,
        };
        if keep {
            if resumable {
                record(&mut checkpoint);
                if let Err(e) = checkpoint.save(destination).await {
                    tracing::warn!("could not record unpack progress for {destination:?}: {e}");
                }
            }
        } else {
            let _ = Checkpoint::clear(destination).await;
            if let Err(e) = tokio::fs::remove_dir_all(dir).await {
                tracing::warn!("could not remove the partly unpacked {dir:?}: {e}");
            }
        }
        result
    }
}

/// Hands downloaded chunks to the blocking unpacker. A closed channel is the end of the body.
struct ChunkReader {
    chunks: tokio::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    current: Vec<u8>,
    at: usize,
}

impl ChunkReader {
    fn new(chunks: tokio::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>) -> Self {
        Self {
            chunks,
            current: Vec::new(),
            at: 0,
        }
    }
}

impl std::io::Read for ChunkReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.current.len() {
            match self.chunks.blocking_recv() {
                Some(Ok(chunk)) => {
                    self.current = chunk;
                    self.at = 0;
                }
                Some(Err(e)) => return Err(e),
                None => return Ok(0),
            }
        }
        let n = buf.len().min(self.current.len() - self.at);
        buf[..n].copy_from_slice(&self.current[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Flush and record progress on an error path, ignoring secondary failures so they cannot
/// mask the original cause.
async fn persist(
    file: &mut tokio::fs::File,
    cp: &mut Checkpoint,
    destination: &Path,
    written: u64,
) {
    let _ = file.flush().await;
    cp.received_bytes = written;
    if let Err(e) = cp.save(destination).await {
        tracing::warn!("could not record download progress for {destination:?}: {e}");
    }
}

/// Seek helper, kept local so the trait import does not leak into the module surface.
trait SeekEnd {
    async fn seek_end(&mut self) -> std::io::Result<()>;
}

impl SeekEnd for tokio::fs::File {
    async fn seek_end(&mut self) -> std::io::Result<()> {
        use tokio::io::AsyncSeekExt;
        self.seek(std::io::SeekFrom::End(0)).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_limit_takes_effect_on_a_running_transfer() {
        // A download already in flight must pick up a new cap.
        let limit = RateLimit::new(0);
        let mut limiter = RateLimiter::shared(limit.clone());
        assert_eq!(limiter.delay_after(10_000), None, "unlimited to begin with");

        limit.set(1_000);
        // The first chunk after a change opens a fresh window, so it is not itself
        // delayed; the next one is, now that the budget is being measured.
        limiter.delay_after(10_000);
        assert!(
            limiter.delay_after(10_000).is_some(),
            "a newly applied cap must start pacing"
        );

        limit.set(0);
        assert_eq!(
            limiter.delay_after(10_000),
            None,
            "lifting the cap frees it"
        );
    }

    #[test]
    fn lowering_the_limit_does_not_demand_a_catch_up_sleep() {
        // Re-measuring already-transferred bytes against a slower rate would produce one
        // enormous sleep and look like the download had frozen.
        let limit = RateLimit::new(10_000_000);
        let mut limiter = RateLimiter::shared(limit.clone());
        limiter.delay_after(5_000_000);

        limit.set(1_000);
        let delay = limiter.delay_after(1);
        assert!(
            delay.is_none_or(|d| d < std::time::Duration::from_secs(1)),
            "got {delay:?}"
        );
    }

    #[test]
    fn a_cancel_signal_is_visible_through_every_clone() {
        // The download loop holds a clone; cancelling through the registry's copy has to
        // be what that loop sees.
        let cancel = Cancel::new();
        let held_by_download = cancel.clone();
        assert!(!held_by_download.is_cancelled());
        cancel.cancel();
        assert!(held_by_download.is_cancelled());
    }

    #[test]
    fn a_rate_limiter_asks_for_no_delay_when_under_budget() {
        let mut limiter = RateLimiter::new(1_000_000);
        // One byte earns a microsecond of budget, far too little to be worth sleeping
        // for, and a chunk arrives this often.
        assert!(limiter.delay_after(1).is_none());
    }

    #[test]
    fn a_rate_limiter_delays_once_ahead_of_budget() {
        let mut limiter = RateLimiter::new(1_000);
        // Ten seconds' worth of bytes taken at once must be paid back.
        let delay = limiter.delay_after(10_000).expect("a delay");
        assert!(delay.as_secs_f64() > 9.0, "got {delay:?}");
    }

    #[test]
    fn a_zero_limit_never_delays() {
        let mut limiter = RateLimiter::new(0);
        assert!(limiter.delay_after(1_000_000).is_none());
    }

    #[test]
    fn the_limit_is_measured_across_the_whole_transfer() {
        // Budget is cumulative, so a burst is repaid over subsequent chunks rather than
        // each chunk being judged in isolation.
        let mut limiter = RateLimiter::new(1_000);
        let first = limiter.delay_after(2_000).expect("a delay");
        let second = limiter.delay_after(2_000).expect("a delay");
        assert!(
            second > first,
            "budget must accumulate: {first:?} then {second:?}"
        );
    }

    #[test]
    fn content_range_must_match_the_requested_offset() {
        assert!(content_range_starts_at("bytes 100-199/200", 100));
        assert!(!content_range_starts_at("bytes 0-199/200", 100));
        assert!(!content_range_starts_at("garbage", 100));
        assert!(!content_range_starts_at("bytes 100-199", 100));
    }

    #[test]
    fn plan_start_resumes_only_with_a_matching_validator() {
        let mut cp = Checkpoint::new(Some(100), Some("v1".into()), None);
        cp.received_bytes = 40;
        assert_eq!(plan_start(Some(&cp), Some("v1"), 40), 40);
        // ETag changed underneath us.
        assert_eq!(plan_start(Some(&cp), Some("v2"), 40), 0);
        // No checkpoint at all.
        assert_eq!(plan_start(None, None, 40), 0);
        // Checkpoint exists but nothing is on disk.
        assert_eq!(plan_start(Some(&cp), Some("v1"), 0), 0);
    }

    #[test]
    fn parses_a_plain_content_disposition() {
        assert_eq!(
            filename_from_disposition(r#"attachment; filename="Celeste.zip""#).as_deref(),
            Some("Celeste.zip")
        );
    }

    #[test]
    fn prefers_the_rfc5987_encoded_filename() {
        // Gameyfin sends both forms; the encoded one carries non-ASCII titles correctly.
        let header =
            "attachment; filename=\"Game Title.zip\"; filename*=utf-8''Game%E2%84%A2%20Title.zip";
        assert_eq!(
            filename_from_disposition(header).as_deref(),
            Some("Game™ Title.zip")
        );
    }

    #[test]
    fn malformed_disposition_yields_nothing() {
        assert_eq!(filename_from_disposition("attachment"), None);
        assert_eq!(filename_from_disposition(""), None);
    }

    #[test]
    fn available_space_is_reported_for_a_real_directory() {
        let space = available_space(&std::env::temp_dir());
        assert!(space.is_some_and(|s| s > 0), "expected a positive figure");
    }

    #[test]
    fn space_check_rejects_an_impossible_download() {
        let err = check_space(&std::env::temp_dir(), u64::MAX / 2).unwrap_err();
        assert!(matches!(err, CoreError::InsufficientSpace { .. }));
    }

    #[test]
    fn space_check_passes_for_a_small_download() {
        check_space(&std::env::temp_dir(), 1024).unwrap();
    }
}
