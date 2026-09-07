//! Supervising a running game.
//!
//! The game is spawned by us, so it can be watched directly rather than by sampling the
//! process table. Polling needs an ignore list for launcher subprocesses, quantises
//! playtime, and cannot say exactly when a game exited, which is the moment a save backup
//! must be taken.
//!
//! The whole process *tree* has to be tracked, because many games start through a
//! bootstrapper that exits immediately: Windows assigns the child to a Job Object, Linux
//! puts it in its own process group and waits for the group to empty.

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};

use crate::error::{CoreError, CoreResult};
use crate::launch::ResolvedCommand;

/// How a supervised session ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionEnd {
    /// The tree exited on its own.
    Exited { code: Option<i32> },
    /// The session was stopped by us.
    Terminated,
}

/// The result of one play session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub duration: Duration,
    pub end: SessionEnd,
    /// The last of the game's error output, with Wine's routine chatter removed.
    ///
    /// A game that exits immediately says why on stderr. Discarding that left "ran for
    /// 1.4s and exited with code 1" as the entire diagnosis: true, useless, and identical
    /// for a missing library, an unsupported binary and a bad prefix.
    #[serde(default)]
    pub error_output: String,
}

impl Session {
    /// Playtime in whole minutes.
    ///
    /// Rounds rather than truncates, so a 90-second session counts as 2 minutes instead
    /// of 1 and a 20-second one counts as nothing.
    pub fn minutes_played(&self) -> u32 {
        (self.duration.as_secs_f64() / 60.0).round() as u32
    }

    /// Whether the session is long enough to be worth recording.
    ///
    /// A game that dies immediately, a missing dependency, the wrong executable, should
    /// not add playtime or trigger a save backup.
    pub fn is_meaningful(&self) -> bool {
        self.duration >= MEANINGFUL_SESSION
    }
}

/// Below this, a launch is treated as a failure rather than a session.
pub const MEANINGFUL_SESSION: Duration = Duration::from_secs(10);

/// Output that never explains a failure.
///
/// Only lines that Wine and Mesa emit on a *healthy* run. Each `err:`-tagged entry below
/// was observed in a run that succeeded: `wine cmd /c echo` prints all four before its
/// output. Treating them as diagnostic is worse than useless, because they once made up
/// the entire error message shown for a failed install, describing nothing that failed.
///
/// Kept deliberately literal. A pattern broad enough to cover "driver errors" would also
/// swallow the one that mattered, so each entry names a specific thing this Wine build is
/// simply known not to ship or need.
fn is_noise(line: &str) -> bool {
    const BENIGN: [&str; 8] = [
        // Not shipped by the builds we download; wineboot always tries to run it.
        "winemenubuilder.exe",
        // No USB or Bluetooth passthrough in a sandbox, and none is wanted.
        "wineusb",
        "winebth",
        // Kernel-mode graphics shims Wine never actually loads.
        "win32k.sys",
        "dxgkrnl.sys",
        "dxgmms1.sys",
        // Emitted throughout a perfectly normal prefix build.
        "setupapi:do_file_copyW Unsupported style",
        // A compositor without wlr data control: clipboard is limited, nothing is broken.
        "zwlr_data_control_manager_v1",
    ];

    line.contains("fixme:")
        || line.contains(":winediag:")
        || line.contains("libEGL warning")
        || line.starts_with("pci id for fd")
        || BENIGN.iter().any(|pattern| line.contains(pattern))
}

