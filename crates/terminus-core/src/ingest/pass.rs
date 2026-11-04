//! One ingest pass over a whole transcript tree.
//!
//! The driver (`ingest::driver`) takes the lock once and opens the store once,
//! for the whole walk, and writes the pass's one `runs` row after it. The
//! **transactions** are still per-file, and on purpose rather than as an
//! optimization: a write transaction held across the walk stops MCP readers,
//! which is the reason redb was rejected, and per-file commits are the
//! granularity the crash harness (`crates/terminus/tests/crash.rs`) proves
//! atomic. They also bound what a kill can lose to one file, and real files are
//! p50 290 KB, p90 1.03 MB, p99 3.1 MB.
//!
//! A per-file failure is recorded against that file's path and skipped, never
//! propagated. Two hard-error paths in ingest reach this loop -
//! `Existing::read` refusing a session archived with no `session_meta` row, and
//! `read_tail` refusing a file shorter than its watermark - and either one
//! stopping the pass would let a single damaged session wedge every future
//! ingest of every other transcript in the tree. That is the exact failure gate
//! fix `3e5d9ff` closed for `reindex`, and it is worth more here: the tree is
//! two thousand files.
//!
//! One of those two is also the only per-file failure that writes anything: a
//! transcript shorter than its watermark leaves the divergence flag on that
//! session, so the divergence reaches `terminus verify` rather than living only
//! in a `runs` row nobody diffs. The archive itself is still untouched.
//!
//! The walk records a file's failure and goes on. A named file's failure, from
//! `ingest::run`, comes back to its caller once its row is written: the skip
//! lives here, in the walk, and not in `ingest_locked` - a caller that named
//! one file deserves the error it asked about.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Config;
use crate::discover;
use crate::error::Result;
use crate::ingest::driver::{self, Walk};
use crate::ingest::Claim;
use crate::project::Resolver;
use crate::store::Store;

/// What one pass over the tree did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Transcripts discovery yielded, whatever became of them.
    pub files_walked: usize,
    /// Transcripts that committed new bytes. A file with nothing new past its
    /// watermark is walked and not committed, which is the steady state.
    pub files_committed: usize,
    pub bytes_read: u64,
    pub turns_added: usize,
    /// Per-file failures: the path and why it was skipped.
    pub failures: Vec<(PathBuf, String)>,
    /// Directories that could not be listed.
    pub unreadable: Vec<(PathBuf, String)>,
    /// Project directories the config excluded, never listed and never opened.
    pub excluded: Vec<PathBuf>,
    /// What recovery repaired before the walk began.
    pub recovery: crate::recover::Recovered,
    /// The line saying the continuation-link sweep could not finish, when it
    /// could not. It reaches `runs.error`; a healthy sweep leaves it `None`
    /// and says nothing.
    pub continuations: Option<String>,
    /// What the decision log drained into the store before the walk.
    pub feedback: crate::feedback::Drained,
    /// What the walk's own turns then made of those decisions.
    pub outcomes: crate::feedback::Labeled,
    /// What the sessions this pass closed had to say for themselves.
    pub observations: crate::observe::Observed,
    /// What retention evicted, deleted and left for the next pass. Empty and
    /// free while retention is off, which is the default and the state every
    /// user starts in.
    pub retention: crate::retention::Applied,
    /// What the rolling snapshot step did, after the lock dropped.
    ///
    /// Its notes do NOT reach `runs.error`: the row has already committed by
    /// the time this runs, so the caller prints them on stderr instead.
    pub snapshot: Snapshotted,
    pub duration: Duration,
}

/// `meta` key holding when the last rolling snapshot was taken, as unix
/// seconds.
///
/// A row in `meta` rather than the newest file's name, because the gate has to
/// be answerable without listing a directory - and because a user who copies
/// their snapshots elsewhere has not thereby asked for a new one on the next
/// prompt.
pub const META_LAST_SNAPSHOT: &str = "last_snapshot_at";

/// What one pass's snapshot step did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshotted {
    /// Where the copy landed, when the interval had elapsed and it worked.
    pub taken: Option<PathBuf>,
    /// Older snapshots the prune removed.
    pub pruned: usize,
    /// Whatever could not be done, said rather than raised. These reach stderr
    /// and not `runs.error`: this step runs AFTER that row commits.
    pub notes: Vec<String>,
}

