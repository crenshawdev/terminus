//! `verbatim reindex`: rebuild every derived table from the blobs (STOR-04).

use serde_json::json;
use verbatim_core::ingest::{lock, Attempt};
use verbatim_core::reindex;

use super::json::Document;
use super::Failure;

pub fn run(json: bool) -> Result<(), Failure> {
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
            let held = format!(
                "another verbatim process holds {}; try again when it finishes",
                data_dir.join(lock::LOCK_FILE_NAME).display()
            );
            if json {
                // A refusal is still an answer, and a `--json` caller that got
                // an empty stdout could not tell it from a crash.
                Document::new("reindex")
                    .failed()
                    .because(&held)
                    .field("sessions", 0)
                    .field("turns", 0)
                    .field("skipped", Vec::<serde_json::Value>::new())
                    .emit();
                return Err(Failure::Silent);
            }
            return Err(Failure::Operational(held));
        }
        Attempt::Acquired(guard) => guard,
    };

    // `open_up_to_date` first, so a store older than this build is brought
    // forward by the same code path rather than by a second one here. On an
    // up-to-date store it is a plain open and the rebuild below is the work.
    let mut store = reindex::open_up_to_date(&data_dir)?;
    let rebuilt = reindex::reindex(&mut store)?;

    if json {
        let mut document = Document::new("reindex")
            .field("sessions", rebuilt.sessions as i64)
            .field("turns", rebuilt.turns as i64)
            .field(
                "skipped",
                rebuilt
                    .failed
                    .iter()
                    .map(|(session_key, reason)| json!({"session_key": session_key, "reason": reason}))
                    .collect::<Vec<_>>(),
            );
        if !rebuilt.failed.is_empty() {
            document = document.failed().because(format!(
                "{} session(s) could not be rebuilt and were skipped",
                rebuilt.failed.len()
            ));
        }
        document.emit();
    } else {
        // Nothing on stdout without `--json`: this command produces no data, and
        // a caller must not have to filter a progress line out of the document.
        // In JSON mode the same counts are in the document, so printing them
        // here as well would be two accounts of one rebuild.
        eprintln!(
            "rebuilt {} turn(s) across {} session(s)",
            rebuilt.turns, rebuilt.sessions
        );
        for (session_key, reason) in &rebuilt.failed {
            eprintln!("{session_key}: {reason}");
        }
        if !rebuilt.failed.is_empty() {
            eprintln!(
                "{} session(s) could not be rebuilt and were skipped",
                rebuilt.failed.len()
            );
        }
    }

    if rebuilt.failed.is_empty() {
        return Ok(());
    }
    // Every undamaged session was still rebuilt; the exit code is what tells a
    // script the store is not whole.
    Err(Failure::Silent)
}
