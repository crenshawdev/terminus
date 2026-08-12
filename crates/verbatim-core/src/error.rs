//! The crate's error type.

use std::path::PathBuf;

/// Anything that can go wrong inside `verbatim-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// STOR-05's refusal. Both integers are named because "unsupported store
    /// format" without them tells the user nothing about which side to move.
    #[error(
        "store at {path} has archive format {store_format}, but this build of \
         verbatim knows archive format {binary_format}; upgrade verbatim to open it"
    )]
    ArchiveFormatTooNew {
        path: PathBuf,
        store_format: i64,
        binary_format: i64,
    },

    /// A file sits where the store belongs but does not carry the version
    /// integers every verbatim store carries.
    #[error("{path} exists but is not a verbatim store ({detail})")]
    NotAStore { path: PathBuf, detail: String },

    /// No data directory could be resolved and none was given.
    #[error("cannot resolve a data directory: {detail}")]
    DataDirUnresolved { detail: String },
}

impl Error {
    /// Attach a path to an [`std::io::Error`]: bare io errors name no file, and
    /// a message that names no file is a message that starts an investigation.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
