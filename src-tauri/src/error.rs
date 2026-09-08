//! Errors crossing the IPC boundary, flattened into a tagged shape so the UI can tell
//! "you are signed out" (actionable) apart from "the server broke".

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("no server configured")]
    NotConnected,

    #[error(transparent)]
    Api(#[from] gameyfin_api::ApiError),

    /// A failure with an explanation already written for the user.
    #[error("{0}")]
    Message(String),
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
        let wire = match self {
            CommandError::NotConnected => Wire::NotConnected {
                message: self.to_string(),
            },
            CommandError::Api(e) if e.is_auth() => Wire::Unauthenticated {
                message: e.to_string(),
            },
            CommandError::Api(e) => Wire::Failed {
                message: e.to_string(),
            },
            CommandError::Message(message) => Wire::Failed {
                message: message.clone(),
            },
        };
        wire.serialize(serializer)
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
