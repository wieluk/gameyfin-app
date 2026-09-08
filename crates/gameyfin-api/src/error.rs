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

    /// The server has save synchronisation turned off (HTTP 405).
    #[error("save sync is turned off on this server")]
    SaveSyncDisabled,

    /// The server has no save sync routes at all (HTTP 404), so it predates the feature.
    /// Distinct from [`Self::SaveSyncDisabled`] because the remedy differs: an
    /// administrator can switch the feature on, but cannot add it to an older server.
    #[error("this server does not support save sync")]
    SaveSyncUnsupported,

    /// The upload exceeded the per-save or per-user limit (HTTP 413).
    #[error("save is too large, or the storage quota is full")]
    QuotaExceeded,

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
    ///
    /// A gateway status counts: a reverse proxy answering 502 or 503 while the application
    /// behind it restarts is the same situation as an unplugged cable, and treating it as
    /// a real answer is what used to sign people out.
    pub fn is_unreachable(&self) -> bool {
        match self {
            ApiError::Transport(_) => true,
            ApiError::Status { status, .. } => matches!(status, 502..=504),
            _ => false,
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
