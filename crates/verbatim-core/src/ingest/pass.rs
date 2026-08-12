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
//! `ingest::run`'s single-file behaviour and its refusals are unchanged. The
//! skip lives here, in the pass, and not in `ingest_locked` - a caller that
//! named one file deserves the error it asked about.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::discover;
use crate::error::Result;
use crate::ingest::lock::{self, Attempt};
use crate::ingest::Outcome;

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
    pub duration: Duration,
}

impl Summary {
    /// Files walked that did not commit and did not fail: nothing new to read.
    pub fn files_unchanged(&self) -> usize {
        self.files_walked - self.files_committed - self.failures.len()
    }
}

/// What an invocation of [`run_with`] did.
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
pub fn run_with(data_dir: &Path, config: &Config) -> Result<PassOutcome> {
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

    let found = discover::discover(config);
    let mut summary = Summary {
        unreadable: found.unreadable,
        excluded: found.excluded,
        recovery,
        ..Summary::default()
    };

    for path in &found.transcripts {
        summary.files_walked += 1;
        // A fresh `Instant` per file: `record_run` measures the file's own pass
        // today, and task 6 moves that row to the pass. Neither reading wants
        // the whole walk's elapsed time attributed to the last file.
        match crate::ingest::ingest_locked(&mut store, path, Instant::now()) {
            Ok(Outcome::Committed(pass)) => {
                summary.files_committed += 1;
                summary.bytes_read += pass.bytes_read;
                summary.turns_added += pass.turns_added;
            }
            // Nothing new past the watermark, or - unreachable here, since this
            // pass holds the lock - a lock this process already owns.
            Ok(Outcome::UpToDate) | Ok(Outcome::LockHeld) => {}
            Err(e) => {
                // Recorded and skipped. The tree is the unit of work; one
                // damaged transcript is not a reason to archive none of it.
                summary.failures.push((path.clone(), e.to_string()));
            }
        }
    }

    summary.duration = started.elapsed();
    Ok(PassOutcome::Ran(summary))
}
