//! The one pass driver (#31).
//!
//! A tree pass, a backfill and one named file are the same pass with a
//! different walk, so they run through one function in one order: take the
//! lock, recover, fill the summary, read the rows, walk, and write one `runs`
//! row. Each caller hands [`drive`] its own [`Walk`]. The steps only the tree
//! pass runs - the decision log drain, continuation links, labels,
//! observations, retention and the snapshot gate - are that walk's own, and
//! every step a walk does not name does nothing, so the backfill and a named
//! file stay out of them by construction.
//!
//! The driver starts no thread. The backfill's workers are started inside its
//! own walk, which is reached only by `terminus backfill`.

use std::path::Path;
use std::time::Instant;

use crate::error::{Error, Result};
use crate::ingest::lock::{self, Attempt};
use crate::ingest::pass::{PassOutcome, Summary};
use crate::ingest::{Ingested, Rows};
use crate::store::Store;

/// One kind of pass, as the driver sees it.
///
/// A trait rather than a closure or an enum of walks, because each walk carries
/// state the driver never reads: the tree pass's snapshot gate, the backfill's
/// worker count and thread set, a named file's own result.
pub(crate) trait Walk {
    /// After recovery and before the rows are read.
    fn before(&mut self, _store: &mut Store, _summary: &mut Summary) {}

    /// Which rows the walk reads: `None` for every row, `Some(key)` for one
    /// key's. A failure here is the walk failing.
    fn only(&mut self) -> Result<Option<String>> {
        Ok(None)
    }

    /// The walk itself. Only a failure of the pass as a whole comes back as
    /// `Err`.
    fn walk(&mut self, store: &mut Store, rows: &Rows, summary: &mut Summary) -> Result<()>;

    /// After the walk and before the row, whether or not the walk failed.
    fn after_walk(&mut self, _store: &mut Store, _summary: &mut Summary) {}

    /// The line the row gives a walk that failed, which goes last under
    /// `error`. A walk that would rather count its failure as one failed file
    /// records it in `summary` and returns `None`.
    fn failed(&mut self, _summary: &mut Summary, e: &Error) -> Option<String> {
        Some(format!("pass failed: {e}"))
    }

    /// After the row has committed and before the lock drops, on the pass's
    /// own store.
    fn after_row(&mut self, _store: &Store) {}
}

