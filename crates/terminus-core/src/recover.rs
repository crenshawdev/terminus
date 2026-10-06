//! Recovery, run at the top of every ingest run (ING-03, D-24).
//!
//! No repair command, no external supervisor, no stale-lock handling. The last
//! of those is not an omission: the ingest lock is an OS lock that dies with the
//! process (`crate::ingest::lock`), so there is no stale lock to recover from.
//! What is left is two things and only two.
//!
//! **The rebuild.** `reindex::open_up_to_date` brings a store older than this
//! build forward before a single transcript is read, so a walk never appends
//! turn rows in one shape beside rows written in another.
//!
//! **The watermark sweep.** A killed pass can leave a watermark claiming bytes
//! the committed blob does not hold - that is the state phase 1's crash harness
//! enumerates and the one STOR-02 says a store must never be left in. The sweep
//! reconciles it against `session_meta.uncompressed_len` rather than by
//! decompressing anything, which is what makes it one indexed pass over ~2,000
//! rows instead of 988 MB of zstd. Cheap enough to run on every entry point,
//! which is what ING-03's "every run" requires.
//!
//! Nothing here destroys archived bytes. A watermark is lowered so the next
//! pass re-reads the gap, and an orphaned watermark is removed because it claims
//! bytes nothing archived; `sessions` and `session_meta` are not touched.
//!
//! **The sweep's first arm is outside a session that was not captured under
//! `full`** (ING-07, phase 8 D-05). It compares a FILE offset against a count of
//! STORED bytes, and that comparison only carries information while the two
//! coordinate systems coincide - which is exactly what a capture mode's elision
//! ends. For a `lean` or `minimal` session `byte_offset > uncompressed_len` is
//! the NORMAL state, so left alone the sweep would "repair" every elided session
//! on every run, lowering its watermark and making the next pass re-read and
//! re-append bytes the blob already holds, forever.
//!
//! Nothing is lost by standing back. The blob and the watermark commit in one
//! transaction (STOR-02), so there is no divergence between them for this to
//! find; the arm was defence in depth against a state the transaction already
//! prevents. It does mean an elided session has no independent second witness
//! for its watermark, and there is none available without a second
//! `session_meta` column that D-13 deliberately did not add.
//!
//! The ORPHAN arm is untouched and must stay untouched. It compares nothing
//! against the stored bytes - a non-zero watermark for a path nothing archived
//! is wrong under every capture mode - and it is what removes the watermark of a
//! session retention deleted.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::store::Store;

/// A watermark that claimed more bytes than its blob holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredWatermark {
    pub session_key: String,
    pub from: u64,
    /// The uncompressed length the committed blob actually holds.
    pub to: u64,
}

/// A watermark naming no archived session at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedWatermark {
    pub session_key: String,
    pub byte_offset: u64,
}

/// What recovery repaired.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recovered {
    pub lowered: Vec<LoweredWatermark>,
    pub removed: Vec<RemovedWatermark>,
    pub duration: Duration,
}

impl Recovered {
    pub fn repaired(&self) -> bool {
        !self.lowered.is_empty() || !self.removed.is_empty()
    }

    /// One line per repair, for a `runs.error` or a `status` report.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for w in &self.lowered {
            out.push(format!(
                "{}: watermark {} was past the {} bytes the blob holds; lowered",
                w.session_key, w.from, w.to
            ));
        }
        for w in &self.removed {
            out.push(format!(
                "{}: watermark {} named no archived session; removed",
                w.session_key, w.byte_offset
            ));
        }
        out
    }
}

/// Open the store for work, recovering it first.
///
/// One callable with two callers - the tree pass and single-file
/// `ingest::run` - because ING-03 says every run and `ingest::run` is a run.
/// Wiring only the pass would leave `terminus ingest <path.jsonl>` proceeding
/// against a watermark ahead of what that file's committed blob holds, so
/// `read_tail` would resume from an offset the archive never reached and the
/// gap would be lost from the blob in silence.
///
/// Both callers already hold the ingest lock when they get here, and both call
/// it before any transcript is read.
pub fn recover(data_dir: &Path) -> Result<(Store, Recovered)> {
    let started = Instant::now();
    let mut store = crate::reindex::open_up_to_date(data_dir)?;
    let mut report = sweep_watermarks(&mut store)?;
    report.duration = started.elapsed();
    Ok((store, report))
}

/// Reconcile `watermarks` against what the committed blobs actually hold.
///
/// One transaction, so a store is never left half-reconciled.
fn sweep_watermarks(store: &mut Store) -> Result<Recovered> {
    let tx = store.conn_mut().transaction()?;

    // `session_meta.uncompressed_len`, not a decompression: the length is the
    // whole question, and reading it keeps the sweep an indexed join over two
    // primary keys.
    //
    // Restricted to sessions captured under `full` (ING-07, phase 8 D-05) - see
    // the module comment. Null is `full`: it is what every row written before
    // the column existed means.
    let lowered: Vec<LoweredWatermark> = tx
        .prepare(
            "SELECT w.transcript_path, w.byte_offset, m.uncompressed_len
             FROM watermarks w
             JOIN session_meta m ON m.session_key = w.transcript_path
             WHERE w.byte_offset > m.uncompressed_len
               AND coalesce(m.capture_mode, 'full') = 'full'
             ORDER BY w.transcript_path",
        )?
        .query_map([], |r| {
            Ok(LoweredWatermark {
                session_key: r.get(0)?,
                from: r.get::<_, i64>(1)? as u64,
                to: r.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;

    // A non-zero watermark for a path nothing archived. Zero is excluded
    // because it claims no bytes: it is what a pass writes for a transcript
    // with no complete record yet, and removing it would be churn, not repair.
    //
    // A session row present WITHOUT its `session_meta` row is deliberately not
    // in either query. That is the damaged state phase 1 refuses to guess at
    // and `verify` reports; touching its watermark here would be exactly the
    // guess D-12 forbids.
    let removed: Vec<RemovedWatermark> = tx
        .prepare(
            "SELECT w.transcript_path, w.byte_offset
             FROM watermarks w
             WHERE w.byte_offset <> 0
               AND NOT EXISTS (
                   SELECT 1 FROM sessions s WHERE s.session_key = w.transcript_path
               )
             ORDER BY w.transcript_path",
        )?
        .query_map([], |r| {
            Ok(RemovedWatermark {
                session_key: r.get(0)?,
                byte_offset: r.get::<_, i64>(1)? as u64,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;

    if lowered.is_empty() && removed.is_empty() {
        // Nothing to write. A healthy store leaves this transaction having read
        // two indexed queries and changed no row.
        drop(tx);
        return Ok(Recovered::default());
    }

    for repair in &lowered {
        tx.execute(
            "UPDATE watermarks SET byte_offset = ?2,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE transcript_path = ?1",
            rusqlite::params![repair.session_key, repair.to as i64],
        )?;
    }
    for repair in &removed {
        tx.execute(
            "DELETE FROM watermarks WHERE transcript_path = ?1",
            [&repair.session_key],
        )?;
    }

    tx.commit()?;
    Ok(Recovered {
        lowered,
        removed,
        duration: Duration::default(),
    })
}
