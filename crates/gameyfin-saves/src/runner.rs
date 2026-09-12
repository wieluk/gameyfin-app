//! Subprocess execution, behind a trait so the driver can be tested without the binary.

use async_trait::async_trait;

use crate::error::{SaveError, SaveResult};

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, program: &str, args: &[String]) -> SaveResult<CommandOutput>;
}

/// Generous enough for a large backup, short enough that a wedged process is not forever.
const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

#[derive(Debug, Clone)]
pub struct ProcessRunner {
    timeout: std::time::Duration,
}

impl Default for ProcessRunner {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl ProcessRunner {
    pub fn with_timeout(timeout: std::time::Duration) -> Self {
        Self { timeout }
    }
}

#[async_trait]
impl CommandRunner for ProcessRunner {
    async fn run(&self, program: &str, args: &[String]) -> SaveResult<CommandOutput> {
        tracing::debug!("running {program} {args:?}");

        let mut command = tokio::process::Command::new(program);
        command.args(args);
        // Without this the child outlives a timed-out call, still holding its config dir.
        command.kill_on_drop(true);

        // Suppress the console window Ludusavi would flash on Windows every backup.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        // Ludusavi can wedge forever, and every save operation shares one lock, so one hung run
        // would freeze them all.
        let output = match tokio::time::timeout(self.timeout, command.output()).await {
            Ok(result) => result.map_err(|source| SaveError::Spawn {
                program: program.to_string(),
                source,
            })?,
            Err(_) => {
                tracing::error!(program, seconds = self.timeout.as_secs(), "ludusavi hung");
                return Err(SaveError::TimedOut {
                    program: program.to_string(),
                    seconds: self.timeout.as_secs(),
                });
            }
        };

        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// A runner that replays canned output, for tests.
#[cfg(any(test, feature = "test-util"))]
#[derive(Default)]
pub struct FakeRunner {
    responses: std::sync::Mutex<Vec<CommandOutput>>,
    pub calls: std::sync::Mutex<Vec<Vec<String>>>,
}

#[cfg(any(test, feature = "test-util"))]
impl FakeRunner {
    pub fn new(responses: Vec<CommandOutput>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into_iter().rev().collect()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    pub fn failure(status: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    pub fn call(&self, index: usize) -> Vec<String> {
        self.calls.lock().unwrap()[index].clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[cfg(any(test, feature = "test-util"))]
#[async_trait]
impl CommandRunner for FakeRunner {
    async fn run(&self, _program: &str, args: &[String]) -> SaveResult<CommandOutput> {
        self.calls.lock().unwrap().push(args.to_vec());
        self.responses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| SaveError::Other("FakeRunner ran out of responses".into()))
    }
}

/// Gated whole: `sleep` and `echo` have no Windows equivalent, and a module left with only its
/// import fails `-D warnings`.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_process_that_never_returns_is_stopped_and_reported() {
        let runner = ProcessRunner::with_timeout(std::time::Duration::from_millis(200));
        let started = std::time::Instant::now();

        let error = runner
            .run("sleep", &["30".to_string()])
            .await
            .expect_err("a 30 second sleep must not finish inside 200ms");

        assert!(matches!(error, SaveError::TimedOut { seconds: 0, .. }));
        // The call returns on the timeout rather than on the child, which `kill_on_drop`
        // then reaps.
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_process_that_finishes_in_time_is_not_disturbed() {
        let runner = ProcessRunner::with_timeout(std::time::Duration::from_secs(30));
        let output = runner
            .run("echo", &["hello".to_string()])
            .await
            .expect("echo finishes at once");

        assert_eq!(0, output.status);
        assert_eq!("hello", output.stdout.trim());
    }
}
