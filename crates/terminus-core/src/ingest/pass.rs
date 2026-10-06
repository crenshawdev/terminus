//! One ingest pass over a whole transcript tree.
//!
//! The lock is taken once and the store is opened once, for the whole walk. The
//! **transactions** are still per-file, and that is D-11 rather than an
//! optimization: a write transaction held across the walk stops MCP readers,
//! which is the reason redb was rejected (`DESIGN-BRIEF.md:83`), and per-file
//! commits are the granularity phase 1's crash harness already proved atomic.
//! They also bound what a kill can lose to one file, and real files are p50
//! 290 KB, p90 1.03 MB, p99 3.1 MB.
//!
//! A per-file failure is recorded against that file's path and skipped, never
//! propagated (D-12). Both of phase 1's hard-error paths reach this loop -
//! `Existing::read` refusing a session archived with no `session_meta` row, and
//! `read_tail` refusing a file shorter than its watermark - and either one
//! stopping the pass would let a single damaged session wedge every future
//! ingest of every other transcript in the tree. That is the exact failure gate
//! fix `3e5d9ff` closed for `reindex`, and it is worth more here: the tree is
//! two thousand files.
//!
//! One of those two is also the only per-file failure that writes anything: a
//! transcript shorter than its watermark leaves D-13's flag on that session, so
//! the divergence reaches `terminus verify` rather than living only in a `runs`
//! row nobody diffs. The archive itself is still untouched.
//!
//! `ingest::run`'s single-file behaviour and its refusals are unchanged. The
//! skip lives here, in the pass, and not in `ingest_locked` - a caller that
//! named one file deserves the error it asked about.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::discover;
use crate::error::Result;
use crate::ingest::lock::{self, Attempt};
use crate::ingest::{Outcome, RunRow};
use crate::project::Resolver;

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
    /// Per-file failures: the path and why it was skipped (D-12).
    pub failures: Vec<(PathBuf, String)>,
    /// Directories that could not be listed.
    pub unreadable: Vec<(PathBuf, String)>,
    /// Project directories the config excluded, never listed and never opened.
    pub excluded: Vec<PathBuf>,
    /// What recovery repaired before the walk began (ING-03, D-24).
    pub recovery: crate::recover::Recovered,
    /// What the decision log drained into the store before the walk (FEED-01).
    pub feedback: crate::feedback::Drained,
    /// What the walk's own turns then made of those decisions (FEED-02).
    pub outcomes: crate::feedback::Labeled,
    /// What the sessions this pass closed had to say for themselves (OBS-01).
    pub observations: crate::observe::Observed,
    /// What retention evicted, deleted and left for the next pass (RET-01,
    /// RET-03, D-10). Empty and free while retention is off, which is the
    /// default and the state every user starts in.
    pub retention: crate::retention::Applied,
    /// What the provider was asked, after the lock dropped (OBS-02, D-07).
    ///
    /// Empty and free while judgment is off, which is the default. Its notes do
    /// NOT reach `runs.error`: this step runs after that row is written, so the
    /// caller prints them on stderr instead.
    pub judgment: crate::observe::Judged,
    /// What the rolling snapshot step did, after the lock dropped (STOR-06,
    /// D-15).
    ///
    /// Its notes do NOT reach `runs.error` either, and for the same reason
    /// [`Summary::judgment`] states: the row has already committed by the time
    /// this runs.
    pub snapshot: Snapshotted,
    pub duration: Duration,
}

/// `meta` key holding when the last rolling snapshot was taken, as unix seconds
/// (D-15).
///
/// A row in `meta` rather than the newest file's name, because the gate has to
/// be answerable without listing a directory - and because a user who copies
/// their snapshots elsewhere has not thereby asked for a new one on the next
/// prompt.
pub const META_LAST_SNAPSHOT: &str = "last_snapshot_at";

