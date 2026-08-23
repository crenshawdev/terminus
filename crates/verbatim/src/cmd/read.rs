//! The one place `search`, `show` and `sessions` open the store.
//!
//! Three commands opening a store three times is three chances to drift on the
//! two behaviours D-10 and D-18 fix, and neither of them is visible in a test
//! that only checks the command answered.
//!
//! **Read-only, always (D-10).** [`verbatim_core::Store::open_read_only`] and
//! never `Store::open`: opening a store is a read, and `Store::open` would
//! `create_dir_all`, initialize a fresh database, run the additive column
//! bring-forward and set two pragmas. `verbatim search` against a machine that
//! has never ingested must not leave a store behind as the side effect of a
//! question, and PLAN-4's server advertises `readOnlyHint` over this same path.
//!
//! **A missing store is an answer, not a failure.** A machine that has installed
//! verbatim and not yet run an ingest is the ordinary starting state, so it gets
//! an empty result carrying a reason and exit 0. A store that is *there* and
//! cannot be read is the other thing: that is operational (exit 1), because a
//! script told "no results" about an unreadable database would carry on.
//!
//! **A store older than this build says so and is not repaired (D-18).**
//! `reindex::open_up_to_date` stays the only caller that rebuilds. A read that
//! silently rewrote four tables would not be a read, and a user who upgraded the
//! binary and searched before the next hook fired must not read a degraded
//! result as a search bug.

use std::path::Path;

use verbatim_core::recall::Scope;
use verbatim_core::{Config, Error, Store};

use super::json::Document;
use super::Failure;

/// What a read command found where the store should be.
pub enum Opened {
    /// A store to query.
    ///
    /// Boxed because the other variant is a string and this one is a whole
    /// `Store` - one open per process, so the indirection costs nothing and the
    /// enum stops being three hundred bytes wide at every call site.
    Ready(Box<Reader>),
    /// Nothing to query, and why. An empty result and exit 0.
    Nothing(String),
}

/// An open store and the config every read path filters through.
///
/// The two travel together because they are only meaningful together: the store
/// holds the sessions and the config says which of them a read path may see
/// (`config::visible`, ING-08), and a caller that had one without the other
/// could ask a question exclusion never got to answer.
pub struct Reader {
    store: Store,
    config: Config,
}

