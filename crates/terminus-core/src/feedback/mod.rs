//! The feedback loop: what injection decided, and what it turned out to be
//! worth (FEED-01..FEED-04).
//!
//! This half is the drain. The prompt path writes one file per decision under
//! the data directory ([`crate::inject::decision`]) because a hook must never
//! write SQLite (D-01), and an ingest pass - which already holds the lock, on a
//! process nobody is waiting for - moves those files into the `decisions` table
//! and deletes them.
//!
//! **Before the walk, on purpose.** The drain runs after recovery and before a
//! single transcript is read, which is what makes the watermark it stamps mean
//! "the archive as it stood when this pass began" rather than "plus whatever
//! this pass just admitted" (D-10). A record whose prompt never opened the
//! store has no watermark of its own, and this is the tightest bound anything
//! can give it.
//!
//! The other half is [`outcomes`], and it runs at the far end of the same pass,
//! **after** the walk: what a decision turned out to be worth is a statement
//! about the turns that followed it, so it is evaluated once those turns are in
//! the store rather than once they are not.

pub mod finalize;
pub mod label;
pub mod replay;
pub mod stats;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::Connection;

use crate::error::Result;
use crate::inject::decision::{self, Decision};
use crate::inject::state;

/// How old a writer's `.tmp` has to be before the drain deletes it.
///
/// A hook process lives for its `DRAIN_DEADLINE` (500 ms) and `INJECT_DEADLINE`
/// (50 ms), both in `crates/terminus/src/cmd/hook.rs`, plus its startup. Ten
/// minutes is over a thousand times that, so no hook still writing owns a file
/// this old, and it is short enough that an abandoned copy of a prompt is gone
/// by the first pass after ten minutes. It lives here and not beside the
/// deadlines because the drain is in this crate and they are in the binary.
const ABANDONED_AFTER: Duration = Duration::from_secs(10 * 60);

/// What one drain did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drained {
    /// Records that became rows.
    pub decisions: usize,
    /// Files that were removed without becoming rows, and why. There is no log
    /// file, so these travel into `runs.error` with the pass's other notes.
    pub discarded: Vec<(PathBuf, String)>,
}

impl Drained {
    /// One reported line per discarded file, in the shape `record_pass` writes.
    pub fn lines(&self) -> Vec<String> {
        self.discarded
            .iter()
            .map(|(path, reason)| crate::ingest::pass::note(path, reason))
            .collect()
    }
}

/// What one pass's outcome step did (FEED-02).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labeled {
    /// Sessions the idle rule closed on this pass, never the number that are
    /// closed: an already-final session is not touched again.
    pub finalized: usize,
    /// Labels written on this pass, by label, in a stable order. A label with
    /// no rows is absent rather than zero.
    pub labels: Vec<(String, usize)>,
    /// Whatever could not be done, said rather than raised. There is no log
    /// file, so these travel into `runs.error` with the pass's other notes.
    pub notes: Vec<String>,
}

impl Labeled {
    /// How many labels of one kind this pass wrote.
    pub fn count(&self, label: &str) -> usize {
        self.labels
            .iter()
            .find(|(name, _)| name == label)
            .map_or(0, |(_, count)| *count)
    }

    /// What could not be done, for `runs.error`.
    pub fn errors(&self) -> Vec<String> {
        self.notes.clone()
    }

    /// What was labelled when anything was, for `runs.notes`.
    pub fn notes(&self) -> Vec<String> {
        if self.labels.is_empty() {
            return Vec::new();
        }
        let counts: Vec<String> = self
            .labels
            .iter()
            .map(|(label, count)| format!("{count} {label}"))
            .collect();
        vec![format!("labelled {}", counts.join(", "))]
    }
}

/// Turn the walk that just finished into outcomes: close the idle sessions,
/// then label the decisions they closed.
///
/// **Nothing here fails a pass.** The archive is the work; a labelling step that
/// could not run costs a number in `terminus stats` and is worth saying, not
/// worth losing a tree walk over - the same call the drain makes at the other
/// end of the pass. Labelling still runs when finalizing failed: it operates on
/// whatever is already marked final, which is a smaller set and not a wrong one.
pub fn outcomes(conn: &mut Connection) -> Labeled {
    let mut labeled = Labeled::default();
    match finalize::finalize(conn) {
        Ok(closed) => labeled.finalized = closed,
        Err(e) => labeled
            .notes
            .push(format!("the idle sessions could not be closed: {e}")),
    }
    match label::label(conn) {
        Ok(counts) => labeled.labels = counts,
        Err(e) => labeled
            .notes
            .push(format!("the decisions could not be labelled: {e}")),
    }
    labeled
}

