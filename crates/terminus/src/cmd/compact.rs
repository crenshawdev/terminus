//! `terminus compact`: reclaim the space retention already freed (RET-04).
//!
//! **It deletes nothing and has no opinion about what should be kept.** Deleting
//! a session is retention's job and retention's only, inside the ingest pass
//! (D-10); this command reclaims the pages a delete left behind. That split is
//! why `compact` takes no age, no project and no policy: there is nothing here
//! to configure, because there is no choice here to make.
//!
//! **`VACUUM` and then `PRAGMA wal_checkpoint(TRUNCATE)`, always both (D-08).**
//! `VACUUM` rebuilds the database file without its free pages - and on a WAL
//! store it writes the whole rebuilt file THROUGH the WAL, so the pages it just
//! reclaimed reappear in `terminus.db-wal`. Measured 2026-08-22 on a synthetic
//! store: after `VACUUM` the database halved to 7,540,736 bytes while the WAL
//! grew to 7,584,952, a total UP from 15.06 MB to 15.13 MB; the truncating
//! checkpoint is what turns that into 7,540,736 and nothing else. `cmd::status`
//! sums all three files for its `size_bytes`, so without the checkpoint the
//! number this product itself reports for the store goes up immediately after a
//! compaction.
//!
//! **Under the ingest lock, taken before anything is opened.** A plain `VACUUM`
//! fails outright while another connection holds a write transaction, so a
//! compaction racing a hook-spawned pass is a raw "database is locked" rather
//! than an answer. The `--purge` arm of `cmd::uninstall` takes the lock for the
//! same reason and reports a held one the same way. Unlike a contended `ingest`,
//! which exits 0 because the next hook spawn will catch what it skipped, a
//! contended `compact` exits 1: it was asked for explicitly, and a compaction
//! that did not happen is an operational failure rather than an empty answer.

use terminus_core::ingest::{lock, Attempt};
use terminus_core::store::{Store, DB_FILE_NAME};

use super::json::Document;
use super::{footprint, human, read, Failure, Footprint};

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "compact";

pub fn run(json: bool) -> Result<(), Failure> {
    let data_dir = super::data_dir()?;

    // Before the lock, because `lock::try_acquire` creates the data directory
    // it locks in. A machine that has never ingested must not acquire a data
    // directory by being asked to compact one, and "there is nothing to
    // reclaim" is a true answer to the question rather than a failure of it.
    if !data_dir.join(DB_FILE_NAME).exists() {
        let reason = format!(
            "no terminus store at {}; there is nothing to reclaim",
            data_dir.display()
        );
        return read::empty(
            document(Footprint::default(), Footprint::default()),
            &reason,
            json,
        );
    }

    let _guard = match lock::try_acquire(&data_dir)? {
        Attempt::Held => {
            // Measured rather than assumed to be zero: the point of saying it
            // is that nothing moved, and a caller comparing this document with
            // the next one needs the same two numbers either way.
            let unchanged = footprint(&data_dir);
            let held = format!(
                "an ingest is running: another terminus process holds {}, so nothing was \
                 compacted; try again when it finishes",
                data_dir.join(lock::LOCK_FILE_NAME).display()
            );
            if json {
                document(unchanged, unchanged)
                    .failed()
                    .because(&held)
                    .emit();
                return Err(Failure::Silent);
            }
            return Err(Failure::Operational(held));
        }
        Attempt::Acquired(guard) => guard,
    };

    // A write open, and deliberately not the read path: this command rewrites
    // the database file. `Store::open` also brings an older store forward,
    // which is the right thing to do under a lock we already hold - compacting
    // is not the moment to leave a store half upgraded.
    let store = Store::open(&data_dir)?;
    let conn = store.conn();

    // Measured with the connection open, which is when `-wal` and `-shm` exist
    // at all, so `before` and `after` are the same measurement of the same
    // store in the same state and the difference between them means something.
    let before = footprint(&data_dir);

    // `VACUUM` cannot run inside a transaction, so it goes to the connection
    // directly. `execute_batch` issues it without wrapping one, which
    // `Connection::execute` would not guarantee for a statement that returns no
    // rows through a prepared handle.
    conn.execute_batch("VACUUM")
        .map_err(|e| Failure::Operational(format!("the store could not be compacted: {e}")))?;
    // Returns a row - busy, log pages, checkpointed pages - so it is queried
    // rather than executed. A non-zero `busy` means a reader held the WAL open
    // and the truncate did not complete; the compaction still happened, and the
    // `after` numbers below report what is actually on disk rather than what
    // was intended.
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(|e| {
            Failure::Operational(format!("the write-ahead log could not be truncated: {e}"))
        })?;

    let after = footprint(&data_dir);

    if json {
        document(before, after).emit();
        return Ok(());
    }

    println!(
        "before         {} byte(s) ({})",
        before.total(),
        human(before.total())
    );
    println!(
        "after          {} byte(s) ({})",
        after.total(),
        human(after.total())
    );
    println!(
        "reclaimed      {} byte(s) ({})",
        reclaimed(before, after),
        human(reclaimed(before, after).max(0) as u64)
    );
    Ok(())
}

/// The envelope for this command.
///
/// `before` and `after` carry the three files apart as well as their sum,
/// because the sum alone cannot be read: a `VACUUM` whose WAL was never
/// truncated and a store that genuinely grew produce the same total, and only
/// the split says which happened.
fn document(before: Footprint, after: Footprint) -> Document {
    Document::new(COMMAND)
        .field("before", before.to_value())
        .field("after", after.to_value())
        .field("reclaimed_bytes", reclaimed(before, after))
}

/// Signed, and not a `u64`. A compaction that reclaimed nothing and left the
/// store fractionally larger is a real outcome worth reporting as what it is;
/// saturating it to zero would report a store that grew as a store that held
/// still.
fn reclaimed(before: Footprint, after: Footprint) -> i64 {
    before.total() as i64 - after.total() as i64
}