impl Summary {
    /// Files walked that did not commit and did not fail: nothing new to read.
    pub fn files_unchanged(&self) -> usize {
        self.files_walked - self.files_committed - self.failures.len()
    }
}

/// What an invocation of [`run_with`] did.
///
/// `Ran` is much larger than `LockHeld` and stays unboxed. One of these is
/// produced per process - a pass is the whole of what `terminus ingest` does -
/// so the lint's allocation would buy two hundred bytes once and cost every
/// caller a deref, on a type `crates/terminus/src/cmd/ingest.rs` and four test
/// files already match by value.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassOutcome {
    /// Another process is mid-pass. Nothing was written, and this is a success:
    /// the hook spawn is the scheduler and a second invocation is expected.
    LockHeld,
    /// The pass walked the tree.
    Ran(Summary),
}

/// Walk every configured transcript root and ingest what is beneath it.
pub fn run(data_dir: &Path) -> Result<PassOutcome> {
    let config = Config::load()?;
    run_with(data_dir, &config)
}

/// The pass, against a config the caller already has.
///
/// The seam tests use, so a test never has to resolve a real transcript root.
///
/// Two halves. The driver runs everything that touches the archive, under the
/// ingest lock, ending with the `runs` row and with whether a snapshot is due,
/// asked of the pass's own store. The snapshot itself comes after it returns,
/// once the guard and the store are dropped and that row has committed, so a
/// snapshot that failed is a note and never a failed pass. When none is due,
/// nothing here opens the store a second time.
pub fn run_with(data_dir: &Path, config: &Config) -> Result<PassOutcome> {
    let mut tree = Tree {
        data_dir,
        config,
        transcripts: Vec::new(),
        gate: Ok(None),
    };
    let mut summary = match driver::drive(data_dir, &mut tree)? {
        PassOutcome::LockHeld => return Ok(PassOutcome::LockHeld),
        PassOutcome::Ran(summary) => summary,
    };
    let gate = tree.gate;

    // The rolling snapshot: the guard and the store handle are gone, the `runs`
    // row has committed, and nothing below can fail the pass. A snapshot is a
    // copy of what is already safely archived, so one that could not be
    // written is a note - failing the pass over it would stop archiving the
    // transcripts the copy exists to protect.
    summary.snapshot = match gate {
        Ok(None) => Snapshotted::default(),
        Ok(Some(now)) => roll(data_dir, config, now),
        Err(detail) => Snapshotted {
            notes: vec![format!("no snapshot was taken: {detail}")],
            ..Snapshotted::default()
        },
    };
    Ok(PassOutcome::Ran(summary))
}

