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

    /// D-13: the transcript on disk is shorter than the bytes the archive
    /// already holds for it, so the pass skipped it and touched nothing.
    ///
    /// Its own variant rather than an [`Error::Io`] carrying a message, because
    /// the pass has to tell this one failure apart from every other in order to
    /// flag the session - and the kind discriminates nothing, since `path_key`
    /// also raises `InvalidData` for a transcript path that is not UTF-8, while
    /// matching on the message text breaks the first time the wording moves.
    #[error(
        "{path} is {len} bytes but its watermark is at {watermark}; \
         the archive was left untouched"
    )]
    TranscriptDiverged {
        path: PathBuf,
        len: u64,
        watermark: u64,
    },

    /// D-13's other half: the file is long enough but is not the file that was
    /// archived. A separate variant from [`Error::TranscriptDiverged`] because
    /// that one's message is about a length, and a transcript that was
    /// truncated and then written back past its old watermark has the right
    /// length and the wrong bytes - reporting it as a length mismatch would
    /// name a number the user can check and find correct.
    #[error(
        "{path} is long enough for its watermark at {watermark} but its first \
         {watermark} bytes are not the ones archived; the archive was left untouched"
    )]
    TranscriptRewritten { path: PathBuf, watermark: u64 },

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

    /// The bytes in `sessions.blob` are not a blob this build can read.
    #[error("malformed session blob: {detail}")]
    BlobFormat { detail: String },

    /// A read asked for bytes the session stream does not contain. Reading past
    /// the end is an error, never a short read: a truncated turn would be
    /// indistinguishable from a turn that really is short.
    #[error(
        "read of {len} bytes at offset {offset} runs past the end of a \
         {uncompressed_len}-byte session stream"
    )]
    RangeOutOfBounds {
        offset: u64,
        len: u64,
        uncompressed_len: u64,
    },

    /// A blob's stored stream does not hash to the checksum recorded for it.
    ///
    /// Raised before an append, never after: an append that trusted the bytes
    /// it copied would mint a fresh checksum over the corruption and destroy
    /// the only evidence that anything was ever wrong (D-06's checksum is the
    /// evidence). A live session is re-ingested repeatedly as it grows, so the
    /// very next append is what would certify the damage permanently.
    #[error("session blob failed its recorded checksum: expected {expected}, found {actual}")]
    BlobChecksumMismatch { expected: String, actual: String },

    /// zstd failed to compress or decompress a block.
    #[error("zstd {operation} failed: {source}")]
    Codec {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    /// No data directory could be resolved and none was given.
    #[error("cannot resolve a data directory: {detail}")]
    DataDirUnresolved { detail: String },

    /// A config file exists and does not parse.
    ///
    /// Named rather than swallowed: a file the user wrote and verbatim could
    /// not read means verbatim would walk a tree nobody asked for and honor no
    /// exclusion at all, which is the one failure that must not be silent. The
    /// detail carries the parser's own position.
    #[error("{path} is not valid TOML: {detail}")]
    ConfigParse { path: PathBuf, detail: String },

    /// No config directory, or no home directory to derive one from.
    #[error("cannot resolve a config directory: {detail}")]
    ConfigUnresolved { detail: String },
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
