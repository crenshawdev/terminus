//! The bounded-parallelism pass a backfill runs (D-11, ING-11).
//!
//! One pass over the tree, with the reading, JSON scanning and zstd compression
//! spread across a fixed number of [`std::thread`] workers while **exactly one**
//! thread owns the writable SQLite connection and performs every write - every
//! per-file transaction, every watermark, the `runs` row, all of it.
//!
//! # Why the writer is single and the workers are few
//!
//! SQLite has one writer. The ceiling on a backfill is therefore not the core
//! count, and adding workers past the point where the writer saturates buys
//! nothing but memory. Compression is the stage that actually scales, which is
//! why it is the stage that moved. `crates/verbatim/src/cmd/backfill.rs` fixes
//! the count in one place, small and deliberately below any machine's cores.
//!
//! # No thread pool, and no thread on the hook path (D-11)
//!
//! `std::thread` and `std::sync::mpsc`, no `rayon` and no global pool. The
//! threads are started here and nowhere else, inside [`run_with`], which is
//! reached only by `verbatim backfill`. A hook-triggered `verbatim ingest` runs
//! `pass::run` exactly as it did before and starts no thread at all - the
//! measured 0.408 ms startup floor is the product, and it must not pay for a
//! command it never runs.
//!
//! # The work in flight is bounded, and that is the "chunked" half of ING-11
//!
//! Files are handed out in rounds of [`QUEUE_PER_WORKER`] per worker, and a
//! round is fully drained before the next is dispatched. Peak memory is
//! therefore the round size and never the corpus: real transcripts are p50
//! 290 KB, p90 1.03 MB and p99 3.1 MB, and an unbounded queue over 2,248 of
//! them would hold the whole 1.1 GB tree in RAM plus its compressed copy.
//!
//! Rounds are also what makes this deadlock-free with `std` channels alone. Both
//! channels are bounded at exactly the round size and at most one round is ever
//! outstanding, so no send on either side can block: the writer never waits to
//! dispatch while a worker waits to deliver.
//!
//! # Everything that makes a pass resumable is unchanged (D-22)
//!
//! The ingest lock taken once for the whole run before the store is opened,
//! recovery at the top, one transaction per file so a kill loses at most one
//! file, a per-file failure recorded against its path and skipped rather than
//! propagated (D-12), and one `runs` row for the whole pass. Files are applied
//! in discovery order, exactly as the sequential walk applies them, so the two
//! reach byte-identical stores rather than merely equivalent ones.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Instant;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::ingest::lock::{self, Attempt};
use crate::ingest::pass::{note, PassOutcome, Summary};
use crate::ingest::{Existing, Outcome, Prepared, RunRow};
use crate::project::Resolver;
use crate::store::Store;

/// Files in flight per worker.
///
/// Two: one being worked on and one already read, so a worker that finishes a
/// small file does not wait for the writer to come back round. Higher multiplies
/// peak memory by the p99 transcript size for no throughput a single writer can
/// absorb.
pub const QUEUE_PER_WORKER: usize = 2;

/// What one backfill pass did.
///
/// [`PassOutcome`] and [`Summary`] are `pass`'s, unchanged and unwrapped: a
/// backfill is a pass, and two accounts of what a pass did would be two things
/// to keep in step. The two fields beside it are the only facts that are about
/// the *pipeline* rather than about the pass.
#[derive(Debug, Clone)]
pub struct Report {
    pub outcome: PassOutcome,
    /// Workers asked for.
    pub workers: usize,
    /// Distinct worker threads that actually prepared at least one file. The
    /// evidence that the work was spread, rather than the claim that it was.
    pub threads_used: usize,
}

/// Walk every configured transcript root in parallel and ingest what is beneath
/// it.
pub fn run(data_dir: &Path, workers: usize) -> Result<Report> {
    let config = Config::load()?;
    run_with(data_dir, &config, workers)
}

/// The pass, against a config the caller already has.
///
/// The seam tests use, mirroring [`crate::ingest::pass::run_with`], so the two
/// can be run over one tree and compared.
pub fn run_with(data_dir: &Path, config: &Config, workers: usize) -> Result<Report> {
    let started = Instant::now();
    // Zero workers is a caller's arithmetic, not a request to do nothing.
    let workers = workers.max(1);

    // Once, for the whole run, and before the store is opened (D-15, D-18). A
    // hook that fires during a backfill loses this race and exits 0.
    let _guard = match lock::try_acquire(data_dir)? {
        Attempt::Held => {
            return Ok(Report {
                outcome: PassOutcome::LockHeld,
                workers,
                threads_used: 0,
            })
        }
        Attempt::Acquired(guard) => guard,
    };

    // Recovery before discovery, and before anything is read (ING-03, D-24).
    let (mut store, recovery) = crate::recover::recover(data_dir)?;

    let found = crate::discover::discover(config);
    let mut summary = Summary {
        unreadable: found.unreadable,
        excluded: found.excluded,
        recovery,
        ..Summary::default()
    };

    // One resolver for the whole run, on the writer thread. Project identity may
    // spawn git and is memoized across 63 distinct `cwd` values in a real
    // corpus, so keeping it here costs a handful of resolutions and saves every
    // worker queueing on a lock around it.
    let mut projects = Resolver::new();
    let mut threads: HashSet<ThreadId> = HashSet::new();

    let walked = pipeline(
        &mut store,
        &found.transcripts,
        &mut summary,
        &mut projects,
        workers,
        &mut threads,
        config.capture_mode(),
    );
    summary.duration = started.elapsed();

    // One row, whatever happened (D-10, D-14).
    let recorded = record_pass(store.conn(), &summary, walked.as_ref().err());

    match walked {
        Ok(()) => {
            recorded?;
            Ok(Report {
                outcome: PassOutcome::Ran(summary),
                workers,
                threads_used: threads.len(),
            })
        }
        Err(e) => Err(e),
    }
}