/// What one pass's snapshot step did (STOR-06, D-15).
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
    /// the hook spawn is the scheduler and a second invocation is expected
    /// (ING-02).
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
/// Two halves, and the boundary between them is D-07. [`locked`] is everything
/// that touches the archive, under the ingest lock, ending with the `runs` row.
/// The judgment call comes after it returns - which is after the guard and the
/// store have been dropped - because a slow or hanging provider held inside the
/// lock would make every hook-spawned pass in that window exit [`LockHeld`] and
/// archive nothing.
///
/// [`LockHeld`]: PassOutcome::LockHeld
pub fn run_with(data_dir: &Path, config: &Config) -> Result<PassOutcome> {
    let mut summary = match locked(data_dir, config)? {
        PassOutcome::LockHeld => return Ok(PassOutcome::LockHeld),
        PassOutcome::Ran(summary) => summary,
    };

    // Outside the lock, outside the store handle, and after the row that says
    // the pass happened. Nothing below can fail the pass: the archive is
    // already committed and this step only ever returns notes (OBS-04).
    summary.judgment = crate::observe::judge_new(data_dir, config);

    // STOR-06 and D-15, in the same position and for the same reason: the guard
    // and the store handle are gone, the `runs` row has committed, and nothing
    // below can fail the pass. A snapshot is a copy of what is already safely
    // archived, so a snapshot that could not be written is a note - failing the
    // pass over it would stop archiving the transcripts the copy exists to
    // protect.
    summary.snapshot = roll(data_dir, config);
    Ok(PassOutcome::Ran(summary))
}

