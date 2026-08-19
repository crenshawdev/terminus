//! The subcommands: `ingest`, `search`, `show`, `sessions`, `verify`, `reindex`,
//! `status`, `doctor`, `mcp`.
//!
//! `mcp` is the one that keeps none of what follows. It is not a data command
//! typed at a terminal: it is a JSON-RPC server spawned by Claude Code, its
//! stdout is the transport rather than a stream of results, and its wire format
//! is the MCP one and not the `{command, ok, reason, data}` envelope below.
//! What it does share is the read path - the same read-only open, the same
//! scoping, the same exclusion - because two definitions of what a read may see
//! is exactly the drift this module exists to prevent.
//!
//! # The contract every data command keeps (RCL-06)
//!
//! **Exit codes.** 0 on success **including an empty result set** - a query that
//! matched nothing is a successful answer to a question with no matches - 1 on
//! operational failure, 2 on misuse. [`Failure`] is the one place that split
//! lives, so a script can tell "this store is corrupt" from "you typed it
//! wrong".
//!
//! **Streams.** Data on stdout, every diagnostic on stderr, on every path. That
//! is what lets `verbatim search --json | jq` work while a warning is still
//! printed.
//!
//! **`--json`.** Every data command accepts it and writes exactly one document
//! on stdout and nothing else: `{command, ok, reason, data}`, built and
//! serialized by `serde_json` and never assembled with `format!` (D-25). `ok` is
//! the exit code's answer, and a caller that parses the document and a caller
//! that checks the code never disagree - `verify --json` on a damaged store
//! writes its failures AND exits 1.
//!
//! The shapes are documented field for field in `docs/json-shapes.md` and
//! `crates/verbatim/tests/cli.rs` holds every command to them: one test per
//! property across all commands, rather than one test per command, so a seventh
//! command is one line rather than a new file.

pub mod doctor;
pub mod hook;
pub mod ingest;
pub mod install;
pub mod json;
pub mod mcp;
pub mod read;
pub mod reindex;
pub mod search;
pub mod sessions;
pub mod show;
pub mod spawn;
pub mod status;
pub mod uninstall;
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

/// One end of a time window, as a string that compares against a stored
/// timestamp directly.
///
/// D-23: all 22,412 turns of a 180-file sample carry exactly
/// `NNNN-NN-NNTNN:NN:NN.NNNZ` - one format, UTC only - so the comparison is
/// lexicographic and there is no date parsing on the stored side. A bare
/// `YYYY-MM-DD` extends to the first instant of that day for `since` and the
/// last for `until`, which is what makes a one-day window include its own day;
/// extending both to midnight would make `--since D --until D` return only the
/// turn at exactly 00:00:00.000 and look like an empty day.
///
/// Anything else is misuse rather than an empty result. Every string orders
/// against every other, so a malformed bound would compare cleanly and return a
/// plausible wrong answer that nothing reports.
///
/// This runs in the CLI, ahead of the query layer, on purpose:
/// `recall::search::run` resolves the scope before it validates its filters, so
/// a bad bound typed in a directory no archived project covers would otherwise
/// be answered with the scope's reason and exit 0. `recall::search` keeps its
/// own copy of the rule for library callers, which is a duplication worth
/// naming - see the open items on this plan.
pub fn time_bound(flag: &str, raw: &str) -> Result<String, Failure> {
    const FULL: &str = "NNNN-NN-NNTNN:NN:NN.NNNZ";
    const DATE: &str = "NNNN-NN-NN";

    let shaped = |shape: &str| {
        raw.len() == shape.len()
            && raw
                .bytes()
                .zip(shape.bytes())
                .all(|(byte, expected)| match expected {
                    b'N' => byte.is_ascii_digit(),
                    other => byte == other,
                })
    };

    if shaped(FULL) {
        return Ok(raw.to_owned());
    }
    if shaped(DATE) {
        let tail = if flag == "until" {
            "T23:59:59.999Z"
        } else {
            "T00:00:00.000Z"
        };
        return Ok(format!("{raw}{tail}"));
    }
    Err(Failure::Misuse(format!(
        "--{flag} {raw:?} is not a time; expected YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS.mmmZ"
    )))
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