/// A worker panic, turned into that file's own failure.
///
/// The drain loop in [`dispatch_round`] waits for exactly as many `Done`
/// messages as it sent, and its disconnect arm only fires once EVERY worker is
/// gone. So a single worker panicking inside `prepare` used to wedge the pass in
/// the worst way available: its `Done` never arrived, the surviving workers kept
/// `done_rx` connected so the disconnect arm never fired, and the scope could
/// not join to re-raise the panic because its own closure was the thing
/// blocked. The backfill simply stopped, with no error and no exit.
///
/// Catching it here restores the invariant the drain loop is written against -
/// every job sent comes back exactly once - and costs nothing else: D-12
/// already says one damaged transcript is not a reason to archive none of the
/// tree, and a panic is only the loudest way for one to be damaged. The file is
/// recorded and skipped like any other per-file failure.
fn panic_error(path: &Path, payload: Box<dyn std::any::Any + Send>) -> Error {
    let detail = payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panicked".to_string());
    Error::Io {
        path: path.to_path_buf(),
        source: std::io::Error::other(format!("preparing this file panicked: {detail}")),
    }
}

/// One file handed to a worker: everything preparing it needs and nothing that
/// needs a connection.
struct Job {
    path: PathBuf,
    session_key: String,
    existing: Existing,
}

/// One file coming back: which thread did it, and what it produced.
struct Done {
    /// Position within the round, so the writer applies files in discovery
    /// order however the workers happened to finish.
    slot: usize,
    thread: ThreadId,
    prepared: Result<Prepared>,
}

