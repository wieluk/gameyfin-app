//! Resumable HTTP downloads.
//!
//! ## Resume on a server that does not support it
//!
//! Gameyfin 2.4's download endpoint streams a `StreamingResponseBody` with no
//! `Accept-Ranges` header and no `Range` handling, so a resumed request is answered with
//! `200 OK` and the *whole* file. Appending that to a partial file would silently corrupt
//! it, the file would end up the right size only by coincidence, and wrong everywhere.
//!
//! So the status code is treated as the contract: only `206 Partial Content` with a
//! matching `Content-Range` is accepted as a resume. A `200` means "start over", which is
//! handled by truncating and restarting rather than by producing a broken file.
//! This keeps the client correct today and makes it faster the moment the server gains
//! `Range` support, with no client change.

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::checkpoint::{Checkpoint, FLUSH_INTERVAL};
use crate::error::{CoreError, CoreResult};

/// Progress of an in-flight transfer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub bytes_per_second: f64,
}

impl Progress {
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total_bytes?;
        (total > 0).then(|| (self.received_bytes as f64 / total as f64).clamp(0.0, 1.0))
    }

    /// Seconds remaining, or `None` when it cannot be estimated.
    pub fn eta_seconds(&self) -> Option<u64> {
        let total = self.total_bytes?;
        if self.bytes_per_second <= 0.0 || total <= self.received_bytes {
            return None;
        }
        Some(((total - self.received_bytes) as f64 / self.bytes_per_second).round() as u64)
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

/// Decide how to begin, given a stored checkpoint and a probe of the server.
///
/// Split out from the transfer loop so the decision is testable without a network.
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

/// Verify that a `206` response actually continues from where we asked.
///
/// A server may legitimately answer a range request with a *different* range than
/// requested; appending it blindly would corrupt the file.
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

/// Filename from a `Content-Disposition` header.
///
/// Gameyfin emits both an ASCII fallback and an RFC 5987 `filename*`, so the encoded form
/// is preferred when present, that is the one carrying non-ASCII titles correctly.
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
    // SAFETY: `c_path` is a valid NUL-terminated string and `stat` is only read after a
    // successful call.
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

    // SAFETY: `wide` is NUL-terminated, and the out-parameter is only read after the call
    // reports success. The two totals we do not need are passed as null, which the API
    // documents as allowed.
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

/// Refuse a download that obviously cannot fit.
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

/// A speed cap that can be changed while a transfer is running.
///
/// The limit used to be captured when a download started, so changing it did nothing
/// until the next one, which is not what a speed control means when a user drags it
/// during an eight-gigabyte transfer. Sharing one atomic between the settings command and
/// every running download makes a change take effect on the next chunk.
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

/// A stop signal for a running transfer.
///
/// Cooperative rather than an abort: the loop notices, flushes what it has and saves its
/// checkpoint, leaving a consistent partial file rather than one truncated mid-write.
///
/// Whether restarting then resumes depends on the server. Gameyfin 2.4 does not implement
/// `Range`, so it answers with the whole file and the transfer starts over; the checkpoint
/// is what makes resuming work the moment the server supports it.
#[derive(Debug, Clone, Default)]
pub struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Paces a transfer to a byte-per-second budget.
///
/// Gameyfin has no server-side throttle, so the limit is applied by reading more slowly:
/// the socket's receive window closes behind us and TCP backpressure does the rest, which
/// genuinely reduces bandwidth rather than merely delaying it.
#[derive(Debug)]
struct RateLimiter {
    limit: RateLimit,
    /// The limit the current budget window was measured against.
    ///
    /// Kept so a change can be noticed: the budget is "bytes since `started`", which is
    /// meaningless once the rate it was computed from has moved.
    window_limit: u64,
    started: std::time::Instant,
    transferred: u64,
}

/// Shortest pause worth taking.
const MIN_DELAY: std::time::Duration = std::time::Duration::from_millis(2);

impl RateLimiter {
    /// A limiter with a fixed rate. Only the shared form is used in practice; this keeps
    /// the pacing arithmetic testable without a shared handle.
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

    /// How long to wait before accepting more, having just taken `bytes`.
    ///
    /// Delays below [`MIN_DELAY`] are skipped: a chunk arrives every few hundred
    /// microseconds, and sleeping for less than a timer tick each time costs more in
    /// scheduling than it saves in bandwidth.
    fn delay_after(&mut self, bytes: u64) -> Option<std::time::Duration> {
        let current = self.limit.get();

        // A changed limit restarts the accounting window. Without this, lowering the cap
        // mid-transfer would re-measure every byte already moved against the new, slower
        // rate and demand one enormous catch-up sleep; raising it would hand out a burst
        // of free bandwidth for the same reason.
        if current != self.window_limit {
            self.window_limit = current;
            self.started = std::time::Instant::now();
            self.transferred = 0;
        }

        if current == 0 {
            return None;
        }
        self.transferred += bytes;

        // Where the transfer *should* be by now, measured from its own start rather than
        // per chunk, so a burst is repaid over the following chunks instead of being
        // smoothed away.
        let earned = std::time::Duration::from_secs_f64(self.transferred as f64 / current as f64);
        earned
            .checked_sub(self.started.elapsed())
            .filter(|delay| *delay >= MIN_DELAY)
    }
}

/// Downloads a single file, resuming when the server allows it.
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

    /// Cap the transfer rate for this downloader alone. Zero means no limit.
    pub fn with_rate_limit(mut self, bytes_per_second: u64) -> Self {
        self.rate_limit = RateLimit::new(bytes_per_second);
        self
    }

    /// Take the cap from a shared handle, so changing it affects a transfer in flight.
    pub fn with_shared_rate_limit(mut self, limit: RateLimit) -> Self {
        self.rate_limit = limit;
        self
    }

    /// Allow this transfer to be stopped through `cancel`.
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    /// Transfer `url` into `destination`, reporting progress through `on_progress`.
    ///
    /// `authorize` attaches credentials; it is a closure rather than a stored strategy so
    /// this crate stays independent of how the caller authenticates.
    pub async fn download<F, A>(
        &self,
        url: &str,
        destination: &Path,
        authorize: A,
        mut on_progress: F,
    ) -> CoreResult<DownloadOutcome>
    where
        F: FnMut(Progress) + Send,
        A: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder + Send + Sync,
    {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let partial_len = tokio::fs::metadata(destination)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let mut checkpoint = Checkpoint::load(destination).await;
        if let Some(cp) = checkpoint.as_mut() {
            cp.reconcile(partial_len);
        }

        let resume_from = plan_start(
            checkpoint.as_ref(),
            checkpoint.as_ref().and_then(|c| c.etag.as_deref()),
            partial_len,
        );

        let mut request = authorize(self.http.get(url));
        if resume_from > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
        }

        let response = request.send().await?.error_for_status()?;
        let status = response.status();
        let headers = response.headers().clone();

        let etag = headers
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let filename = headers
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(filename_from_disposition);

        // Decide whether this response really continues the partial file.
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
                // The server sent the whole file despite the Range header, expected against
                // Gameyfin 2.4, which has no Range support.
                (StartMode::RestartedByServer, 0)
            } else {
                (StartMode::Fresh, 0)
            };

        // `Content-Length` is the remaining bytes, so a resumed transfer must add the
        // offset back to recover the true total.
        let total_bytes = response.content_length().map(|len| len + written);

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

        // Record the transfer before any bytes move, so even an immediate failure leaves
        // enough behind to resume from rather than starting over.
        cp.save(destination).await?;

        let mut limiter = RateLimiter::shared(self.rate_limit.clone());

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            // Checked before taking the chunk so a cancel during a stalled transfer is
            // still noticed, and before any further writing so nothing is half-applied.
            if self.cancel.is_cancelled() {
                persist(&mut file, &mut cp, destination, written).await;
                return Err(CoreError::Cancelled);
            }

            // A dropped connection must not also lose the progress already on disk: the
            // periodic flush may be up to FLUSH_INTERVAL behind, and for a transfer that
            // fails quickly it will not have run at all.
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
                // Leave the partial file and its checkpoint in place so the next attempt
                // can resume rather than starting over.
                cp.received_bytes = written;
                cp.save(destination).await?;
                return Err(CoreError::SizeMismatch {
                    expected,
                    actual: written,
                });
            }
        }

        Checkpoint::clear(destination).await?;

        Ok(DownloadOutcome {
            path: destination.to_path_buf(),
            bytes: written,
            mode,
        })
    }
}

