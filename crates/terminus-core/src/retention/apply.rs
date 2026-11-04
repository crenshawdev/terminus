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
use crate::store::{archive, Store};

/// What one retention step did, and what it could not do.
///
/// The session keys are carried rather than only counted, so a caller can say
/// which sessions it acted on; [`Applied::notes`] summarizes instead, because
/// `runs.notes` is read by a human and a hundred keys is not a report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Sessions whose block rows were deleted.
    pub evicted: Vec<String>,
    /// Sessions removed from the store entirely.
    pub deleted: Vec<String>,
    /// Deletions that took an `observations` row with them.
    ///
    /// Counted and said out loud because it is the one thing a delete destroys
    /// that no rebuild reproduces: an observation's facts come off the session's
    /// block rows, and the delete takes those too, which is why `reindex`
    /// deliberately does not touch that table.
    pub observations_lost: usize,
    /// Logged prompts the deletions removed, each with its labels. Counted and
    /// said for the reason an observation is: nothing can rebuild one.
    pub decisions_lost: usize,
    /// Eligible sessions the per-pass bound left for the next pass.
    pub over: usize,
    /// Sessions an excluded project holds, which retention passed over.
    pub excluded: usize,
    /// Sessions selected for deletion whose transcript was back on disk when
    /// the delete came, so [`apply`] kept them (#43). Not a failure and not in
    /// `notes`: the session is simply still archived.
    pub kept: usize,
    /// Whatever could not be done, said rather than raised. There is no log
    /// file, so these travel into `runs.error` with the pass's other failures.
    pub notes: Vec<String>,
}

impl Applied {
    /// Did this step do anything, or find anything worth saying?
    pub fn is_silent(&self) -> bool {
        self.evicted.is_empty()
            && self.deleted.is_empty()
            && self.over == 0
            && self.excluded == 0
            && self.kept == 0
            && self.notes.is_empty()
    }

    /// What could not be done, for `runs.error`.
    pub fn errors(&self) -> Vec<String> {
        self.notes.clone()
    }

    /// What this step did, for `runs.notes`: a healthy pass with retention
    /// configured says it, and none of it is a failure.
    pub fn notes(&self) -> Vec<String> {
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
                    ", removing {} observation row(s) with them - the session's archive went \
                     too, so no rebuild reproduces them",
                    self.observations_lost
                ));
            }
            out.push(line);
        }
        if self.decisions_lost > 0 {
            out.push(format!(
                "retention removed {} logged prompt(s) and their labels with those sessions; \
                 terminus stats and terminus replay no longer count them",
                self.decisions_lost
            ));
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
        if self.kept > 0 {
            out.push(format!(
                "{} session(s) selected for deletion were kept because their transcript was back \
                 on disk when the delete came; they are still archived",
                self.kept
            ));
        }
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
            Ok(Some(deleted)) => {
                out.deleted.push(session_key.clone());
                if deleted.observations > 0 {
                    out.observations_lost += 1;
                }
                out.decisions_lost += deleted.decisions;
            }
            Ok(None) => out.kept += 1,
            Err(e) => out
                .notes
                .push(format!("{session_key}: the deletion failed: {e}")),
        }
    }
    out
}

/// Delete one session's block rows and mark it evicted.
///
/// The `sessions` row stays, so `session_no` and every derived row keyed on
/// it stay too.
///
/// **`checksum` and `uncompressed_len` are left exactly as they are.** They
/// describe the bytes this session HAD, which is what makes an eviction
/// auditable, and lowering `uncompressed_len` here would put every evicted
/// session permanently above its own watermark - `recover::recover`'s
/// lowered-watermark sweep would then "repair" all of them on every pass,
/// forever.
fn evict_one(conn: &mut Connection, session_key: &str) -> Result<()> {
    let tx = conn.transaction()?;
    archive::remove(&tx, session_key)?;
    tx.execute(
        "UPDATE session_meta SET is_evicted = 1 WHERE session_key = ?1",
        [session_key],
    )?;
    tx.commit()?;
    Ok(())
}

/// What one deletion took that nothing can rebuild.
pub(crate) struct Deleted {
    observations: usize,
    decisions: usize,
}

