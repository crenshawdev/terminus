//! Terminus archive core.
//!
//! The session blob is truth; everything else is derived and rebuildable from
//! it (`.planning/PROJECT.md`, D-13). This crate owns the store, the
//! block-framed zstd blob format and the ingest path. It links no async
//! runtime and no thread-pool crate: the hook path must not pay for a runtime
//! it does not use (`DESIGN-BRIEF.md:39`).
//!
//! It does link an HTTP client, as of phase 7, and the exception is worth
//! naming precisely rather than leaving the sentence above to quietly become
//! false. [`observe::net`] is the one module that names it and the one
//! constructor that opens a connection (PRIV-03, D-06): a blocking pure-Rust
//! client with no runtime behind it, reached only by the opt-in provider call
//! and by nothing on the hook path. The root `Cargo.toml` records what it cost.
//!
//! It does start threads, in exactly one place, and the exception is worth
//! naming precisely rather than leaving the sentence above to quietly become
//! false. [`ingest::backfill`] runs a fixed set of [`std::thread`] workers for
//! the parse and compression of a whole-history backfill, with one thread
//! owning every SQLite write (D-11). Nothing else starts one: a hook fires
//! `ingest::pass`, which is single-threaded, and `backfill` is reached only by
//! `terminus backfill`. There is still no pool, global or otherwise - no
//! `rayon`, nothing that outlives the call that created it.

pub mod blob;
pub mod capture;
pub mod config;
pub mod credentials;
pub mod derive;
pub mod discover;
pub mod error;
pub mod feedback;
pub mod index;
pub mod ingest;
pub mod inject;
pub mod lineage;
pub mod observe;
pub mod owner_only;
pub mod parse;
pub mod project;
pub mod recall;
pub mod recover;
pub mod reindex;
pub mod retention;
pub mod store;
pub mod verify;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use config::{config_dir, Config};
pub use error::{Error, Result};
pub use store::{data_dir, RebuildRequired, Store, ARCHIVE_FORMAT, DERIVED_SCHEMA};
