//! The `runs` table: the one row every pass leaves behind.
//!
//! The column list lives here and nowhere else. The driver writes the row the
//! same way for a tree pass, a backfill and one named file, so the last run
//! `terminus status` reports means the same thing whichever of the three ran.

use rusqlite::OptionalExtension;

use crate::error::Result;
use crate::ingest::pass::{note, Summary};

/// The last run, as `terminus status` prints it and `terminus doctor` judges
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub started_at: String,
    /// Null in the column reads as 0.
    pub duration_ms: i64,
    pub files_seen: i64,
    pub files_committed: i64,
    pub files_failed: i64,
    pub bytes_read: i64,
    pub turns_added: i64,
    pub error: Option<String>,
    pub notes: Option<String>,
}

/// The newest `runs` row, by id (#31).
///
/// `Ok(None)` is an empty table and `Err` is a read that failed, so "no run
/// yet" and "the query failed" stay two answers without a separate count. The
/// one reader `status` and `doctor` both go through, so the two cannot read
/// the row into different shapes.
pub fn last(conn: &rusqlite::Connection) -> Result<Option<Run>> {
    Ok(conn
        .query_row(
            "SELECT started_at, coalesce(duration_ms, 0), files_seen, files_committed,
                    files_failed, bytes_read, turns_added, error, notes
             FROM runs ORDER BY id DESC LIMIT 1",
            [],
            |r| {
                Ok(Run {
                    started_at: r.get(0)?,
                    duration_ms: r.get(1)?,
                    files_seen: r.get(2)?,
                    files_committed: r.get(3)?,
                    files_failed: r.get(4)?,
                    bytes_read: r.get(5)?,
                    turns_added: r.get(6)?,
                    error: r.get(7)?,
                    notes: r.get(8)?,
                })
            },
        )
        .optional()?)
}

/// The one `runs` row a pass leaves behind.
///
/// `runs.error` and `runs.notes` are the only textual channel this product has -
/// there is no log file, by design, and `terminus status` surfaces this table -
/// so everything worth a human's attention goes in one of them. `error` holds
/// what went wrong: the path and reason of every file that was skipped, every
/// directory that could not be listed, every step that could not run, and the
/// failure that ended the pass if one did. `notes` holds what a healthy pass
/// did worth saying - every watermark recovery repaired, first because
/// recovery runs first, then labels written, finalized sessions left for the
/// next pass to observe, and sessions retention evicted or deleted - so
/// `doctor` can flag `error` alone, and a pass that only recovered shows no
/// error (#96). Each stays null when there is genuinely nothing to say.
///
/// `fatal` is the line that says the pass as a whole failed, already worded by
/// the walk that failed, and it goes last.
pub(crate) fn insert(
    conn: &rusqlite::Connection,
    summary: &Summary,
    fatal: Option<String>,
) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    for (path, reason) in summary.failures.iter().chain(&summary.unreadable) {
        errors.push(note(path, reason));
    }
    errors.extend(summary.feedback.lines());
    errors.extend(summary.continuations.clone());
    errors.extend(summary.outcomes.errors());
    errors.extend(summary.observations.errors());
    errors.extend(summary.retention.errors());
    errors.extend(fatal);
    let mut notes = summary.recovery.lines();
    notes.extend(summary.feedback.notes());
    notes.extend(summary.outcomes.notes());
    notes.extend(summary.observations.notes());
    notes.extend(summary.retention.notes());
    let joined = |lines: Vec<String>| (!lines.is_empty()).then(|| lines.join("\n"));
    let error = joined(errors);
    let notes = joined(notes);

    let elapsed = summary.duration;
    conn.execute(
        "INSERT INTO runs (
            started_at, finished_at, duration_ms, files_seen, bytes_read,
            turns_added, error, files_committed, files_failed, notes
         ) VALUES (
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
         )",
        rusqlite::params![
            format!("-{} seconds", elapsed.as_secs_f64()),
            elapsed.as_millis() as i64,
            summary.files_walked as i64,
            summary.bytes_read as i64,
            summary.turns_added as i64,
            error,
            summary.files_committed as i64,
            summary.failures.len() as i64,
            notes,
        ],
    )?;
    Ok(())
}
