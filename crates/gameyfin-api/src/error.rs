use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    /// The server rejected the call as unauthenticated (HTTP 401/403), or answered
    /// with the login page, which Gameyfin does instead of a 401 in some configurations.
    #[error("not authenticated: {0}")]
    Unauthenticated(String),

    #[error("{endpoint} failed with HTTP {status}: {body}")]
    Status {
        endpoint: String,
        status: u16,
        body: String,
    },

    #[error("{endpoint} returned an undecodable body: {source}")]
    Decode {
        endpoint: String,
        #[source]
        source: serde_json::Error,
    },

    /// The server has save synchronisation turned off (HTTP 405).
    #[error("save sync is turned off on this server")]
    SaveSyncDisabled,

    /// No save sync routes at all (HTTP 404). Unlike [`Self::SaveSyncDisabled`], an
    /// administrator cannot fix this with a switch.
    #[error("this server does not support save sync ({endpoint} returned HTTP {status})")]
    SaveSyncUnsupported { endpoint: String, status: u16 },

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

    /// True when nobody answered, so the user keeps their session and cached library. A proxy's
    /// 502 or 503 counts: the application behind it is restarting.
    pub fn is_unreachable(&self) -> bool {
        match self {
            ApiError::Transport(_) => true,
            ApiError::Status { status, .. } => matches!(status, 502..=504),
            _ => false,
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
