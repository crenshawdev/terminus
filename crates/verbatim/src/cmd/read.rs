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
