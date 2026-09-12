//! Errors crossing the IPC boundary, tagged so the UI can tell "signed out" from "failed".

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("no server configured")]
    NotConnected,

    #[error(transparent)]
    Api(#[from] gameyfin_api::ApiError),

    /// Already worded for the user.
    #[error("{0}")]
    Message(String),
}

impl CommandError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Wire {
    NotConnected { message: String },
    Unauthenticated { message: String },
    Failed { message: String },
}

impl Serialize for CommandError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let message = self.to_string();
        match self {
            CommandError::NotConnected => Wire::NotConnected { message },
            CommandError::Api(e) if e.is_auth() => Wire::Unauthenticated { message },
            _ => Wire::Failed { message },
        }
        .serialize(serializer)
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

/// Prefixes any displayable error with what was being attempted.
pub trait Context<T> {
    fn context(self, what: impl std::fmt::Display) -> CommandResult<T>;
}

impl<T, E: std::fmt::Display> Context<T> for Result<T, E> {
    fn context(self, what: impl std::fmt::Display) -> CommandResult<T> {
        self.map_err(|e| CommandError::Message(format!("{what}: {e}")))
    }
}

/// Runs blocking work off the async runtime, folding a panic into the same error.
pub async fn blocking<T, E>(
    what: &str,
    f: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> CommandResult<T>
where
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .context(what)?
        .context(what)
}
