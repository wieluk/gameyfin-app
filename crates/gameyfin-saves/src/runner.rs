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

/// Runs the real binary.
#[derive(Debug, Default, Clone)]
pub struct ProcessRunner;

#[async_trait]
impl CommandRunner for ProcessRunner {
    async fn run(&self, program: &str, args: &[String]) -> SaveResult<CommandOutput> {
        tracing::debug!("running {program} {args:?}");

        let mut command = tokio::process::Command::new(program);
        command.args(args);

        // Suppress the console window Ludusavi would flash on Windows every backup.
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let output = command.output().await.map_err(|source| SaveError::Spawn {
            program: program.to_string(),
            source,
        })?;

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
    /// Queue responses, returned in order.
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

    /// The arguments of the nth call.
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
