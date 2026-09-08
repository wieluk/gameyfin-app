use thiserror::Error;

/// Errors returned by the Gameyfin API client.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The server rejected the call as unauthenticated (HTTP 401/403), or answered
    /// with the login page, which Gameyfin does instead of a 401 in some configurations.
    #[error("not authenticated: {0}")]
    Unauthenticated(String),

    /// The server answered with a non-success status.
    #[error("{endpoint} failed with HTTP {status}: {body}")]
    Status {
        endpoint: String,
        status: u16,
        body: String,
    },

    /// The response body could not be decoded.
    #[error("{endpoint} returned an undecodable body: {source}")]
    Decode {
        endpoint: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("no Gameyfin server URL configured")]
    NoServerUrl,

    #[error("invalid server URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error(transparent)]
    Transport(#[from] reqwest::Error),

    #[error("{0}")]
    Other(String),
}

impl ApiError {
    /// True when re-authenticating could plausibly fix this error.
    pub fn is_auth(&self) -> bool {
        matches!(self, ApiError::Unauthenticated(_))
    }

    /// True when the request never got an answer out of the server.
    ///
    /// The distinction the app cares about is "the server said no" versus "there was
    /// nobody to ask". Only the second means the user should keep their session and see
    /// their cached library rather than being sent back to the sign-in wizard.
    pub fn is_unreachable(&self) -> bool {
        matches!(self, ApiError::Transport(_))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