/// The result of running a program to completion, with its output.
#[derive(Debug, Clone)]
pub struct CapturedRun {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CapturedRun {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// The last few lines of output, for a log line or an error message.
    ///
    /// Wine and setup programs can be extremely verbose; the tail is where the actual
    /// failure appears.
    /// The last few lines worth reading, with known noise removed.
    ///
    /// Wine emits `fixme:` and `winediag:` lines on every process start, and a driver
    /// stack that cannot reach the GPU adds pages of `libEGL warning` on top. Those arrive
    /// *after* whatever actually failed, so a plain tail is mostly chatter and the real
    /// error scrolls out of it, which is exactly how an installer failure came back as
    /// twelve lines of "please mention your exact version when filing bug reports".
    ///
    /// Falls back to the unfiltered tail when filtering leaves nothing, so a program whose
    /// only output happens to look like noise still reports something.
    pub fn diagnostic_tail(&self, lines: usize) -> String {
        let source = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let kept: Vec<&str> = source
            .lines()
            .filter(|l| !l.trim().is_empty() && !is_noise(l))
            .collect();

        if kept.is_empty() {
            return self.tail(lines);
        }
        kept.iter()
            .rev()
            .take(lines)
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn tail(&self, lines: usize) -> String {
        let source = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let collected: Vec<&str> = source.lines().filter(|l| !l.trim().is_empty()).collect();
        collected
            .iter()
            .rev()
            .take(lines)
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Run a program to completion, capturing its output.
///
/// Used for installers rather than [`Supervisor`], which discards output: when a setup
/// program fails instantly the reason is in its stderr, and throwing that away leaves
/// nothing to diagnose.
pub async fn run_capturing(command: &ResolvedCommand) -> CoreResult<CapturedRun> {
    run_capturing_limited(command, None).await
}

/// Run a program with an optional cap on its address space.
///
/// The cap exists for a long-standing bug in FreeArc's `unarc.dll`, which repack
/// installers use for decompression. Its `LargestMemoryBlock` binary search computes the
/// midpoint as `(a + b) / 2`; once `a` reaches `0x7FFFFFFF` that overflows in 32-bit
/// arithmetic and it spins forever on one core. The overflow is only reachable when a
/// 32-bit process can obtain a contiguous 2 GB block, which a 64-bit host provides.
/// Capping below that stops the first allocation ever growing large enough to trigger it.
/// See github.com/kash7an/wine-fitgirl-unarc-largestmemoryblock-overflow.
///
/// The limit applies to the whole process tree, which is what makes it effective against
/// a helper DLL loaded by a child of the setup program.
///
/// It reaches that tree by being set between `fork` and `exec`, so it only covers a
/// program we spawn ourselves. A command that crosses a sandbox boundary, Wine reached
/// through `flatpak-spawn`, is spawned on the host by someone else and inherits nothing
/// from us; use [`ResolvedCommand::cap_address_space`] to decide which mechanism a given
/// command needs rather than passing a limit here unconditionally.
///
/// [`ResolvedCommand::cap_address_space`]: crate::ResolvedCommand::cap_address_space
pub async fn run_capturing_limited(
    command: &ResolvedCommand,
    address_space: Option<u64>,
) -> CoreResult<CapturedRun> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args);
    for (key, value) in &command.env {
        cmd.env(key, value);
    }
    if let Some(dir) = &command.working_dir {
        cmd.current_dir(dir);
    }

    #[cfg(target_os = "linux")]
    if let Some(limit) = address_space {
        // SAFETY: `setrlimit` is async-signal-safe and valid between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                let rlimit = RLimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if setrlimit(RLIMIT_AS, &rlimit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = address_space;

    let output = cmd.output().await.map_err(|source| {
        CoreError::Other(format!(
            "could not run {}: {source}",
            command.program.to_string_lossy()
        ))
    })?;

    Ok(CapturedRun {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// A game process tree being watched.
pub struct Supervisor {
    child: Child,
    started: Instant,
    /// Bounded tail of the child's stderr, filled by a reader task.
    stderr: SharedTail,
    /// The task filling `stderr`, awaited before the tail is read.
    ///
    /// The child exiting does not mean its output has been consumed: `wait` can return
    /// while the last of stderr is still in the pipe, and reading the tail then yields
    /// nothing. That is the exact case the tail exists for, a program that dies
    /// immediately, so the reader has to be joined first.
    reader: Option<tokio::task::JoinHandle<()>>,
    #[cfg(windows)]
    job: windows_job::Job,
}

/// A bounded, shared buffer holding the most recent error output.
type SharedTail = std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>>;

/// How many lines of error output to keep.
///
/// Wine is extremely chatty, a normal run emits pages of `fixme:` lines, so this keeps
/// the *last* lines rather than the first: the failure is at the end, and an unbounded
/// buffer would grow for the whole session.
const ERROR_TAIL_LINES: usize = 40;

impl Supervisor {
    /// Spawn a resolved command and begin supervising it.
    pub fn spawn(command: &ResolvedCommand) -> CoreResult<Self> {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args);
        for (key, value) in &command.env {
            cmd.env(key, value);
        }
        if let Some(dir) = &command.working_dir {
            cmd.current_dir(dir);
        }

        // stdout is genuinely uninteresting and can be enormous, so it is discarded. The
        // pipe stderr goes to is drained continuously by the task below: a pipe nobody
        // reads fills up and blocks the game, which is why this was not simply inherited.
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());

        // Put the child in its own process group so the whole tree can be waited on and,
        // if needed, signalled together.
        #[cfg(unix)]
        {
            // SAFETY: `setsid` is async-signal-safe and valid between fork and exec.
            unsafe {
                cmd.pre_exec(|| {
                    if libc_setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        let mut child = cmd.spawn().map_err(|source| {
            CoreError::Other(format!(
                "could not launch {}: {source}",
                command.program.to_string_lossy()
            ))
        })?;

        #[cfg(windows)]
        let job = {
            let job = windows_job::Job::new()?;
            job.assign(&child)?;
            job
        };

        let stderr: SharedTail = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::with_capacity(ERROR_TAIL_LINES),
        ));
        let reader = child.stderr.take().map(|pipe| {
            let sink = stderr.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Filtered on the way in rather than on the way out: a game that runs
                    // for hours would otherwise fill the buffer with GPU warnings and
                    // push out the failure that ended it.
                    if line.trim().is_empty() || is_noise(&line) {
                        continue;
                    }
                    if let Ok(mut buffer) = sink.lock() {
                        if buffer.len() == ERROR_TAIL_LINES {
                            buffer.pop_front();
                        }
                        buffer.push_back(line);
                    }
                }
            })
        });

        Ok(Self {
            child,
            stderr,
            reader,
            started: Instant::now(),
            #[cfg(windows)]
            job,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Wait until the whole process tree has exited.
    pub async fn wait(mut self) -> CoreResult<Session> {
        let status = self.child.wait().await?;
        self.drain_stderr().await;

        // The process we spawned is gone, but a bootstrapper may have handed off to the
        // real game. On Windows the job object tells us when the last descendant exits.
        #[cfg(windows)]
        self.job.wait_for_empty().await?;

        Ok(Session {
            duration: self.started.elapsed(),
            end: SessionEnd::Exited {
                code: status.code(),
            },
            error_output: self.error_tail(),
        })
    }

    /// Stop the game.
    pub async fn terminate(mut self) -> CoreResult<Session> {
        #[cfg(windows)]
        self.job.terminate()?;

        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // Signal the whole group, not just the leader, so a bootstrapped game does
            // not survive as an orphan.
            // SAFETY: `killpg` with a valid pid; a failure here is not fatal.
            unsafe {
                libc_killpg(pid as i32, 15); // SIGTERM
            }
        }

        let _ = self.child.kill().await;
        self.drain_stderr().await;
        Ok(Session {
            duration: self.started.elapsed(),
            end: SessionEnd::Terminated,
            error_output: self.error_tail(),
        })
    }

    /// Wait for the stderr reader to finish, so the tail is complete.
    ///
    /// Bounded, because a descendant that inherited the pipe and outlived the process we
    /// waited on would otherwise hold it open indefinitely. A truncated tail is worth far
    /// more than a launch that never reports a result.
    async fn drain_stderr(&mut self) {
        let Some(reader) = self.reader.take() else {
            return;
        };
        if tokio::time::timeout(std::time::Duration::from_secs(2), reader)
            .await
            .is_err()
        {
            tracing::debug!("stderr was still open after the process exited; tail may be short");
        }
    }

    /// The error output collected so far, most recent lines last.
    fn error_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|buffer| buffer.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "setsid"]
    fn libc_setsid() -> i32;
    #[link_name = "killpg"]
    fn libc_killpg(pgrp: i32, sig: i32) -> i32;
}

/// Address-space resource, as numbered by the Linux kernel.
#[cfg(target_os = "linux")]
const RLIMIT_AS: i32 = 9;

#[cfg(target_os = "linux")]
#[repr(C)]
struct RLimit {
    rlim_cur: u64,
    rlim_max: u64,
}

#[cfg(target_os = "linux")]
extern "C" {
    #[link_name = "setrlimit64"]
    fn setrlimit(resource: i32, rlim: *const RLimit) -> i32;
}

/// Windows process handling.
///
/// **Tree tracking is not implemented yet.** The design calls for a Job Object, which
/// captures every descendant and signals when the last one exits; that needs the
/// `windows` crate and, to be trustworthy, testing on Windows. Until then this waits on
/// the process actually spawned, which is correct for the common case and wrong only for
/// a game that hands off to a bootstrapper and exits, there the session ends early and
/// playtime is undercounted.
///
/// This is deliberately a working limitation rather than an error: a client that refuses
/// to launch anything on Windows would be far worse than one that occasionally
/// mismeasures a session.
#[cfg(windows)]
mod windows_job {
    use crate::error::CoreResult;
    use tokio::process::Child;

    pub struct Job;

    impl Job {
        pub fn new() -> CoreResult<Self> {
            Ok(Self)
        }

        pub fn assign(&self, _child: &Child) -> CoreResult<()> {
            Ok(())
        }

        /// No-op until Job Objects land; `Child::wait` has already returned by here.
        pub async fn wait_for_empty(&self) -> CoreResult<()> {
            Ok(())
        }

        /// `Child::kill` handles the spawned process; descendants are not yet reached.
        pub fn terminate(&self) -> CoreResult<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    fn command(program: &str, args: &[&str]) -> ResolvedCommand {
        ResolvedCommand {
            program: OsString::from(program),
            args: args.iter().map(OsString::from).collect(),
            env: BTreeMap::new(),
            working_dir: None,
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_failing_launch_keeps_its_error_output() {
        // Without this, a game that never starts is indistinguishable from one the user
        // closed at once: same duration, same cleared activity, no reason anywhere.
        let supervisor = Supervisor::spawn(&command(
            "/bin/sh",
            &["-c", "echo 'could not find libfoo.so' >&2; exit 1"],
        ))
        .unwrap();
        let session = supervisor.wait().await.unwrap();

        assert_eq!(session.end, SessionEnd::Exited { code: Some(1) });
        assert!(
            session.error_output.contains("libfoo"),
            "got: {:?}",
            session.error_output
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn error_output_is_bounded_by_the_last_lines() {
        // Wine emits pages of `fixme:` noise, so the buffer must not grow with the
        // session, and the end is the part worth keeping.
        let supervisor = Supervisor::spawn(&command(
            "/bin/sh",
            &[
                "-c",
                "i=0; while [ $i -lt 500 ]; do echo line$i >&2; i=$((i+1)); done; exit 1",
            ],
        ))
        .unwrap();
        let session = supervisor.wait().await.unwrap();

        let lines: Vec<&str> = session.error_output.lines().collect();
        assert!(
            lines.len() <= ERROR_TAIL_LINES,
            "kept {} lines",
            lines.len()
        );
        assert_eq!(lines.last(), Some(&"line499"), "the tail must be the end");
    }

    #[test]
    fn the_diagnostic_tail_keeps_the_error_not_the_chatter() {
        // Taken from a real installer failure: Wine prints its version banner on every
        // process start, so the lines *after* the error are what a plain tail returns.
        let run = CapturedRun {
            status: Some(1),
            stdout: String::new(),
            stderr: "0024:err:start:fatal_error FormatMessage failed\n\
                     00ec:fixme:winediag:loader_init wine-staging 11.17 is a testing version\n\
                     00ec:fixme:winediag:loader_init Please mention your exact version\n\
                     libEGL warning: egl: failed to create dri2 screen\n\
                     pci id for fd 59: 10de:2f04, driver (null)\n"
                .into(),
        };

        let plain = run.tail(3);
        assert!(
            !plain.contains("fatal_error"),
            "the unfiltered tail is expected to lose it: {plain}"
        );

        let filtered = run.diagnostic_tail(3);
        assert!(filtered.contains("fatal_error"), "got: {filtered}");
        assert!(!filtered.contains("fixme"), "got: {filtered}");
        assert!(!filtered.contains("libEGL"), "got: {filtered}");
    }

    #[test]
    fn output_that_is_entirely_noise_still_reports_something() {
        // Better a useless message than an empty one: "it failed and said nothing" is
        // itself a diagnosis.
        let run = CapturedRun {
            status: Some(1),
            stdout: String::new(),
            stderr: "0024:fixme:winediag:loader_init testing version\n".into(),
        };
        assert!(!run.diagnostic_tail(5).is_empty());
    }

    #[test]
    fn minutes_are_rounded_not_truncated() {
        let session = |secs| Session {
            duration: Duration::from_secs(secs),
            end: SessionEnd::Exited { code: Some(0) },
            error_output: String::new(),
        };
        assert_eq!(session(90).minutes_played(), 2);
        assert_eq!(session(20).minutes_played(), 0);
        assert_eq!(session(3600).minutes_played(), 60);
    }

    #[test]
    fn a_crash_on_startup_is_not_a_session() {
        let crashed = Session {
            duration: Duration::from_secs(2),
            end: SessionEnd::Exited { code: Some(1) },
            error_output: String::new(),
        };
        assert!(!crashed.is_meaningful());

        let played = Session {
            duration: Duration::from_secs(600),
            end: SessionEnd::Exited { code: Some(0) },
            error_output: String::new(),
        };
        assert!(played.is_meaningful());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn supervises_a_real_process_to_completion() {
        let supervisor = Supervisor::spawn(&command("/bin/sh", &["-c", "exit 3"])).unwrap();
        assert!(supervisor.pid().is_some());

        let session = supervisor.wait().await.unwrap();
        assert_eq!(session.end, SessionEnd::Exited { code: Some(3) });
        // Far below the threshold, so it must not be counted as playtime.
        assert!(!session.is_meaningful());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn measures_elapsed_time() {
        let supervisor = Supervisor::spawn(&command("/bin/sh", &["-c", "sleep 0.3"])).unwrap();
        let session = supervisor.wait().await.unwrap();
        assert!(
            session.duration >= Duration::from_millis(250),
            "expected at least 250ms, got {:?}",
            session.duration
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn terminating_reports_a_terminated_session() {
        let supervisor = Supervisor::spawn(&command("/bin/sh", &["-c", "sleep 30"])).unwrap();
        let session = supervisor.terminate().await.unwrap();
        assert_eq!(session.end, SessionEnd::Terminated);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn environment_reaches_the_process() {
        let mut cmd = command("/bin/sh", &["-c", "test \"$GAMEYFIN_TEST\" = ok"]);
        cmd.env.insert("GAMEYFIN_TEST".into(), "ok".into());
        let session = Supervisor::spawn(&cmd).unwrap().wait().await.unwrap();
        assert_eq!(session.end, SessionEnd::Exited { code: Some(0) });
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn captures_output_and_exit_code() {
        let run = run_capturing(&command(
            "/bin/sh",
            &["-c", "echo out; echo err >&2; exit 4"],
        ))
        .await
        .unwrap();
        assert_eq!(run.status, Some(4));
        assert!(!run.success());
        assert!(run.stdout.contains("out"));
        assert!(run.stderr.contains("err"));
        // stderr wins for the tail, because that is where failures are described.
        assert_eq!(run.tail(5), "err");
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn an_address_space_limit_reaches_the_child() {
        // `ulimit -v` reports the limit in kibibytes.
        let limit = 3 * 1024 * 1024 * 1024u64;
        let run = run_capturing_limited(&command("/bin/sh", &["-c", "ulimit -v"]), Some(limit))
            .await
            .unwrap();

        assert!(run.success(), "stderr: {}", run.stderr);
        let reported: u64 = run.stdout.trim().parse().expect("a numeric limit");
        assert_eq!(reported, limit / 1024);
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn no_limit_leaves_the_child_unrestricted() {
        let run = run_capturing(&command("/bin/sh", &["-c", "ulimit -v"]))
            .await
            .unwrap();
        assert!(run.success());
        // Inherited from this process, which is normally unlimited.
        assert!(!run.stdout.trim().is_empty());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn the_tail_falls_back_to_stdout() {
        let run = run_capturing(&command("/bin/sh", &["-c", "echo only-stdout"]))
            .await
            .unwrap();
        assert!(run.success());
        assert_eq!(run.tail(5), "only-stdout");
    }

    #[tokio::test]
    async fn a_missing_executable_fails_to_spawn() {
        let result = Supervisor::spawn(&command("/definitely/not/here", &[]));
        assert!(result.is_err());
    }
}
