//! One ingest pass over one named transcript.
//!
//! The order of the first two steps is the design: the lock is taken **before**
//! the store is opened (D-15), so a run that loses the race costs one file open
//! and one lock syscall and touches no database.

pub mod lock;

use std::path::Path;

pub use lock::{Attempt, IngestLock, LOCK_FILE_NAME};

use crate::error::Result;
use crate::store::Store;

/// What one invocation of [`run`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Another process is mid-pass. Nothing was written, and this is a success:
    /// the hook spawn is the scheduler and a second invocation is expected
    /// (ING-02).
    LockHeld,
    /// The pass committed.
    Committed(Pass),
}

/// What a committed pass moved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pass {
    pub bytes_read: u64,
    pub turns_added: usize,
    pub watermark: u64,
}

/// Ingest one transcript into the store under `data_dir`.
///
/// The body of a committed pass arrives in task 3; what is settled here is the
/// shape every later step hangs off - lock first, store second, and a `runs`
/// row for a pass that commits.
pub fn run(data_dir: &Path, _transcript: &Path) -> Result<Outcome> {
    let started = std::time::Instant::now();
    let _guard = match lock::try_acquire(data_dir)? {
        Attempt::Held => return Ok(Outcome::LockHeld),
        Attempt::Acquired(guard) => guard,
    };

    let store = Store::open(data_dir)?;
    let pass = Pass::default();
    record_run(&store, &pass, started.elapsed())?;
    Ok(Outcome::Committed(pass))
}

/// Write the one `runs` row a committed pass leaves behind.
///
/// There is no log file; `status` (phase 2) surfaces this table. A *failed*
/// pass needs a second transaction after a rollback and is phase 2's (ING-03,
/// ING-09): phase 1 records only a pass that committed.
pub(crate) fn record_run(
    store: &Store,
    pass: &Pass,
    elapsed: std::time::Duration,
) -> Result<()> {
    store.conn().execute(
        "INSERT INTO runs (started_at, finished_at, duration_ms, files_seen, bytes_read, turns_added)
         VALUES (
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            ?2, 1, ?3, ?4
         )",
        rusqlite::params![
            format!("-{} seconds", elapsed.as_secs_f64()),
            elapsed.as_millis() as i64,
            pass.bytes_read as i64,
            pass.turns_added as i64,
        ],
    )?;
    Ok(())
}