/// Move every decision file into the `decisions` table.
///
/// **One transaction for the batch, and the files are deleted only after it
/// commits.** A kill before the commit leaves every file where it was and
/// inserts nothing, so the next pass drains them; the reverse order would delete
/// the only copy of a record whose insert then rolled back. The window that
/// remains - killed after the commit and before the unlink - re-inserts those
/// records on the next pass, which is a duplicate row in a log rather than a
/// lost decision, and it is the cheaper of the two failures.
///
/// A file that does not parse is deleted and named. It is not a record this
/// build can read and never will be (there is no migration - see
/// [`crate::inject::decision`]), so leaving it would mean re-reading it on every
/// pass forever.
///
/// Every drain first sweeps the writers' abandoned `.tmp` files (see
/// [`sweep`]), including one that finds no record or whose insert fails.
pub fn drain(conn: &mut Connection, data_dir: &Path) -> Result<Drained> {
    sweep(data_dir);
    let found = decision::read_all(data_dir);
    if found.is_empty() {
        return Ok(Drained::default());
    }

    // Read once, before anything is inserted and before the walk: this is the
    // bound D-10 wants for a record that never opened the store.
    let watermark: Option<i64> = conn
        .query_row("SELECT max(session_no) FROM sessions", [], |r| r.get(0))
        .unwrap_or(None);

    let mut drained = Drained::default();
    let tx = conn.transaction()?;
    for entry in &found {
        match &entry.decision {
            Some(decision) => {
                insert(&tx, decision, watermark)?;
                drained.decisions += 1;
            }
            None => drained.discarded.push((
                entry.path.clone(),
                "not a decision record this build can read".to_owned(),
            )),
        }
    }
    tx.commit()?;

    for entry in &found {
        if let Err(e) = std::fs::remove_file(&entry.path) {
            // Worth saying: the file will be drained again on the next pass,
            // and that is where a duplicate row comes from.
            drained
                .discarded
                .push((entry.path.clone(), format!("could not be removed: {e}")));
        }
    }
    Ok(drained)
}

/// Delete the writers' temporaries once they are older than
/// [`ABANDONED_AFTER`].
///
/// A decision or state write cut off at the hook's deadline leaves its `.tmp`
/// behind, and a decision's holds the whole prompt. Only names the writers make
/// are candidates, and only regular files: `symlink_metadata`, so a symlink with
/// such a name is left alone rather than judged by its target. This is
/// housekeeping, so nothing it deletes or fails to delete is reported.
fn sweep(data_dir: &Path) {
    let writers = [
        (
            decision::DIR_NAME,
            decision::TEMPORARY_PREFIX,
            decision::TEMPORARY_SUFFIX,
        ),
        (
            state::DIR_NAME,
            state::TEMPORARY_PREFIX,
            state::TEMPORARY_SUFFIX,
        ),
    ];
    for (dir, prefix, suffix) in writers {
        let Ok(entries) = std::fs::read_dir(data_dir.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !(name.starts_with(prefix) && name.ends_with(suffix)) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_file() && older(meta.modified().ok()) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Whether `modified` is further back than [`ABANDONED_AFTER`]. An mtime that
/// could not be read, or is in the future, is not.
fn older(modified: Option<SystemTime>) -> bool {
    modified
        .and_then(|at| at.elapsed().ok())
        .is_some_and(|age| age > ABANDONED_AFTER)
}

/// One record, as a row.
///
/// The wall clock arrives as unix milliseconds and SQLite formats it, so the
/// column carries exactly the shape every other stored timestamp does and this
/// crate grows no second date implementation (see
/// [`crate::inject::decision`]). The list-shaped fields are JSON documents: they
/// are read back whole by replay and stats and joined on by nothing.
///
/// `compacted` is always written as 1 or 0 and `dropped` always as a document,
/// an empty array for an ordinary decision. Null in either column is kept for
/// the rows drained before the columns existed, whose values are unknown.
fn insert(conn: &Connection, decision: &Decision, watermark: Option<i64>) -> Result<()> {
    conn.execute(
        "INSERT INTO decisions (
            session_id, ts, cwd, prompt, watermark_session_no, chars_injected,
            spellings, candidates, injected, suppressed, thresholds,
            compacted, dropped
         ) VALUES (
            ?1, strftime('%Y-%m-%dT%H:%M:%fZ', ?2 / 1000.0, 'unixepoch'), ?3, ?4,
            ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
         )",
        rusqlite::params![
            decision.session_id,
            decision.at_ms,
            decision.cwd,
            decision.prompt,
            // The prompt path's own bound when it had one; this pass's
            // otherwise (D-10).
            decision.watermark_session_no.or(watermark),
            decision.chars_injected as i64,
            document(&decision.spellings),
            document(&decision.candidates),
            document(&decision.injected),
            document(&decision.suppressed),
            document(&decision.thresholds),
            i64::from(decision.compacted),
            document(&decision.dropped),
        ],
    )?;
    Ok(())
}

/// One field as a JSON document, or `null` where it cannot be serialized.
///
/// A field that will not serialize is one column of one row of a log, and
/// failing the drain over it would cost the archive its pass.
fn document<T: serde::Serialize>(value: &T) -> Option<String> {
    serde_json::to_string(value).ok()
}
