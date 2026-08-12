//! Verbatim archive core.
//!
//! The session blob is truth; everything else is derived and rebuildable from
//! it (`.planning/PROJECT.md`, D-13). This crate owns the store, the
//! block-framed zstd blob format and the ingest path. It links no async
//! runtime, no HTTP client and no thread pool: the hook path must not pay for
//! a runtime it does not use (`DESIGN-BRIEF.md:39`).

pub mod blob;
pub mod error;
pub mod parse;
pub mod store;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use error::{Error, Result};
pub use store::{data_dir, RebuildRequired, Store, ARCHIVE_FORMAT, DERIVED_SCHEMA};
