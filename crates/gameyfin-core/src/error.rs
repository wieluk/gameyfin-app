use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("not enough space on {path}: need {needed} bytes, {available} available")]
    InsufficientSpace {
        path: String,
        needed: u64,
        available: u64,
    },

    /// The server ignored a range request. Expected on Gameyfin 2.4, which has no `Range`
    /// support, so the caller starts fresh.
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

    /// A download that cannot be unpacked while it arrives, so the caller saves it as a file.
    #[error("{0}")]
    CannotStream(String),

    /// Windows requires administrator rights. A variant, not a message, so the caller can ask
    /// the user and relaunch through the shell's consent dialog.
    #[error("{program} will only run with administrator rights")]
    ElevationRequired { program: String },

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
