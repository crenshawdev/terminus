//! `verbatim reindex`: rebuild every derived table from the blobs (STOR-04).

use verbatim_core::ingest::{lock, Attempt};
use verbatim_core::reindex;

use super::Failure;

pub fn run() -> Result<(), Failure> {
    let data_dir = super::data_dir()?;

    // The lock, and it belongs here rather than inside `reindex::reindex`.
    // This command drops and recreates all four derived tables, which is the
    // most destructive thing phase 1 can do, and it was running unguarded: the
    // invariant `ingest` rests on - that the LOCK guard makes it the only
    // writer, so nothing changes the store between its read and its write - was
    // simply false whenever a reindex ran beside it. Without the lock the two
    // race in both directions: a reindex could drop the tables under a
    // hook-spawned ingest, or lose the SQLite write lock to one and die on the
    // busy timeout with a raw "database is locked", which is the wait D-15
    // exists to avoid.
    //
    // Here and not in the library because `ingest::run` already holds this lock
    // when it calls `open_up_to_date`; acquiring again underneath it would
    // report contention against itself.
    let _guard = match lock::try_acquire(&data_dir)? {
        Attempt::Held => {
            // Non-zero, unlike a contended `ingest`. AC6's exit-0 is for the
            // hook path, where a skipped pass is caught by the next one. A
            // reindex is asked for explicitly, so silently not doing it would
            // be the wrong answer.
            return Err(Failure::Operational(format!(
                "another verbatim process holds {}; try again when it finishes",
                data_dir.join(lock::LOCK_FILE_NAME).display()
            )));
        }
        Attempt::Acquired(guard) => guard,
    };

    // `open_up_to_date` first, so a store older than this build is brought
    // forward by the same code path rather than by a second one here. On an
    // up-to-date store it is a plain open and the rebuild below is the work.
    let mut store = reindex::open_up_to_date(&data_dir)?;
    let rebuilt = reindex::reindex(&mut store)?;

    // Nothing on stdout: this command produces no data, and a phase-3 `--json`
    // caller must not have to filter a progress line out of it.
    eprintln!(
        "rebuilt {} turn(s) across {} session(s)",
        rebuilt.turns, rebuilt.sessions
    );

    if rebuilt.failed.is_empty() {
        return Ok(());
    }
    for (session_key, reason) in &rebuilt.failed {
        eprintln!("{session_key}: {reason}");
    }
    eprintln!(
        "{} session(s) could not be rebuilt and were skipped",
        rebuilt.failed.len()
    );
    // Every undamaged session was still rebuilt; the exit code is what tells a
    // script the store is not whole.
    Err(Failure::Silent)
}