/// Is a rolling snapshot due? Asked of the pass's own store, inside the lock,
/// after the `runs` row has committed.
///
/// `Some(now)` when one is due, as of `now` on SQLite's clock, the reading the
/// stamp records. `None` when snapshots are off or the interval has not
/// elapsed.
///
/// **The gate is answered before any expensive work.** The hook spawn is the
/// scheduler and fires on every prompt; a snapshot of the real store is
/// 1.087 GB and 0.56 s, so an ungated step writes a gigabyte per prompt and the
/// first user to notice is one whose disk fills. The interval is what makes
/// snapshots on by default affordable, and this is where it is enforced. Asking
/// it here rather than after the lock drops is what keeps a pass with nothing
/// due to one writable open of the store.
///
/// **One clock.** `now` comes from SQLite, the same source every stored
/// timestamp and every snapshot file name comes from, so the comparison is
/// between two readings of one clock rather than between SQLite's and the
/// process's.
///
/// **Nothing here raises.** A failed read of the clock or the stamp comes back
/// as `Err` with its text, and [`run_with`] writes the note; see
/// [`Snapshotted`].
fn due(store: &crate::store::Store, config: &Config) -> std::result::Result<Option<i64>, String> {
    if !config.snapshots_enabled() {
        return Ok(None);
    }

    let now: i64 = store
        .conn()
        .query_row("SELECT strftime('%s', 'now')", [], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;

    match store.meta_int(META_LAST_SNAPSHOT) {
        // Not yet due. The common case by a wide margin - a machine that fires
        // a hook on every prompt reaches here dozens of times between two
        // snapshots - so it costs one `meta` read and returns silent.
        Ok(Some(last))
            if now.saturating_sub(last)
                < (config.snapshot_interval_hours().saturating_mul(3_600) as i64) =>
        {
            Ok(None)
        }
        // Never taken one, or the interval has elapsed.
        Ok(_) => Ok(Some(now)),
        Err(e) => Err(e.to_string()),
    }
}

/// Take the rolling snapshot [`due`] found due, as of `now`.
///
/// Runs after the lock is released, on a handle of its own, because the pass's
/// is gone with the lock. Holding the lock through a `VACUUM INTO` of a 3 GB
/// store would make every hook in that window lose the race for nothing, and a
/// copy taken while another pass holds the lock is still a valid store.
///
/// **Nothing here raises.** Every failure is a note; see [`Snapshotted`].
fn roll(data_dir: &Path, config: &Config, now: i64) -> Snapshotted {
    let mut out = Snapshotted::default();

    let store = match crate::store::Store::open(data_dir) {
        Ok(store) => store,
        Err(e) => {
            out.notes.push(format!("no snapshot was taken: {e}"));
            return out;
        }
    };

    match crate::store::snapshot::take(data_dir) {
        Ok(path) => out.taken = Some(path),
        Err(e) => {
            out.notes
                .push(format!("the store could not be snapshotted: {e}"));
            return out;
        }
    }

    match crate::store::snapshot::prune(
        &data_dir.join(crate::store::snapshot::DIR_NAME),
        config.snapshots_kept(),
    ) {
        Ok(pruned) => out.pruned = pruned,
        Err(e) => out
            .notes
            .push(format!("older snapshots could not be pruned: {e}")),
    }

    // Last, and only after a copy actually landed: a stamp written ahead of a
    // failed snapshot would buy the failure a whole interval of silence.
    if let Err(e) = store.set_meta_int(META_LAST_SNAPSHOT, now) {
        out.notes.push(format!(
            "the snapshot was taken and its timestamp could not be recorded, so the next \
             ingest will take another: {e}"
        ));
    }
    out
}

/// The tree pass, as the driver walks it.
struct Tree<'a> {
    data_dir: &'a Path,
    config: &'a Config,
    /// What discovery found, filled in the step before the rows are read.
    transcripts: Vec<PathBuf>,
    /// Whether a snapshot is due, asked after the row; see [`due`].
    gate: std::result::Result<Option<i64>, String>,
}