/// Run one pass of `walk` against the store under `data_dir`.
///
/// `LockHeld` when another process is mid-pass, with nothing written. A failed
/// recovery propagates with no row, as it always has: the store it would be
/// written to is the thing that could not be opened.
pub(crate) fn drive(data_dir: &Path, walk: &mut impl Walk) -> Result<PassOutcome> {
    let started = Instant::now();

    // Once, for the whole pass, and before the store is opened. A per-file
    // lock would be two thousand syscalls and would let another process
    // interleave between files, which is exactly the invariant `ingest_locked`
    // rests on: the guard makes this the only writer, so nothing changes the
    // store between its read and its write. A hook that fires during a
    // backfill loses this race and exits 0.
    let _guard = match lock::try_acquire(data_dir)? {
        Attempt::Held => return Ok(PassOutcome::LockHeld),
        Attempt::Acquired(guard) => guard,
    };

    // Recovery before anything is read, on every pass, a named file's
    // included: without it a pass would proceed against a watermark ahead of
    // what the file's committed blocks hold, `read_tail` would resume from an
    // offset the archive never reached, and the gap would be lost from the
    // archive in silence.
    let (mut store, recovery) = crate::recover::recover(data_dir)?;
    let mut summary = Summary {
        recovery,
        ..Summary::default()
    };

    // The summary's fill, once and ahead of the walk's first step. A store
    // from before the summary gets it whole here, on whichever walk runs
    // first, and a store that already has the key costs one lookup. A fill
    // that cannot run is a line in the `runs` row and never the pass's
    // failure: no key is written, so the next pass tries again.
    summary.summary_fill = fill_summary(&mut store).err().map(|e| {
        format!(
            "summary could not be filled: {}",
            e.to_string().replace('\n', " ")
        )
    });

    walk.before(&mut store, &mut summary);

    // The rows in one query, after recovery and the step before, and before
    // the walk (#85). Recovery's watermark sweep rewrites these rows, so
    // reading them any earlier would walk watermarks it has since changed. A
    // read that fails is the walk failing, and still leaves its `runs` row.
    let walked = walk
        .only()
        .and_then(|only| crate::ingest::read_rows(store.conn(), only.as_deref()))
        .and_then(|rows| walk.walk(&mut store, &rows, &mut summary));

    walk.after_walk(&mut store, &mut summary);

    summary.duration = started.elapsed();

    // One row, whatever happened, inserted once and never updated, so every
    // fault point inside the walk stays ahead of every `runs` write. A pass
    // that died writes it in a transaction of its own, after the per-file
    // transaction it was inside has rolled back - with no log file by design,
    // this row is the only place the failure can be seen. A pass that found
    // nothing new writes one too, because "the last ingest run" has to move.
    let fatal = walked
        .as_ref()
        .err()
        .and_then(|e| walk.failed(&mut summary, e));
    let recorded = crate::ingest::runs::insert(store.conn(), &summary, fatal);

    // The walk's error is the one worth returning. If the row could not be
    // written either, the store is unwritable and that is the same fact said
    // twice.
    walked?;
    recorded?;

    walk.after_row(&store);
    Ok(PassOutcome::Ran(summary))
}

/// The fill in a transaction of its own, which rolls back whole when dropped.
fn fill_summary(store: &mut Store) -> Result<()> {
    let tx = store.conn_mut().transaction()?;
    crate::summary::fill(&tx)?;
    Ok(tx.commit()?)
}

/// What one walked file did to the pass: counted, or recorded and skipped.
///
/// The sequential walk and the backfill's writer both end each file here, so a
/// file fails the same way on both. A named file does not: its failure comes
/// back to its caller, and it never sets the divergence flag.
pub(crate) fn tally(store: &Store, summary: &mut Summary, path: &Path, result: Result<Ingested>) {
    match result {
        Ok(Ingested::Committed(pass)) => {
            summary.files_committed += 1;
            summary.bytes_read += pass.bytes_read;
            summary.turns_added += pass.turns_added;
            // The pass-level fault point: a kill aimed here lands between two
            // files, with the walk half done and the store consistent. Timed
            // kills reach this region only by luck, and on a tree of small
            // transcripts nearly never. Inert without `testkit`.
            crate::ingest::fault::stall_after_files(summary.files_committed);
        }
        // Nothing new past the watermark.
        Ok(Ingested::UpToDate) => {}
        Err(e) => {
            // Recorded and skipped. The tree is the unit of work; one damaged
            // transcript is not a reason to archive none of it.
            let mut reason = e.to_string();

            // The divergence flag, and the only per-file failure that leaves a
            // mark on the store. Matched on the variant and never on the
            // message: a transcript at a non-UTF-8 path fails with the same io
            // kind and must flag nothing. The flag goes in a transaction of its
            // own - the bare statement autocommits - because the file's own
            // transaction has already rolled back, and it is what lets
            // `terminus verify` name the divergence instead of it living only
            // in a `runs` row nobody diffs.
            if matches!(
                e,
                Error::TranscriptDiverged { .. } | Error::TranscriptRewritten { .. }
            ) {
                if let Err(flag) = crate::ingest::flag_divergence(store.conn(), path) {
                    // Same note rather than a second failure: `files_failed`
                    // counts files, and one file must not become two.
                    reason.push_str(&format!(" (the flag could not be set: {flag})"));
                }
            }

            summary.failures.push((path.to_path_buf(), reason));
        }
    }
}
