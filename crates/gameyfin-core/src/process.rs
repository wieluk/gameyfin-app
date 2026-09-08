//! Supervising a running game. Watched directly rather than by polling the process table,
//! for an exact exit time (when a save backup must be taken). The whole process *tree* is
//! tracked, since many games start via a bootstrapper that exits immediately: Windows uses
//! a Job Object, Linux a process group.

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
    /// The last of the game's error output, with Wine's routine chatter removed, so a game
    /// that exits immediately still says why.
    #[serde(default)]
    pub error_output: String,
}

impl Session {
    /// Playtime in whole minutes, rounded so a 90-second session counts as 2.
    pub fn minutes_played(&self) -> u32 {
        (self.duration.as_secs_f64() / 60.0).round() as u32
    }

    /// Whether the session ran long enough to record playtime or trigger a save backup.
    pub fn is_meaningful(&self) -> bool {
        self.duration >= MEANINGFUL_SESSION
    }
}

/// Below this, a launch is treated as a failure rather than a session.
pub const MEANINGFUL_SESSION: Duration = Duration::from_secs(10);

/// Lines Wine and Mesa emit even on a healthy run, so they never explain a failure. Kept
/// literal: a broader pattern would also swallow the message that mattered.
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

    /// The last few lines worth reading, with known Wine/driver noise removed (it arrives
    /// after whatever failed). Falls back to the unfiltered tail when filtering leaves nothing.
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

/// Run a program to completion, capturing output. Used for installers, whose failure
/// reason is in stderr, rather than [`Supervisor`], which discards output.
pub async fn run_capturing(command: &ResolvedCommand) -> CoreResult<CapturedRun> {
    run_capturing_limited(command, None).await
}

/// Run a program with an optional cap on its address space.
///
/// The cap works around a bug in FreeArc's `unarc.dll` (used by repack installers): its
/// `LargestMemoryBlock` search overflows in 32-bit arithmetic and spins forever once a
/// contiguous 2 GB block is available, which a 64-bit host provides. Set between `fork`
/// and `exec`, so it covers only a program we spawn ourselves, not one reached through
/// `flatpak-spawn`; use [`ResolvedCommand::cap_address_space`] to pick the mechanism.
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

    let output = cmd
        .output()
        .await
        .map_err(|source| spawn_error(command, source))?;

    Ok(CapturedRun {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Windows' `ERROR_ELEVATION_REQUIRED`: `CreateProcess` returns it for a program whose
/// manifest asks for admin rights, which must instead be requested through the shell.
pub const ERROR_ELEVATION_REQUIRED: i32 = 740;

/// Turn a failure to spawn into an error the caller can act on.
fn spawn_error(command: &ResolvedCommand, source: std::io::Error) -> CoreError {
    if source.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) {
        return CoreError::ElevationRequired {
            program: command.program.to_string_lossy().into_owned(),
        };
    }
    CoreError::Other(format!(
        "could not run {}: {source}",
        command.program.to_string_lossy()
    ))
}

/// Run a program with administrator rights, waiting for it to finish. Goes through the
/// shell (the UAC dialog is the consent step), via PowerShell rather than an `unsafe`
/// `ShellExecuteEx` FFI call. The elevated process is not our child, so only its exit code
/// comes back and [`CapturedRun`]'s output fields are empty.
pub async fn run_elevated(command: &ResolvedCommand) -> CoreResult<CapturedRun> {
    #[cfg(not(windows))]
    {
        let _ = command;
        Err(CoreError::Other(
            "running a program as administrator is a Windows-only thing".to_string(),
        ))
    }

    #[cfg(windows)]
    {
        // Base64-encoded so nothing in the script has to survive PowerShell's quoting.
        let script = elevation_script(command);
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &encode_command(&script),
            ])
            // Without this a console window flashes up behind the consent dialog.
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .await
            .map_err(|source| {
                CoreError::Other(format!("could not ask for administrator rights: {source}"))
            })?;

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if output.status.code() == Some(EXIT_UAC_REFUSED) {
            return Err(CoreError::Other(
                "The administrator prompt was dismissed, so the program did not run.".to_string(),
            ));
        }

        Ok(CapturedRun {
            status: output.status.code(),
            stdout: String::new(),
            stderr,
        })
    }
}

/// Windows' flag for "start this process without a console window".
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(any(windows, test))]
/// What [`elevation_script`] exits with when the user dismissed the consent dialog.
///
/// Chosen from the range Windows itself does not use for process exit codes, so it cannot
/// be confused with something the installer returned.
const EXIT_UAC_REFUSED: i32 = 223;