impl Walk for Tree<'_> {
    fn before(&mut self, store: &mut Store, summary: &mut Summary) {
        // The decision log drain, after recovery and BEFORE the walk. The
        // prompt path cannot write SQLite, so this is where its decision files
        // become rows - and the position is what makes the watermark this
        // stamps the archive as it stood when the pass began rather than as
        // this pass leaves it.
        //
        // A drain that fails is a note and not a failed pass: the archive is
        // the work, and a decision log that could not be moved is worth saying
        // and not worth losing a tree walk over.
        summary.feedback =
            match crate::feedback::drain(store.conn_mut(), self.data_dir, self.config) {
                Ok(drained) => drained,
                Err(e) => crate::feedback::Drained {
                    discarded: vec![(
                        self.data_dir.join(crate::inject::decision::DIR_NAME),
                        format!("the decision log could not be drained: {e}"),
                    )],
                    ..crate::feedback::Drained::default()
                },
            };

        let found = discover::discover(self.config);
        summary.unreadable = found.unreadable;
        summary.excluded = found.excluded;
        self.transcripts = found.transcripts;
    }

    fn walk(
        &mut self,
        store: &mut Store,
        rows: &crate::ingest::Rows,
        summary: &mut Summary,
    ) -> Result<()> {
        // One resolver for the whole walk. 1,253 real transcripts carry 63
        // distinct `cwd` values, so the memo is what keeps the git spawns
        // proportional to the projects rather than to the files.
        let mut projects = Resolver::new();
        walk(
            store,
            rows,
            self.data_dir,
            &self.transcripts,
            summary,
            &mut projects,
            self.config.capture_mode(),
        )
    }

    fn after_walk(&mut self, store: &mut Store, summary: &mut Summary) {
        // Continuation links, first after the walk and on every tree pass
        // whatever the walk committed (#11). A session archived before its
        // predecessor has a NULL link its own file will never change, and the
        // predecessor may have come from `terminus backfill` or a named file,
        // neither of which runs this, so the tree pass is where it is noticed.
        // It reads only what the walk and earlier passes archived, so it sits
        // directly after the walk, ahead of the steps that judge sessions.
        //
        // Nothing here fails the pass: a sweep that could not finish is one
        // line in `runs.error`, and the next tree pass tries again.
        summary.continuations = crate::ingest::fill_continuations(store.conn())
            .err()
            .map(|e| format!("continuation links could not be filled: {e}"));

        // Decision outcomes, AFTER the walk and before the `runs` row. The
        // turns this pass just committed are what a session's idleness is
        // measured from and what a decision's outcome is read off, so running
        // this first would judge every decision against the archive as it
        // stood before the transcripts that answer it arrived.
        summary.outcomes = crate::feedback::outcomes(store.conn_mut());

        // Observations, and they have to be here: `feedback::outcomes` is what
        // sets `session_meta.is_final`, so a session that went quiet before
        // this pass becomes final and gets its observation on the same pass
        // rather than on the next one. Before the driver writes the row, so
        // whatever it could not do lands in this pass's `runs` row rather than
        // the following pass's.
        //
        // No model is called here and no network connection is opened.
        summary.observations = crate::observe::observe_new(store.conn(), self.config);

        // Retention, and both halves of the position matter. AFTER
        // `observe_new`, because that step reads one blob per newly finalized
        // session and an eviction that ran first would destroy the bytes it
        // was about to read. BEFORE the driver writes the row, because
        // `runs.error` and `runs.notes` are the only textual channel this
        // product has - there is no log file, by design - and a step placed
        // after that row is written has nowhere to report.
        //
        // Nothing here fails the pass. The archive is the work; a retention
        // step that could not run is a note, and a `?` propagated out of it is
        // exactly what would let one damaged row stop every future ingest of
        // every other transcript in the tree. `retention::retain` is shaped to
        // make that the only possibility: it returns notes and never an error.
        summary.retention = crate::retention::retain(store, self.config);
    }

    fn after_row(&mut self, store: &Store) {
        self.gate = due(store, self.config);
    }
}

/// The walk itself. Every per-file failure is recorded and skipped; only a
/// failure of the pass as a whole comes back as `Err`.
fn walk(
    store: &mut crate::store::Store,
    rows: &crate::ingest::Rows,
    data_dir: &Path,
    transcripts: &[PathBuf],
    summary: &mut Summary,
    projects: &mut Resolver,
    // Resolved once for the whole walk, not per file: it is one config read
    // either way, and a mode that could change between two files of one pass
    // would be a store whose blobs disagree about what a pass captured.
    mode: crate::config::CaptureMode,
) -> Result<()> {
    // Which file took each key this walk, so a second path with the same key is
    // never ingested twice and a second file is never appended onto the first
    // one's session.
    let mut taken = std::collections::HashMap::new();
    for path in transcripts {
        if let Some(after) = crate::ingest::fault::pass_fails_after(data_dir) {
            if summary.files_walked >= after {
                return Err(crate::error::Error::io(
                    data_dir.join(crate::ingest::fault::PASS_FAIL_AFTER_FILE),
                    std::io::Error::other(format!("pass fault: failed after {after} file(s)")),
                ));
            }
        }

        summary.files_walked += 1;
        let (path, session_key) = match crate::ingest::claim(rows, &mut taken, path) {
            Ok(Claim::Ingest { file, session_key }) => (file, session_key),
            Ok(Claim::Same) => continue,
            Ok(Claim::Held(e)) | Err(e) => {
                driver::tally(store, summary, path, Err(e));
                continue;
            }
        };
        let ingested =
            crate::ingest::ingest_locked(store, rows, &path, session_key, projects, mode);
        driver::tally(store, summary, &path, ingested);
    }
    Ok(())
}

/// One reported line for one path.
///
/// `Error::Io` already renders as `{path}: {source}`, so prefixing every reason
/// with its path unconditionally prints the path twice in the only channel a
/// user reads.
pub fn note(path: &Path, reason: &str) -> String {
    let path = path.display().to_string();
    if reason.starts_with(&path) {
        reason.to_owned()
    } else {
        format!("{path}: {reason}")
    }
}
