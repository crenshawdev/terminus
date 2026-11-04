//! The subcommands: `ingest`, `backfill`, `search`, `show`, `sessions`, `verify`,
//! `reindex`, `status`, `compact`, `usage`, `export`, `doctor`, `mcp`.
//!
//! `mcp` is the one that keeps none of what follows. It is not a data command
//! typed at a terminal: it is a JSON-RPC server spawned by Claude Code, its
//! stdout is the transport rather than a stream of results, and its wire format
//! is the MCP one and not the `{command, ok, reason, data}` envelope below.
//! What it does share is the read path - the same read-only open, the same
//! scoping, the same exclusion - because two definitions of what a read may see
//! is exactly the drift this module exists to prevent.
//!
//! # The contract every data command keeps
//!
//! **Exit codes.** 0 on success **including an empty result set** - a query that
//! matched nothing is a successful answer to a question with no matches - 1 on
//! operational failure, 2 on misuse. [`Failure`] is the one place that split
//! lives, so a script can tell "this store is corrupt" from "you typed it
//! wrong".
//!
//! **Streams.** Data on stdout, every diagnostic on stderr, on every path. That
//! is what lets `terminus search --json | jq` work while a warning is still
//! printed.
//!
//! **`--json`.** Every data command accepts it and writes exactly one document
//! on stdout and nothing else: `{command, ok, reason, data}`, built and
//! serialized by `serde_json` and never assembled with `format!`. `ok` is
//! the exit code's answer, and a caller that parses the document and a caller
//! that checks the code never disagree - `verify --json` on a damaged store
//! writes its failures AND exits 1.
//!
//! The shapes are documented field for field in `docs/guide/reference/json-shapes.md` and
//! `crates/terminus/tests/cli.rs` holds every command to them: one test per
//! property across all commands, rather than one test per command, so a seventh
//! command is one line rather than a new file.

pub mod backfill;
pub mod compact;
pub mod data;
pub mod doctor;
pub mod export;
pub mod hook;
pub mod ingest;
pub mod install;
pub mod json;
pub mod mcp;
pub mod observations;
pub mod read;
pub mod reindex;
pub mod replay;
pub mod retention;
pub mod search;
pub mod sessions;
pub mod show;
pub mod spawn;
pub mod stats;
pub mod status;
pub mod uninstall;
pub mod usage;
pub mod verify;

use std::path::{Path, PathBuf};

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
    /// sessions that failed; a trailing "terminus: ..." line would be a second
    /// account of the same thing.
    Silent,
    /// Exit 2: the command line does not name work that could be attempted.
    Misuse(String),
}

impl Failure {
    /// The failure's own words, for a line that reports it rather than
    /// returns it (#107).
    ///
    /// `Silent` has no message because the command already wrote its account;
    /// no caller that reports a failure this way is fed one, so the wording is
    /// the fallback a future caller would print.
    pub fn text(self) -> String {
        match self {
            Failure::Operational(message) | Failure::Misuse(message) => message,
            Failure::Silent => "no reason was recorded".to_owned(),
        }
    }
}

impl From<terminus_core::Error> for Failure {
    fn from(error: terminus_core::Error) -> Self {
        Failure::Operational(error.to_string())
    }
}

/// The data directory every command works against.
///
/// Resolved once, here, and passed down explicitly afterwards rather than
/// re-derived at each use.
pub fn data_dir() -> Result<PathBuf, Failure> {
    Ok(terminus_core::data_dir()?)
}

/// The store's footprint on disk: `terminus.db` and its two WAL sidecars.
///
/// Three files and not one. SQLite in WAL mode holds recently written pages in
/// `-wal` until something checkpoints them, so the database file alone can be
/// half the truth - and a bare `VACUUM` on a WAL store moves the pages it frees
/// INTO the WAL, which makes `terminus.db` shrink while the store on disk grows.
/// `cmd::status` reports this same sum as `size_bytes`, so this is the number
/// the product itself calls the store's size and the one `compact` has to move.
///
/// A file that is not there counts zero rather than failing: `-wal` and `-shm`
/// exist only for the life of a connection, so their absence is the ordinary
/// state of a store nothing has open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Footprint {
    pub db: u64,
    pub wal: u64,
    pub shm: u64,
}

impl Footprint {
    pub fn total(self) -> u64 {
        self.db + self.wal + self.shm
    }

