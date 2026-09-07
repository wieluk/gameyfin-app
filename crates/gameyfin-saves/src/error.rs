use thiserror::Error;

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("could not run {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("ludusavi {command} exited with {status}: {stderr}")]
    CommandFailed {
        command: String,
        status: i32,
        stderr: String,
    },

    /// Ludusavi prints JSON on stdout but warnings on stderr, so an empty stdout with a
    /// zero exit is a distinct failure worth naming.
    #[error("ludusavi {command} produced no output")]
    EmptyOutput { command: String },

    #[error("could not parse ludusavi {command} output: {source}")]
    Parse {
        command: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("no Ludusavi entry matches '{title}'")]
    NoMatch { title: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("could not serialise ludusavi config: {0}")]
    ConfigSerialize(#[from] serde_yaml_ng::Error),

    #[error("{0}")]
    Other(String),
}

pub type SaveResult<T> = Result<T, SaveError>;