/// The round-by-round pipeline.
///
/// Only a failure of the pass as a whole comes back as `Err`; every per-file
/// failure is recorded against its path and skipped (D-12), because the tree is
/// the unit of work and one damaged transcript is not a reason to archive none
/// of it.
fn pipeline(
    store: &mut Store,
    transcripts: &[PathBuf],
    summary: &mut Summary,
    projects: &mut Resolver,
    workers: usize,
    threads: &mut HashSet<ThreadId>,
    // Resolved once for the whole run and copied into every worker (ING-07).
    // `CaptureMode` is `Copy`, so the workers share a value rather than a lock.
    mode: crate::config::CaptureMode,
) -> Result<()> {
    if transcripts.is_empty() {
        return Ok(());
    }
    let round = workers * QUEUE_PER_WORKER;

    // Scoped threads: the workers borrow nothing that outlives this call, and
    // the scope joins every one of them before it returns - including on the
    // panic and early-return paths, which is what makes "the writer is the only
    // thread left touching the store" true rather than hoped for.
    std::thread::scope(|scope| {
        // Both bounded at the round size, with at most one round outstanding,
        // so neither side can ever block on a send. See the module comment.
        let (job_tx, job_rx) = mpsc::sync_channel::<(usize, Job)>(round);
        let (done_tx, done_rx) = mpsc::sync_channel::<Done>(round);
        let job_rx = Arc::new(Mutex::new(job_rx));

        for _ in 0..workers {
            let jobs = Arc::clone(&job_rx);
            let done = done_tx.clone();
            scope.spawn(move || {
                loop {
                    // The lock is held for the receive and released before the
                    // work, so the workers share one queue and take the next
                    // file whenever they are free - which is what keeps a
                    // 3 MB transcript from stalling three idle threads.
                    let next = {
                        let queue = jobs.lock().unwrap_or_else(|e| e.into_inner());
                        queue.recv()
                    };
                    let Ok((slot, job)) = next else {
                        // The writer dropped its end: the walk is over.
                        return;
                    };
                    // Every job that was sent comes back exactly once,
                    // whatever happened to it - see `panic_error`.
                    let path = job.path.clone();
                    let prepared =
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            crate::ingest::fault::panic_preparing(&job.path);
                            crate::ingest::prepare(&job.path, job.session_key, &job.existing, mode)
                        })) {
                            Ok(prepared) => prepared,
                            Err(payload) => Err(panic_error(&path, payload)),
                        };
                    if done
                        .send(Done {
                            slot,
                            thread: std::thread::current().id(),
                            prepared,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
        // The writer holds no sender of its own, so `done_rx` closes when the
        // last worker exits rather than never.
        drop(done_tx);

        let mut outcome = Ok(());
        for batch in transcripts.chunks(round) {
            if let Err(e) =
                dispatch_round(store, batch, summary, projects, threads, &job_tx, &done_rx)
            {
                outcome = Err(e);
                break;
            }
        }

        // Whatever happened, release the workers and let the scope join them.
        drop(job_tx);
        outcome
    })
}

/// Read one round's `Existing` rows, hand them out, drain the results and apply
/// them in discovery order.
#[allow(clippy::too_many_arguments)]
fn dispatch_round(
    store: &mut Store,
    batch: &[PathBuf],
    summary: &mut Summary,
    projects: &mut Resolver,
    threads: &mut HashSet<ThreadId>,
    job_tx: &mpsc::SyncSender<(usize, Job)>,
    done_rx: &mpsc::Receiver<Done>,
) -> Result<()> {
    // Two files never share a session key - the key IS the canonical path (D-01)
    // - so reading a whole round's rows before applying any of it cannot read
    // something a later apply in the same round would have changed.
    let mut sent = 0usize;
    let mut results: Vec<Option<Result<Prepared>>> = (0..batch.len()).map(|_| None).collect();

    for (slot, path) in batch.iter().enumerate() {
        summary.files_walked += 1;
        let job = match read_existing(store, path) {
            Ok(job) => job,
            // A path that is not UTF-8, or a store row this build refuses.
            // Recorded and skipped like any other per-file failure.
            Err(e) => {
                results[slot] = Some(Err(e));
                continue;
            }
        };
        // Cannot block: the channel is bounded at the round size and this loop
        // sends at most one round.
        if job_tx.send((slot, job)).is_err() {
            // Every worker is gone. Nothing more can be prepared, so the rest
            // of this round is left unsent and the loop below drains what did.
            break;
        }
        sent += 1;
    }

    for _ in 0..sent {
        let Ok(done) = done_rx.recv() else {
            // Every worker exited without answering. That can only be a panic
            // inside `prepare`, which the scope will re-raise when it joins.
            break;
        };
        threads.insert(done.thread);
        results[done.slot] = Some(done.prepared);
    }

    for (slot, result) in results.into_iter().enumerate() {
        let Some(result) = result else { continue };
        commit_one(store, &batch[slot], result, summary, projects)?;
    }
    Ok(())
}

/// The writer's half for one file: apply it, or record why it was skipped.
fn commit_one(
    store: &mut Store,
    path: &Path,
    prepared: Result<Prepared>,
    summary: &mut Summary,
    projects: &mut Resolver,
) -> Result<()> {
    let applied = prepared.and_then(|prepared| {
        // A fresh `Instant` per file: the elapsed time the pass reports is the
        // run's, measured by the caller, not this file's.
        crate::ingest::apply(store, prepared, Instant::now(), RunRow::PassOwns, projects)
    });

    match applied {
        Ok(Outcome::Committed(pass)) => {
            summary.files_committed += 1;
            summary.bytes_read += pass.bytes_read;
            summary.turns_added += pass.turns_added;
            // The same file-aimed fault point the sequential walk has, so a
            // kill can be aimed between two files of a backfill too. Inert
            // without `testkit`.
            crate::ingest::fault::stall_after_files(summary.files_committed);
        }
        Ok(Outcome::UpToDate | Outcome::LockHeld | Outcome::Excluded(_)) => {}
        Err(e) => {
            let mut reason = e.to_string();
            // D-13, matched on the variant and never on the message, exactly as
            // the sequential walk matches it: a transcript at a non-UTF-8 path
            // fails with the same io kind and must flag nothing.
            if matches!(
                e,
                crate::error::Error::TranscriptDiverged { .. }
                    | crate::error::Error::TranscriptRewritten { .. }
            ) {
                if let Err(flag) = crate::ingest::flag_divergence(store.conn(), path) {
                    reason.push_str(&format!(" (the flag could not be set: {flag})"));
                }
            }
            summary.failures.push((path.to_path_buf(), reason));
        }
    }
    Ok(())
}

/// What the store already holds for one transcript, read on the writer thread.
fn read_existing(store: &Store, path: &Path) -> Result<Job> {
    let session_key = crate::ingest::path_key(path)?;
    let existing = Existing::read(store.conn(), &session_key)?;
    Ok(Job {
        path: path.to_path_buf(),
        session_key,
        existing,
    })
}

/// The one `runs` row this pass leaves behind (D-10).
///
/// Field for field the row `pass::record_pass` writes, because a backfill is a
/// pass and `verbatim status` reads one table. It is a second copy of that
/// function rather than a call to it only because `pass::record_pass` is
/// private and `pass.rs` is outside this plan's file lease; the two belong
/// together in one place, and that move is an open item on this plan rather
/// than a difference in behaviour.
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
