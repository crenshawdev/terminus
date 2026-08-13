//! The subcommands: `ingest`, `verify`, `reindex`, `status`.
//!
//! `--json` on data commands, stable output shapes and the full exit-code
//! contract are RCL-06 in phase 3. What is here is only what the phase-1
//! acceptance criteria assert on: exit 0 on success, 1 on operational failure,
//! 2 on misuse, data on stdout, errors on stderr.

pub mod ingest;
pub mod json;
pub mod read;
pub mod reindex;
pub mod search;
pub mod show;
pub mod status;
pub mod verify;

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
    /// Exit 1, with nothing more to say. `verify` has already printed the
    /// sessions that failed; a trailing "verbatim: ..." line would be a second
    /// account of the same thing.
    Silent,
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

/// The `--json` flag as spelled on every command line.
pub const JSON_FLAG: &str = "json";

/// Parse the rest of the command line for a subcommand whose only argument is
/// `--json`.
///
/// The flag is parsed by the subcommand rather than by `run()`, because
/// `main.rs` hands the `lexopt::Parser` on and a flag consumed before dispatch
/// could not be spelled differently by two commands that need to. This is the
/// shape for `verify`, `reindex` and `status`, which take nothing else;
/// `search`, `show` and `sessions` match `--json` as one arm of their own loop.
///
/// Everything else is still rejected. A subcommand that silently ignored an
/// argument is how `verify --json` came to look supported before it was.
/// The value of a flag that takes one, as a misuse when it is absent.
///
/// Shared by every command that has flags, so "--project with nothing after it"
/// says the same thing whichever command was asked.
pub fn value(parser: &mut lexopt::Parser, flag: &str) -> Result<String, Failure> {
    parser
        .value()
        .map_err(|_| Failure::Misuse(format!("--{flag} needs a value")))
        .map(|v| v.to_string_lossy().into_owned())
}

pub fn json_flag(parser: &mut lexopt::Parser) -> Result<bool, Failure> {
    use lexopt::prelude::*;

    let mut json = false;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long(JSON_FLAG) => json = true,
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }
    Ok(json)
}