impl Reader {
    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

/// What a read command prints when the store predates this build.
///
/// One line, on stderr, and then the query runs anyway against the old-shape
/// index. Saying it is the whole of D-18: the results really are degraded, and
/// the difference between a user who knows that and a user who does not is the
/// difference between waiting for the next ingest and filing a search bug.
const STALE: &str = "this store's derived tables predate this build, so results may be \
                     incomplete; the next `verbatim ingest` rebuilds them";

/// What `stats` and `replay` say on a store that has no decision log.
///
/// The same shape as a machine that has never ingested: a reason and exit 0.
/// Nothing has been recorded, which is an answer to "how is injection doing"
/// rather than a failure of the question.
pub const NO_DECISION_LOG: &str = "this store predates the decision log, so nothing has been \
                                   recorded yet; the next `verbatim ingest` creates it";

/// Open the store every read command reads.
pub fn open() -> Result<Opened, Failure> {
    let data_dir = super::data_dir()?;
    let config = Config::load()?;
    open_in(&data_dir, config)
}

/// The same, against a named data directory.
///
/// Split from [`open`] so the environment resolution and the open rule can be
/// tested apart: a test that had to set `VERBATIM_DATA_DIR` would be setting a
/// process-global in a parallel test binary.
pub fn open_in(data_dir: &Path, config: Config) -> Result<Opened, Failure> {
    let store = match Store::open_read_only(data_dir) {
        Ok(store) => store,
        // The ordinary state of a machine that has never ingested. Nothing is
        // created here, and that includes the data directory itself.
        Err(Error::StoreNotFound { path }) => {
            return Ok(Opened::Nothing(format!(
                "no verbatim store at {}; nothing has been archived yet",
                path.display()
            )))
        }
        Err(other) => return Err(Failure::Operational(other.to_string())),
    };

    if store.predates_this_build() {
        eprintln!("verbatim: {STALE}");
    }

    Ok(Opened::Ready(Box::new(Reader { store, config })))
}

/// What a store missing part of the decision log is: old, or damaged.
///
/// The distinction is the whole point of the type. `decisions` and `labels`
/// arrived together in one `CREATE_SQL` (D-02), so a store that predates them
/// is missing BOTH - and a store missing exactly one of them was written by a
/// build that had both and has since lost one. Collapsing the two into "no
/// decision log" would report a damaged store as a successful empty answer, and
/// the number a caller then reads as "injection has recorded nothing" would in
/// fact be "half the log is gone".
pub enum DecisionLog {
    /// Both tables are there and can be queried.
    Present,
    /// Neither table: a store written before the log existed.
    Absent,
    /// One and not the other. Names the one that is gone.
    Damaged(&'static str),
}

/// What this store can be asked about the decision log.
///
/// Asking here, at the open, is what keeps the ordinary case an answer: left to
/// the query it surfaces as [`Failure::Operational`] carrying a raw
/// `no such table: decisions`, which `main` prints on stderr and exits 1 on -
/// without knowing whether `--json` was asked for, so the envelope the contract
/// promises is never written at all.
///
/// Nothing is repaired either way, because a read never migrates (D-18). For
/// [`DecisionLog::Absent`] the next write-mode `verbatim ingest` creates both
/// tables through `bring_forward`'s missing-table arm (D-02); for
/// [`DecisionLog::Damaged`] it would create the missing one and leave the
/// surviving one's now-orphaned rows, which is why that case is reported as a
/// failure a person has to look at rather than repaired by being read.
///
/// This is a snapshot taken at open and not re-checked before the query, so a
/// writer that drops a table in between still reaches the raw sqlite path. That
/// race is not closable from a read-only connection - the check and the query
/// are two statements whatever their order - and it is the same one
/// [`Store::missing_columns`] has always had.
pub fn decision_log(reader: &Reader) -> DecisionLog {
    let missing = reader.store().missing_tables();
    match (missing.contains(&"decisions"), missing.contains(&"labels")) {
        (false, false) => DecisionLog::Present,
        (true, true) => DecisionLog::Absent,
        (true, false) => DecisionLog::Damaged("decisions"),
        (false, true) => DecisionLog::Damaged("labels"),
    }
}

/// The reason a damaged store reports, naming the table that is gone.
pub fn damaged(table: &str) -> String {
    format!(
        "this store is missing the `{table}` table but not the rest of the decision log, so \
         something removed it; `verbatim doctor` reports on the store, and the rows that are \
         left cannot be read as a complete record"
    )
}

/// The project a read command works in: the one named, or the one the process
/// is standing in.
///
/// D-12: the default is a longest-prefix match of the working directory against
/// the project keys `session_meta` already holds, resolved inside
/// [`verbatim_core::recall::scope`]. The literal `*` means every project, and
/// exclusion stays in force under it.
pub fn scope(named: Option<&str>) -> Result<Scope, Failure> {
    match named {
        Some(value) => Ok(Scope::parse(value)),
        None => Ok(Scope::current_directory()?),
    }
}

/// Say why a result is empty, on the stream the mode wants, and exit 0.
///
/// An empty result is a successful answer to a question with no matches
/// (RCL-06), so this never fails. The reason goes into the document in JSON mode
/// and onto stderr otherwise - never onto stdout, which stays either a document
/// or the hits themselves.
pub fn empty(document: Document, reason: &str, json: bool) -> Result<(), Failure> {
    if json {
        document.because(reason).emit();
    } else {
        eprintln!("verbatim: {reason}");
    }
    Ok(())
}

/// Say why this store cannot answer the question, and exit 1.
///
/// The counterpart of [`empty`] for a store that is *wrong* rather than empty.
/// The `--json` arm writes the envelope with `ok: false` and then returns
/// [`Failure::Silent`], because `main` would otherwise print a second account of
/// the same thing on stderr after the document already carried it - the shape
/// `verify` established. Exit 1 either way, so a caller reading the code and a
/// caller parsing the document agree.
pub fn unusable(document: Document, reason: String, json: bool) -> Failure {
    if json {
        document.failed().because(&reason).emit();
        Failure::Silent
    } else {
        Failure::Operational(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data directory with no store in it: an empty result with a reason, and
    /// not one byte created there.
    ///
    /// The second half is what `Store::open` would fail: it would create the
    /// directory, initialize a database and set `journal_mode=wal`, so a machine
    /// that has never ingested would acquire a store by being asked a question.
    #[test]
    fn an_empty_data_directory_is_a_reason_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");

        match open_in(&data_dir, Config::default()).unwrap() {
            Opened::Nothing(reason) => {
                assert!(reason.contains("no verbatim store"), "{reason}");
                assert!(
                    reason.contains(&data_dir.display().to_string()),
                    "the reason must name where it looked: {reason}"
                );
            }
            Opened::Ready(_) => panic!("a directory with no store opened as a store"),
        }

        assert!(
            !data_dir.exists(),
            "a read created the data directory it was asked about"
        );
    }

    /// A file that exists and is not a store is operational, not an empty
    /// result: something is there and this build cannot read it.
    #[test]
    fn a_file_that_is_not_a_store_is_operational() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("verbatim.db"), b"not a database at all").unwrap();

        match open_in(dir.path(), Config::default()) {
            Err(Failure::Operational(_)) => {}
            Err(other) => panic!("expected an operational failure, got {other:?}"),
            Ok(_) => panic!("a file that is not a database opened as a store"),
        }
    }
}