/// Flush what has been written and record it, ignoring secondary failures.
///
/// This runs on an error path, where the transfer has already failed. A checkpoint that
/// cannot be written is unfortunate but must not mask the original cause.
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
        // The whole point of sharing the handle: a download already in flight must pick
        // up a new cap rather than keeping the one it started with.
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
    fn a_downloader_takes_no_limit_by_default() {
        let plain = Downloader::new(reqwest::Client::new());
        assert_eq!(plain.rate_limit.get(), 0);
        assert_eq!(
            Downloader::new(reqwest::Client::new())
                .with_rate_limit(1024)
                .rate_limit
                .get(),
            1024
        );
        // Zero is "no limit", not "stop".
        assert_eq!(
            Downloader::new(reqwest::Client::new())
                .with_rate_limit(0)
                .rate_limit
                .get(),
            0
        );
        // A shared handle keeps its identity, so later changes reach the transfer.
        let shared = RateLimit::new(4096);
        let downloader =
            Downloader::new(reqwest::Client::new()).with_shared_rate_limit(shared.clone());
        shared.set(2048);
        assert_eq!(downloader.rate_limit.get(), 2048);
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
    fn progress_reports_fraction_and_eta() {
        let p = Progress {
            received_bytes: 50,
            total_bytes: Some(100),
            bytes_per_second: 10.0,
        };
        assert_eq!(p.fraction(), Some(0.5));
        assert_eq!(p.eta_seconds(), Some(5));
    }

    #[test]
    fn progress_without_a_total_has_no_fraction_or_eta() {
        let p = Progress {
            received_bytes: 50,
            total_bytes: None,
            bytes_per_second: 10.0,
        };
        assert_eq!(p.fraction(), None);
        assert_eq!(p.eta_seconds(), None);
    }

    #[test]
    fn stalled_transfer_has_no_eta() {
        let p = Progress {
            received_bytes: 1,
            total_bytes: Some(100),
            bytes_per_second: 0.0,
        };
        assert_eq!(p.eta_seconds(), None);
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
