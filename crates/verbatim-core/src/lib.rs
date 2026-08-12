//! Verbatim archive core.
//!
//! The session blob is truth; everything else is derived and rebuildable from
//! it (`.planning/PROJECT.md`, D-13). This crate owns the store, the
//! block-framed zstd blob format and the ingest path. It links no async
//! runtime, no HTTP client and no thread pool: the hook path must not pay for
//! a runtime it does not use (`DESIGN-BRIEF.md:39`).

pub mod blob;
pub mod config;
pub mod derive;
pub mod discover;
pub mod error;
pub mod ingest;
pub mod parse;
pub mod recover;
pub mod reindex;
pub mod store;
pub mod verify;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use config::{config_dir, Config};
pub use error::{Error, Result};
pub use store::{data_dir, RebuildRequired, Store, ARCHIVE_FORMAT, DERIVED_SCHEMA};