/// Take a rolling snapshot, if one is due (STOR-06, D-15).
///
/// **The gate is answered before any expensive work.** The hook spawn is the
/// scheduler and fires on every prompt; a snapshot of the real store is 1.087 GB
/// and 0.56 s, so an ungated step writes a gigabyte per prompt and the first
/// user to notice is one whose disk fills. The interval is what makes STOR-06's
/// "on by default" affordable, and this is where it is enforced.
///
/// **One clock.** `now` comes from SQLite, the same source every stored
/// timestamp and every snapshot file name comes from, so the comparison is
/// between two readings of one clock rather than between SQLite's and the
/// process's.
///
/// **Nothing here raises.** Every failure is a note; see [`Snapshotted`].
fn roll(data_dir: &Path, config: &Config) -> Snapshotted {
    let mut out = Snapshotted::default();
    if !config.snapshots_enabled() {
        return out;
    }

    // Its own handle, because the pass's is gone - the same shape
    // `observe::judge_new` has, and for the same D-07 reason.
    let store = match crate::store::Store::open(data_dir) {
        Ok(store) => store,
        Err(e) => {
            out.notes.push(format!("no snapshot was taken: {e}"));
            return out;
        }
    };

    let now: i64 = match store
        .conn()
        .query_row("SELECT strftime('%s', 'now')", [], |r| {
            r.get::<_, String>(0)
        }) {
        Ok(text) => match text.parse() {
            Ok(seconds) => seconds,
            Err(e) => {
                out.notes.push(format!("no snapshot was taken: {e}"));
                return out;
            }
        },
        Err(e) => {
            out.notes.push(format!("no snapshot was taken: {e}"));
            return out;
        }
    };

    match store.meta_int(META_LAST_SNAPSHOT) {
        // Not yet due. The common case by a wide margin - a machine that fires
        // a hook on every prompt reaches here dozens of times between two
        // snapshots - so it costs one `meta` read and returns silent.
        Ok(Some(last))
            if now.saturating_sub(last)
                < (config.snapshot_interval_hours().saturating_mul(3_600) as i64) =>
        {
            return out
        }
        // Never taken one, or the interval has elapsed.
        Ok(_) => {}
        Err(e) => {
            out.notes.push(format!("no snapshot was taken: {e}"));
            return out;
        }
    }

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

/// Everything the ingest lock covers, from taking it to the `runs` row.
fn locked(data_dir: &Path, config: &Config) -> Result<PassOutcome> {
    let started = Instant::now();

    // Once, for the whole walk. A per-file lock would be two thousand syscalls
    // and would let another process interleave between files, which is exactly
    // the invariant `ingest_locked` rests on: the guard makes this the only
    // writer, so nothing changes the store between its read and its write.
    let _guard = match lock::try_acquire(data_dir)? {
        Attempt::Held => return Ok(PassOutcome::LockHeld),
        Attempt::Acquired(guard) => guard,
    };

    // Recovery before discovery, and before anything is read (ING-03, D-24).
    // It also brings a store older than this build forward (STOR-05), so a walk
    // never appends turn rows in one shape beside rows written in another.
    let (mut store, recovery) = crate::recover::recover(data_dir)?;

    // FEED-01, after recovery and BEFORE the walk. The prompt path cannot write
    // SQLite (D-01), so this is where its decision files become rows - and the
    // position is what makes the watermark this stamps the archive as it stood
    // when the pass began rather than as this pass leaves it (D-10).
    //
    // A drain that fails is a note and not a failed pass: the archive is the
    // work, and a decision log that could not be moved is worth saying and not
    // worth losing a tree walk over.
    let feedback = match crate::feedback::drain(store.conn_mut(), data_dir) {
        Ok(drained) => drained,
        Err(e) => crate::feedback::Drained {
            decisions: 0,
            discarded: vec![(
                data_dir.join(crate::inject::decision::DIR_NAME),
                format!("the decision log could not be drained: {e}"),
            )],
        },
    };

    let found = discover::discover(config);
    let mut summary = Summary {
        unreadable: found.unreadable,
        excluded: found.excluded,
        recovery,
        feedback,
        ..Summary::default()
    };

    // One resolver for the whole walk (D-05, ING-05). 1,253 real transcripts
    // carry 63 distinct `cwd` values, so the memo is what keeps the git spawns
    // proportional to the projects rather than to the files.
    let mut projects = Resolver::new();
    let walked = walk(
        &mut store,
        data_dir,
        &found.transcripts,
        &mut summary,
        &mut projects,
        config.capture_mode(),
    );

    // FEED-02, AFTER the walk and before the `runs` row. The turns this pass
    // just committed are what a session's idleness is measured from and what a
    // decision's outcome is read off, so running this first would judge every
    // decision against the archive as it stood before the transcripts that
    // answer it arrived.
    summary.outcomes = crate::feedback::outcomes(store.conn_mut());

    // OBS-01, and it has to be here: `feedback::outcomes` is what sets
    // `session_meta.is_final`, so a session that went quiet before this pass
    // becomes final and gets its observation on the same pass rather than on
    // the next one. Before `record_pass`, so whatever it could not do lands in
    // this pass's `runs.error` rather than the following pass's.
    //
    // No model is called here and no network connection is opened. The
    // judgment half is its own command, outside this lock (D-07): a slow
    // provider held here would make every hook-spawned pass in that window exit
    // as `LockHeld` and archive nothing.
    summary.observations = crate::observe::observe_new(store.conn(), config);

    // RET-03 and D-10, and both halves of the position matter. AFTER
    // `observe_new`, because that step reads one blob per newly finalized
    // session and an eviction that ran first would destroy the bytes it was
    // about to read. BEFORE `record_pass`, because `runs.error` is the only
    // textual channel this product has - there is no log file, by design - and
    // a step placed after that row is written has nowhere to report.
    //
    // Nothing here fails the pass. The archive is the work; a retention step
    // that could not run is a note, and a `?` propagated out of it is exactly
    // what would let one damaged row stop every future ingest of every other
    // transcript in the tree. `retention::retain` is shaped to make that the
    // only possibility: it returns notes and never an error.
    summary.retention = crate::retention::retain(&mut store, config);

    summary.duration = started.elapsed();

    // One row, whatever happened (D-10, D-14). A pass that died writes it in a
    // transaction of its own, after the per-file transaction it was inside has
    // rolled back - with no log file by design, this row is the only place the
    // failure can be seen. A pass that walked a tree and found nothing new
    // writes one too, because "the last ingest run" has to move.
    let recorded = record_pass(store.conn(), &summary, walked.as_ref().err());

    match walked {
        Ok(()) => {
            recorded?;
            Ok(PassOutcome::Ran(summary))
        }
        // The walk's error is the one worth returning. If the row could not be
        // written either, the store is unwritable and that is the same fact
        // said twice.
        Err(e) => Err(e),
    }
}

/// The walk itself. Every per-file failure is recorded and skipped; only a
/// failure of the pass as a whole comes back as `Err`.
fn walk(
    store: &mut crate::store::Store,
    data_dir: &Path,
    transcripts: &[PathBuf],
    summary: &mut Summary,
    projects: &mut Resolver,
    // Resolved once for the whole walk, not per file (ING-07): it is one config
    // read either way, and a mode that could change between two files of one
    // pass would be a store whose blobs disagree about what a pass captured.
    mode: crate::config::CaptureMode,
) -> Result<()> {
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
        // A fresh `Instant` per file: the elapsed time the pass reports is the
        // walk's, measured by the caller, not this file's.
        match crate::ingest::ingest_locked(
            store,
            path,
            Instant::now(),
            RunRow::PassOwns,
            projects,
            mode,
        ) {
            Ok(Outcome::Committed(pass)) => {
                summary.files_committed += 1;
                summary.bytes_read += pass.bytes_read;
                summary.turns_added += pass.turns_added;
                // The pass-level fault point: a kill aimed here lands between
                // two files, with the walk half done and the store consistent.
                // Timed kills reach this region only by luck, and on a tree of
                // small transcripts nearly never.
                crate::ingest::fault::stall_after_files(summary.files_committed);
            }
            // Nothing new past the watermark, or - both unreachable here - a
            // lock this process already owns, and an exclusion only the
            // single-file entry point tests, on files this walk never yielded.
            Ok(Outcome::UpToDate | Outcome::LockHeld | Outcome::Excluded(_)) => {}
            Err(e) => {
                // Recorded and skipped. The tree is the unit of work; one
                // damaged transcript is not a reason to archive none of it.
                let mut reason = e.to_string();

                // D-13, and the only per-file failure that leaves a mark on the
                // store. Matched on the variant and never on the message: a
                // transcript at a non-UTF-8 path fails with the same io kind
                // and must flag nothing. The flag goes in a transaction of its
                // own - the bare statement autocommits - because the file's own
                // transaction has already rolled back, and it is what lets
                // `terminus verify` name the divergence instead of it living
                // only in a `runs` row nobody diffs.
                if matches!(
                    e,
                    crate::error::Error::TranscriptDiverged { .. }
                        | crate::error::Error::TranscriptRewritten { .. }
                ) {
                    if let Err(flag) = crate::ingest::flag_divergence(store.conn(), path) {
                        // Same note rather than a second failure: `files_failed`
                        // counts files, and one file must not become two.
                        reason.push_str(&format!(" (the flag could not be set: {flag})"));
                    }
                }

                summary.failures.push((path.clone(), reason));
            }
        }
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

/// The one `runs` row a pass leaves behind (D-10).
///
/// `runs.error` is the only textual channel this product has - there is no log
/// file, by design, and `terminus status` surfaces this table - so everything
/// worth a human's attention goes in it: the path and reason of every file that
/// was skipped, every directory that could not be listed, every watermark
/// recovery had to repair, and the failure that ended the pass if one did. It
/// stays null when there is genuinely nothing to say.
fn record_pass(
    conn: &rusqlite::Connection,
    summary: &Summary,
    fatal: Option<&crate::error::Error>,
) -> Result<()> {
    let mut notes: Vec<String> = Vec::new();
    for (path, reason) in summary.failures.iter().chain(&summary.unreadable) {
        notes.push(note(path, reason));
    }
    notes.extend(summary.recovery.lines());
    notes.extend(summary.feedback.lines());
    notes.extend(summary.outcomes.lines());
    notes.extend(summary.observations.lines());
    notes.extend(summary.retention.lines());
    if let Some(e) = fatal {
        notes.push(format!("pass failed: {e}"));
    }
    let error = if notes.is_empty() {
        None
    } else {
        Some(notes.join("\n"))
    };

    let elapsed = summary.duration;
    conn.execute(
        "INSERT INTO runs (
            started_at, finished_at, duration_ms, files_seen, bytes_read,
            turns_added, error, files_committed, files_failed
         ) VALUES (
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            ?2, ?3, ?4, ?5, ?6, ?7, ?8
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
        ],
    )?;
    Ok(())
}