#[cfg(any(windows, test))]
/// A PowerShell single-quoted string.
///
/// The only rule inside one is that `'` doubles: no backslash escapes, no variable
/// expansion. That makes it safe for a Windows path, which is full of backslashes, and
/// for the parentheses and spaces this app puts in its folder names.
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(any(windows, test))]
/// The script that asks Windows to run one command elevated and reports its exit code.
fn elevation_script(command: &ResolvedCommand) -> String {
    let mut script =
        String::from("$ErrorActionPreference = 'Stop'; try { $process = Start-Process -FilePath ");
    script.push_str(&ps_quote(&command.program.to_string_lossy()));

    if !command.args.is_empty() {
        let args: Vec<String> = command
            .args
            .iter()
            .map(|arg| ps_quote(&arg.to_string_lossy()))
            .collect();
        script.push_str(" -ArgumentList ");
        script.push_str(&args.join(","));
    }

    if let Some(dir) = &command.working_dir {
        script.push_str(" -WorkingDirectory ");
        script.push_str(&ps_quote(&dir.to_string_lossy()));
    }

    script.push_str(" -Verb RunAs -Wait -PassThru }");
    // Distinguish a dismissed consent dialog (a refusal) from a program failure.
    script.push_str(&format!(
        " catch {{ exit {EXIT_UAC_REFUSED} }}; if ($null -eq $process.ExitCode) {{ exit 0 }}; exit $process.ExitCode"
    ));
    script
}

#[cfg(any(windows, test))]
/// Encode a script the way PowerShell's `-EncodedCommand` expects: UTF-16LE, then base64.
fn encode_command(script: &str) -> String {
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64(&utf16)
}

#[cfg(any(windows, test))]
/// Standard base64, written out rather than pull in a dependency for one caller.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(triple >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

/// A game process tree being watched.
pub struct Supervisor {
    child: Child,
    started: Instant,
    /// Bounded tail of the child's stderr, filled by a reader task.
    stderr: SharedTail,
    /// The task filling `stderr`, joined before the tail is read: `wait` can return while
    /// the last of stderr is still in the pipe.
    reader: Option<tokio::task::JoinHandle<()>>,
    #[cfg(windows)]
    job: windows_job::Job,
}

/// A bounded, shared buffer holding the most recent error output.
type SharedTail = std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>>;

/// How many lines of error output to keep. The last lines, since the failure is at the end.
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

        // stdout discarded; stderr piped and drained continuously by the task below, since
        // a full pipe would block the game.
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());

        // Own process group, so the whole tree can be waited on and signalled together.
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
                    // Filtered on the way in, so hours of GPU warnings don't push out the failure.
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
            // Signal the whole group so a bootstrapped game does not survive as an orphan.
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

    /// Wait for the stderr reader to finish so the tail is complete, with a timeout in case
    /// a descendant inherited the pipe and outlived the process.
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

/// Windows process handling. **Tree tracking is not implemented yet**: this waits on the
/// spawned process only, so a game that hands off to a bootstrapper undercounts playtime.
/// A Job Object is the fix, pending testing on Windows.
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

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // The high bits, where a sloppy shift shows up.
        assert_eq!(base64(&[0xFF, 0xFF, 0xFF]), "////");
    }

    #[test]
    fn the_encoded_command_is_utf16_little_endian() {
        // PowerShell rejects anything else, silently, by running nothing at all.
        assert_eq!(encode_command("hi"), base64(&[b'h', 0, b'i', 0]));
    }

    #[test]
    fn quoting_survives_the_characters_this_app_puts_in_paths() {
        // Backslashes are literal inside a PowerShell single-quoted string, which is why
        // it is the right quote for a Windows path.
        assert_eq!(
            ps_quote("C:\\Games\\(76) Metal Slug Tactics"),
            "'C:\\Games\\(76) Metal Slug Tactics'"
        );
        // The one character that has to be escaped, and the only way to do it.
        assert_eq!(ps_quote("it's"), "'it''s'");
    }

    #[test]
    fn the_elevation_script_passes_the_program_its_arguments_and_folder() {
        let mut command = command(
            "C:\\Downloads\\(76) Game\\setup.exe",
            &["/SP-", "/DIR=C:\\Games\\(76) Game"],
        );
        command.working_dir = Some(std::path::PathBuf::from("C:\\Downloads\\(76) Game"));

        let script = elevation_script(&command);
        assert!(script.contains("-Verb RunAs"), "got: {script}");
        assert!(script.contains("-Wait"), "got: {script}");
        assert!(
            script.contains("-ArgumentList '/SP-','/DIR=C:\\Games\\(76) Game'"),
            "got: {script}"
        );
        assert!(
            script.contains("-WorkingDirectory 'C:\\Downloads\\(76) Game'"),
            "got: {script}"
        );
        // A dismissed consent dialog has to be distinguishable from a failed install.
        assert!(
            script.contains(&format!("exit {EXIT_UAC_REFUSED}")),
            "got: {script}"
        );
        assert!(script.contains("exit $process.ExitCode"), "got: {script}");
    }

    #[test]
    fn a_program_without_arguments_gets_no_empty_argument_list() {
        // `Start-Process -ArgumentList` with nothing after it is a syntax error.
        let script = elevation_script(&command("C:\\setup.exe", &[]));
        assert!(!script.contains("-ArgumentList"), "got: {script}");
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