/// Remove one session and everything keyed on it, answering what went with it,
/// or `None` when the session was kept.
///
/// **It asks about the transcript again first (#43).** [`super::evaluate`] named
/// this session when its transcript was gone, and a transcript can come back
/// between that and now. So the stored `session_meta.transcript_path` is read
/// again, in the same transaction as the delete it guards, and a session whose
/// transcript is back is left untouched and counted on [`Applied::kept`]. The
/// stored path, never the key: after a moved `.claude` directory the key names a
/// path that is gone. The check is per session, not once at the top of
/// [`apply`], so a transcript that comes back partway through a batch is still
/// caught. It narrows the window between selection and delete without closing
/// it, because the ingest lock guards the store and not the transcript tree.
///
/// **Child first, and that is not tidiness.** The derived tables and their
/// order are `reindex::clear_derived`'s, which this calls first. The bundled
/// SQLite is compiled with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`, so the references
/// declared in `schema::CREATE_SQL` really are enforced on every connection
/// whatever the pragma comment there says. What is retention's own:
/// `observations` and `session_meta` hang off `sessions(session_key)` and
/// `blocks` off `sessions(session_no)`, so they go after the derived rows and
/// before `sessions`.
///
/// **The session's `decisions` and their `labels` go too.** A decision holds a
/// whole prompt, so a deletion that left it would keep the text the user asked
/// retention to remove. See [`delete_decisions`] for which ones belong to a
/// session. `terminus stats` and `terminus replay` lose them, and the note says
/// so.
///
/// The watermark goes last, keyed on the session key: `watermarks` is keyed on
/// the transcript path, and the session key IS that path (`recover::recover`
/// joins them on exactly that equality). Leaving it would let a row claim bytes
/// for a session that is gone - and `recover`'s orphan sweep would delete it on
/// the next pass and report a repair for damage retention caused.
fn delete_one(conn: &mut Connection, session_key: &str) -> Result<Option<Deleted>> {
    let tx = conn.transaction()?;
    let transcript_path: String = tx.query_row(
        "SELECT transcript_path FROM session_meta WHERE session_key = ?1",
        [session_key],
        |r| r.get(0),
    )?;
    if !super::transcript_is_gone(&transcript_path) {
        return Ok(None);
    }
    let deleted = delete_session(&tx, session_key)?;
    tx.commit()?;
    Ok(Some(deleted))
}

/// Delete one session and every row keyed on it, inside the caller's
/// transaction: the derived rows through `reindex::clear_derived`, then the
/// rows retention owns, child first.
pub(crate) fn delete_session(tx: &Connection, session_key: &str) -> Result<Deleted> {
    let session_id: Option<String> = tx.query_row(
        "SELECT session_id FROM session_meta WHERE session_key = ?1",
        [session_key],
        |r| r.get(0),
    )?;
    crate::reindex::clear_derived(tx, session_key)?;
    let observations = tx.execute(
        "DELETE FROM observations WHERE session_key = ?1",
        [session_key],
    )?;
    tx.execute(
        "DELETE FROM session_meta WHERE session_key = ?1",
        [session_key],
    )?;
    archive::remove(tx, session_key)?;
    tx.execute("DELETE FROM sessions WHERE session_key = ?1", [session_key])?;
    tx.execute(
        "DELETE FROM watermarks WHERE transcript_path = ?1",
        [session_key],
    )?;
    let decisions = match session_id {
        Some(session_id) => delete_decisions(tx, &session_id)?,
        None => 0,
    };
    Ok(Deleted {
        observations,
        decisions,
    })
}

/// Delete the decisions of `session_id` and their labels, unless a top-level
/// session with that id is still archived. Answers how many decisions went.
///
/// A decision belongs to a session through `session_id`, and a subagent's
/// transcript reports its parent's id. So deleting a subagent's session leaves
/// the parent's decisions alone, and deleting the parent takes them even when
/// one of its subagents is still archived, because those prompts were typed in
/// the parent.
///
/// A label on another session's decision can name a turn of the deleted
/// session. It stays with that decision, and it holds an id and a verdict.
fn delete_decisions(tx: &Connection, session_id: &str) -> Result<usize> {
    let still_archived: bool = tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM session_meta
              WHERE session_id = ?1 AND parent_session_key IS NULL
         )",
        [session_id],
        |r| r.get(0),
    )?;
    if still_archived {
        return Ok(0);
    }
    tx.execute(
        "DELETE FROM labels
          WHERE decision_id IN (SELECT id FROM decisions WHERE session_id = ?1)",
        [session_id],
    )?;
    Ok(tx.execute("DELETE FROM decisions WHERE session_id = ?1", [session_id])?)
}
