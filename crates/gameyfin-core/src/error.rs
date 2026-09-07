use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("not enough space on {path}: need {needed} bytes, {available} available")]
    InsufficientSpace {
        path: String,
        needed: u64,
        available: u64,
    },

    /// The server restarted the transfer instead of honouring a range request.
    ///
    /// Gameyfin 2.4 does not implement `Range` (its download endpoint streams a
    /// `StreamingResponseBody` with no `Accept-Ranges`), so this is expected until the
    /// corresponding server change lands and the caller must fall back to a fresh start.
    #[error("server ignored the range request and restarted the transfer")]
    RangeNotHonoured,

    #[error("download finished at {actual} bytes but {expected} were expected")]
    SizeMismatch { expected: u64, actual: u64 },

    /// The caller asked for the transfer to stop. Not a failure: the partial file and its
    /// checkpoint are intact, so it resumes rather than restarting.
    #[error("cancelled")]
    Cancelled,

    #[error("no executable found under {0}")]
    NoExecutable(String),

    #[error("{path} is not an archive this app can unpack")]
    UnsupportedArchive { path: String },

    #[error(transparent)]
    Api(#[from] gameyfin_api::ApiError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Other(String),
}

pub type CoreResult<T> = Result<T, CoreError>;
