//! The mutating half of retention: doing what [`super::evaluate`] named.
//!
//! **One transaction per session**, for the reason `ingest::pass` gives for
//! per-file transactions: a kill loses one session's worth of work and never
//! leaves the store half way through a batch. A failure on one session is
//! recorded against that session and the loop carries on, the way `pass::walk`
//! records a per-file failure and keeps walking - retention must not be able to
//! wedge on one damaged row.

use rusqlite::Connection;

use crate::error::Result;
use crate::store::Store;

/// What one retention step did, and what it could not do.
///
/// The session keys are carried rather than only counted, so a caller can say
/// which sessions it acted on; [`Applied::lines`] summarizes instead, because
/// `runs.error` is read by a human and a hundred keys is not a report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Sessions whose blob was emptied.
    pub evicted: Vec<String>,
    /// Sessions removed from the store entirely.
    pub deleted: Vec<String>,
    /// Deletions that took an `observations` row with them.
    ///
    /// Counted and said out loud because it is the one thing a delete destroys
    /// that no rebuild reproduces: the judgment half of an observation is a
    /// paid model call, and `reindex` deliberately does not touch that table
    /// for exactly this reason (phase 7 D-02).
    pub observations_lost: usize,
    /// Eligible sessions the per-pass bound left for the next pass.
    pub over: usize,
    /// Sessions an excluded project holds, which retention passed over.
    pub excluded: usize,
    /// Whatever could not be done, said rather than raised. There is no log
    /// file, so these travel into `runs.error` with the pass's other notes.
    pub notes: Vec<String>,
}

impl Applied {
    /// Did this step do anything, or find anything worth saying?
    pub fn is_silent(&self) -> bool {
        self.evicted.is_empty()
            && self.deleted.is_empty()
            && self.over == 0
            && self.excluded == 0
            && self.notes.is_empty()
    }

    /// The reported lines, in the shape `pass::record_pass` writes.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.evicted.is_empty() {
            out.push(format!(
                "retention evicted {} session(s); each keeps its row, its metadata and every \
                 derived row",
                self.evicted.len()
            ));
        }
        if !self.deleted.is_empty() {
            let mut line = format!("retention deleted {} session(s)", self.deleted.len());
            if self.observations_lost > 0 {
                line.push_str(&format!(
                    ", removing {} observation row(s) with them - an observation is a model call \
                     no rebuild reproduces",
                    self.observations_lost
                ));
            }
            out.push(line);
        }
        if self.over > 0 {
            out.push(format!(
                "{} more session(s) are waiting on retention; the next ingest takes the next {}",
                self.over,
                super::MAX_PER_PASS
            ));
        }
        if self.excluded > 0 {
            out.push(format!(
                "{} session(s) in excluded projects were left alone; exclusion means never read, \
                 on the ingest and the read paths both",
                self.excluded
            ));
        }
        out.extend(self.notes.iter().cloned());
        out
    }
}

/// Evict and delete the sessions a [`super::Selection`] named.
pub fn apply(store: &mut Store, selection: &super::Selection) -> Applied {
    let mut out = Applied::default();
    for session_key in &selection.evict {
        match evict_one(store.conn_mut(), session_key) {
            Ok(()) => out.evicted.push(session_key.clone()),
            Err(e) => out
                .notes
                .push(format!("{session_key}: the eviction failed: {e}")),
        }
    }
    for session_key in &selection.delete {
        match delete_one(store.conn_mut(), session_key) {
            Ok(observation) => {
                out.deleted.push(session_key.clone());
                if observation {
                    out.observations_lost += 1;
                }
            }
            Err(e) => out
                .notes
                .push(format!("{session_key}: the deletion failed: {e}")),
        }
    }
    out
}

/// Empty one session's blob and mark it evicted (RET-02).
///
/// `x''` and never a null: `sessions.blob` is declared `BLOB NOT NULL` in
/// `crate::store::schema`, so the column takes zero bytes instead.
///
/// **`checksum` and `uncompressed_len` are left exactly as they are.** They
/// describe the bytes this session HAD, which is what makes an eviction
/// auditable, and lowering `uncompressed_len` here would put every evicted
/// session permanently above its own watermark - `recover::recover`'s
/// lowered-watermark sweep would then "repair" all of them on every pass,
/// forever.
fn evict_one(conn: &mut Connection, session_key: &str) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE sessions SET blob = x'' WHERE session_key = ?1",
        [session_key],
    )?;
    tx.execute(
        "UPDATE session_meta SET is_evicted = 1 WHERE session_key = ?1",
        [session_key],
    )?;
    tx.commit()?;
    Ok(())
}

/// Remove one session and everything keyed on it, answering whether an
/// `observations` row went with it.
///
/// **Child first, and that is not tidiness.** The bundled SQLite is compiled
/// with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`, so the references declared in
/// `schema::CREATE_SQL` really are enforced on every connection whatever the
/// pragma comment there says - `reindex` records the same fact and drops in
/// reverse for the same reason. `paths`, `entities` and `compaction_boundaries`
/// hang off `turns(id)`; `turns_fts` is contentless with `rowid` IS `turns.id`;
/// `observations` and `session_meta` hang off `sessions(session_key)`.
///
/// **`decisions` and `labels` are deliberately untouched.** `decisions` is
/// keyed on `session_id` with no declared reference and `labels.turn_id`
/// carries no foreign key on purpose (phase 6 D-03), and FEED-03's replay
/// history has to survive a deletion - it is the record of what injection
/// decided, which no blob ever contained and no reingest can rebuild.
///
/// The watermark goes last, keyed on the session key: `watermarks` is keyed on
/// the transcript path, and the session key IS that path (`recover::recover`
/// joins them on exactly that equality). Leaving it would let a row claim bytes
/// for a session that is gone - and `recover`'s orphan sweep would delete it on
/// the next pass and report a repair for damage retention caused.
fn delete_one(conn: &mut Connection, session_key: &str) -> Result<bool> {
    let tx = conn.transaction()?;
    const TURN_IDS: &str = "SELECT id FROM turns WHERE session_key = ?1";
    for statement in [
        format!("DELETE FROM paths WHERE turn_id IN ({TURN_IDS})"),
        format!("DELETE FROM entities WHERE turn_id IN ({TURN_IDS})"),
        format!("DELETE FROM compaction_boundaries WHERE turn_id IN ({TURN_IDS})"),
        format!("DELETE FROM turns_fts WHERE rowid IN ({TURN_IDS})"),
        "DELETE FROM turns WHERE session_key = ?1".to_owned(),
    ] {
        tx.execute(&statement, [session_key])?;
    }
    let observations = tx.execute(
        "DELETE FROM observations WHERE session_key = ?1",
        [session_key],
    )?;
    tx.execute(
        "DELETE FROM session_meta WHERE session_key = ?1",
        [session_key],
    )?;
    tx.execute("DELETE FROM sessions WHERE session_key = ?1", [session_key])?;
    tx.execute(
        "DELETE FROM watermarks WHERE transcript_path = ?1",
        [session_key],
    )?;
    tx.commit()?;
    Ok(observations > 0)
}