    /// The same three numbers and their sum, as a `--json` value.
    ///
    /// The components travel with the total because the total on its own cannot
    /// be read: "the store got bigger" and "the WAL has not been checkpointed
    /// yet" are the same number, and only the split tells them apart.
    pub fn to_value(self) -> serde_json::Value {
        serde_json::json!({
            "db": self.db,
            "wal": self.wal,
            "shm": self.shm,
            "total": self.total(),
        })
    }
}

/// Measure that footprint now.
///
/// `cmd::status` keeps its own copy of this rule.
pub fn footprint(data_dir: &Path) -> Footprint {
    let of = |name: String| {
        std::fs::metadata(data_dir.join(name))
            .map(|meta| meta.len())
            .unwrap_or(0)
    };
    let db = terminus_core::store::DB_FILE_NAME;
    Footprint {
        db: of(db.to_owned()),
        wal: of(format!("{db}-wal")),
        shm: of(format!("{db}-shm")),
    }
}

/// Whether `data_dir` holds a store.
///
/// A store this process cannot look at is a failure, not a missing store:
/// `Path::exists` says false for both, and a command would then answer an
/// archive it cannot read with "there is no store". The rule
/// `Store::open_read_only` follows.
pub fn has_store(data_dir: &Path) -> Result<bool, Failure> {
    let path = data_dir.join(terminus_core::store::DB_FILE_NAME);
    path.try_exists().map_err(|e| {
        let detail = e.to_string();
        Failure::Operational(terminus_core::Error::StoreUnreadable { path, detail }.to_string())
    })
}

/// What a directory holds, in bytes: every file under it, recursively.
///
/// Every file rather than the three [`footprint`] sums. `data move` carries
/// the injection-state directory (`decision::DIR_NAME`) and the snapshots too,
/// and `uninstall --purge` deletes them, so a size that named only the database
/// would understate a directory holding three rolling copies of it by a factor
/// of four (#95).
pub fn directory_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => total += directory_bytes(&entry.path()),
            Ok(_) => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => {}
        }
    }
    total
}

/// A byte count as a person reads it, for the non-`--json` output.
///
/// Never in the document: a consumer that wants "3.2 MiB" can render it, and
/// one comparing two runs cannot parse it back out of prose.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Whole seconds as a person reads a duration: `Ns` under a minute, `Nm Ns`
/// under an hour, `Nh Nm` from there on.
///
/// A signed count because `observations` reads one from a stored column, and a
/// damaged row can make it negative; that prints as the seconds it is. The
/// words for "none" and "under a second" stay with the callers that have them
/// (#107).
pub fn duration(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

/// The `--json` flag as spelled on every command line.
pub const JSON_FLAG: &str = "json";

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

/// The value of `--since` or `--until`, through terminus-core's date check,
/// [`terminus_core::recall::checked_bound`], as a misuse when it fails.
///
/// The check is the library's and not the CLI's, so every command and the MCP
/// server refuse the same values and extend a bare date the same way. The core
/// message starts with the field, so the flag's dashes in front of it name the
/// flag as it was typed, beside the value.
pub fn date_value(parser: &mut lexopt::Parser, flag: &'static str) -> Result<String, Failure> {
    let raw = value(parser, flag)?;
    terminus_core::recall::checked_bound(flag, &raw)
        .map_err(|error| Failure::Misuse(format!("--{error}")))
}

/// Parse the rest of the command line for a subcommand whose only argument is
/// `--json`, rejecting anything else.
///
/// Each subcommand parses its own flags because `main.rs` hands the
/// `lexopt::Parser` on. This is the shape for the commands that take nothing
/// else; the rest match `--json` as one arm of their own loop.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One formatter for every size a command prints, so the install estimate
    /// and `status` agree on a byte count (#107).
    #[test]
    fn a_byte_count_reads_in_powers_of_two() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1024), "1.0 KiB");
        assert_eq!(human(1_400_000_000), "1.3 GiB");
    }

    #[test]
    fn a_duration_reads_as_seconds_minutes_or_hours() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(2), "2s");
        assert_eq!(duration(59), "59s");
        assert_eq!(duration(100), "1m 40s");
        assert_eq!(duration(201), "3m 21s");
        assert_eq!(duration(3599), "59m 59s");
        assert_eq!(duration(7_400), "2h 3m");
        assert_eq!(duration(-5), "-5s");
    }
}
