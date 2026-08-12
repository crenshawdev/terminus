//! The subcommands. Phase 1 ships three: `ingest`, `verify`, `reindex`.
//!
//! `--json` on data commands, stable output shapes and the full exit-code
//! contract are RCL-06 in phase 3. What is here is only what the phase-1
//! acceptance criteria assert on: exit 0 on success, 1 on operational failure,
//! 2 on misuse, data on stdout, errors on stderr.

pub mod ingest;

use std::path::PathBuf;

/// How a command ended.
///
/// An operational failure is a condition the store or the filesystem reported,
/// not a mistake in the command line, and the two get different exit codes so a
/// script can tell "this store is corrupt" from "you typed it wrong".
#[derive(Debug)]
pub enum Failure {
    /// Exit 1: the work was attempted and did not succeed.
    Operational(String),
    /// Exit 2: the command line does not name work that could be attempted.
    Misuse(String),
}

impl From<verbatim_core::Error> for Failure {
    fn from(error: verbatim_core::Error) -> Self {
        Failure::Operational(error.to_string())
    }
}

/// The data directory every command works against.
///
/// Resolved once, here, and passed down explicitly afterwards rather than
/// re-derived at each use (`DESIGN-BRIEF.md:404`).
pub fn data_dir() -> Result<PathBuf, Failure> {
    Ok(verbatim_core::data_dir()?)
}
