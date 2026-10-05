//! `terminus backfill`: archive the history already on disk, without the user
//! waiting for it (ING-11, AC8).
//!
//! # The estimate comes first, and it is read-only
//!
//! A user who has just installed terminus is about to hand it a gigabyte of
//! transcripts, and the one thing they need before that starts is how much
//! there is and roughly how long it takes. So the first output of this command
//! is a count, a byte total and a time - computed before any ingest work
//! exists, from a tree walk, one `metadata` call per file and one query for the
//! watermarks.
//!
//! Never a query per file. AC8 puts the whole estimate inside 100 ms over 2,248
//! transcripts, and 2,248 round trips to SQLite would not fit; the watermarks
//! come back as one table read and are matched in memory.
//!
//! The store is opened through [`crate::cmd::read`] and never `Store::open`
//! (D-12, phase 3 D-10). A machine that has never ingested must be able to ask
//! "how long will this take" and still have no store afterwards - the answer
//! there is the whole tree, which is exactly what a missing watermark table
//! yields.
//!
//! # The bytes are the remainder, not the corpus
//!
//! Each file contributes its length minus its stored watermark. A half-ingested
//! corpus therefore estimates what is left, which is what makes the number mean
//! something on the second run and after a kill.
//!
//! # Then the shell comes back
//!
//! The work runs in a process detached through the same double fork the hooks
//! use ([`crate::cmd::spawn`], D-03), so `terminus backfill` returns in
//! milliseconds with a gigabyte still to archive. There is no log file by
//! design: the `runs` row the pass writes is the record, and `terminus status`
//! is where it is read.
//!
//! Resumability needs nothing here (D-22). It is the per-file watermarks and
//! the per-file transactions phase 2 already ships, which is why a killed
//! backfill converges on the same store as an uninterrupted one without this
//! module holding any state of its own.

use std::collections::HashMap;
use std::path::Path;

use terminus_core::{discover, Config};

use super::read::{self, Opened};
use super::Failure;

/// How many threads the backfill parses and compresses on (D-11).
///
/// Fixed here, in one place: the pipeline is started with it, the estimate names
/// it, and [`MEASURED_MS`] was measured at it, so the number a user is shown and
/// the number of workers that turn up cannot drift apart. Changing it invalidates
/// the measured rate below, which is the reason both live in this file.
///
/// Deliberately small, and deliberately not derived from the machine's core
/// count: the ceiling on a backfill is SQLite's single writer, compression is
/// the only stage that scales, and a backfill is something a user runs while
/// they are trying to do other work.
pub const WORKERS: usize = 4;

/// The pipeline's own measured rate: 1,413,907,546 bytes of real corpus
/// archived in 49,089 ms across [`WORKERS`] workers on 2026-08-20.
///
/// The pair is kept whole rather than pre-divided so the provenance survives; a
/// later measurement replaces two numbers that can be read straight off a `runs`
/// row.
///
/// **It is not D-23's single-threaded rate divided by the worker count, and
/// that difference is measured rather than assumed.** The same corpus on the
/// same machine took 53,007 ms through the sequential `pass::run`, so four
/// workers bought 8%, not 4x. D-11 predicts exactly this and says why: SQLite
/// has one writer, and on real transcripts the writer's own work - the
/// transaction, the derived rows, the FTS index - dominates the parse and the
/// compression that were moved off it. Dividing by the worker count would print
/// "about 10s" over a corpus that takes fifty, which is the one thing an
/// estimate must not do. D-23 asks for a measured rate; this is the measured
/// rate of the thing that actually runs.
const MEASURED_BYTES: u64 = 1_413_907_546;
const MEASURED_MS: u64 = 49_089;

/// What a backfill would have to read, before it reads any of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Estimate {
    /// Transcripts discovery yields under the configured roots.
    pub transcripts: usize,
    /// Bytes past the stored watermarks: the work that is actually left.
    pub unread_bytes: u64,
    /// Directories the walk could not list, with why.
    ///
    /// Part of the estimate rather than a separate failure, because that is
    /// what they are: a tree that cannot be listed is not zero transcripts, it
    /// is an unknown number of them, and a count printed without saying so
    /// would read as "there is nothing to archive".
    pub unreadable: Vec<(std::path::PathBuf, String)>,
}

impl Estimate {
    /// How long that is likely to take, at [`MEASURED_BYTES`] over
    /// [`MEASURED_MS`].
    pub fn duration(&self) -> std::time::Duration {
        let ms =
            u128::from(self.unread_bytes) * u128::from(MEASURED_MS) / u128::from(MEASURED_BYTES);
        std::time::Duration::from_millis(ms.min(u128::from(u64::MAX)) as u64)
    }
}

/// Count the work without doing any of it.
///
/// Creates nothing: the walk lists directories, the sizes come from directory
/// metadata, and the store is opened read-only or not at all.
pub fn estimate(data_dir: &Path, config: &Config) -> Result<Estimate, Failure> {
    let found = discover::discover(config);
    let archived = watermarks(data_dir, config.clone())?;

    let mut unread_bytes = 0u64;
    for path in &found.transcripts {
        // Metadata, never an open. A file that vanished between the walk and
        // this call contributes nothing rather than failing an estimate - it is
        // a number, and the pass itself will report what it cannot read.
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let done = path
            .to_str()
            .and_then(|key| archived.get(key).copied())
            .unwrap_or(0);
        unread_bytes += len.saturating_sub(done);
    }

    Ok(Estimate {
        transcripts: found.transcripts.len(),
        unread_bytes,
        unreadable: found.unreadable,
    })
}

/// Every stored watermark, as one query.
///
/// The session key is the canonical transcript path as text, which is exactly
/// what discovery's walk builds (D-01, D-17), so the join is a string lookup
/// and needs no path resolution here.
fn watermarks(data_dir: &Path, config: Config) -> Result<HashMap<String, u64>, Failure> {
    let opened = read::open_in(data_dir, config)?;
    let Opened::Ready(reader) = opened else {
        // No store yet: nothing is archived, so every byte is unread.
        return Ok(HashMap::new());
    };

    let conn = reader.store().conn();
    let mut statement = conn
        .prepare("SELECT transcript_path, byte_offset FROM watermarks")
        .map_err(|e| Failure::Operational(e.to_string()))?;
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| Failure::Operational(e.to_string()))?;

    let mut out = HashMap::new();
    for row in rows {
        let (path, offset) = row.map_err(|e| Failure::Operational(e.to_string()))?;
        out.insert(path, offset.max(0) as u64);
    }
    Ok(out)
}

/// The estimate as the lines a user reads.
///
/// All of it on stdout, problems included, and that is deliberate: `backfill`
/// is a human-only command like `install` and `uninstall` (D-24) with no
/// `--json` document to keep clean, and `install` prints this block inside its
/// own summary. A directory that could not be listed belongs beside the count
/// it silently reduced, not on a different stream.
pub fn print_estimate(estimate: &Estimate) {
    println!(
        "{} transcript(s), {} not yet archived",
        estimate.transcripts,
        bytes(estimate.unread_bytes)
    );
    println!(
        "about {} across {WORKERS} workers - an estimate from a measured rate \
         ({} in {}), not a promise",
        duration(estimate.duration()),
        bytes(MEASURED_BYTES),
        duration(std::time::Duration::from_millis(MEASURED_MS)),
    );
    for (path, reason) in &estimate.unreadable {
        println!(
            "  problem: {} could not be listed ({reason}), so nothing under it is counted \
             above and nothing under it will be archived",
            path.display()
        );
    }
}

/// A byte count a human reads, in powers of ten because that is how disk sizes
/// are quoted and this number is compared against `du`.
fn bytes(count: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1_000_000_000, "GB"),
        (1_000_000, "MB"),
        (1_000, "KB"),
        (1, "B"),
    ];
    for (scale, unit) in UNITS {
        if count >= scale {
            if scale == 1 {
                return format!("{count} {unit}");
            }
            // One decimal place: the estimate is not precise enough for two,
            // and "1.1 GB" is the number D-23 is quoted in.
            let tenths = count * 10 / scale;
            return format!("{}.{} {unit}", tenths / 10, tenths % 10);
        }
    }
    format!("{count} B")
}

/// A duration a human reads, at the resolution the estimate deserves.
fn duration(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    if secs == 0 {
        return "under a second".to_owned();
    }
    if secs < 60 {
        return format!("{secs}s");
    }
    if secs < 3_600 {
        return format!("{}m {}s", secs / 60, secs % 60);
    }
    format!("{}h {}m", secs / 3_600, (secs % 3_600) / 60)
}

/// The internal argument that means "you are the working process".
///
/// Without it a backfill prints an estimate and spawns a detached copy of
/// itself; with it, it prints nothing and does the pass. That asymmetry is the
/// whole reason the flag exists: a child invoked with the same command line as
/// its parent would print an estimate and spawn again, forever.
///
/// Deliberately absent from `USAGE`, like [`crate::cmd::spawn::HANDOFF`] and
/// for the same reason - it is a mechanism, not an interface. Typing it runs
/// the pass in the foreground, which is a thing a caller could already do with
/// `terminus ingest`, so nothing is granted by knowing it. It is the argument
/// the crash harness uses, because the working process is the one worth
/// killing and the detached one cannot be reached from the shell that started
/// it.
pub const WORK_FLAG: &str = "work";

/// The parsed command line for `backfill`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Args {
    /// True in the detached child: do the work, print no estimate, spawn
    /// nothing.
    pub work: bool,
}

pub fn run(args: Args) -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    if args.work {
        return work(&data_dir);
    }

    let config = Config::load()?;
    let estimate = estimate(&data_dir, &config)?;
    print_estimate(&estimate);
    // The zeroes printed above already say why nothing started, so an empty
    // tree needs no line of its own here.
    start(&data_dir, &estimate).map(drop)
}

/// Hand the work to a process that outlives this one, and return (AC8,
/// ING-11).
///
/// Through the same double fork the hooks use (D-03), so a backfill started
/// from a shell that is then closed, or from a hook Claude Code later kills,
/// keeps going. Nothing is waited for: the point of the command is that the
/// shell comes back.
///
/// `false` means there was nothing to hand over. A tree with no transcripts in
/// it is archived by doing nothing, and a pass over it would create a store and
/// a WAL as the side effect of an install on a machine that has never run
/// Claude Code - the exact state AC6 asks `doctor` to leave alone and
/// [`crate::cmd::read`] refuses to bring into being. The estimate is already
/// the count of what is there, so this costs no second walk.
pub fn start(data_dir: &Path, estimate: &Estimate) -> Result<bool, Failure> {
    start_from(None, data_dir, estimate)
}

/// [`start`], with the executable to spawn named explicitly.
///
/// `exe` is `None` for every caller but `install`, which passes the stable path
/// it has just renamed a binary over - see [`super::spawn::detached_from`] for
/// why its own `current_exe()` no longer resolves by then.
pub fn start_from(
    exe: Option<&Path>,
    data_dir: &Path,
    estimate: &Estimate,
) -> Result<bool, Failure> {
    if estimate.transcripts == 0 {
        return Ok(false);
    }
    let args = ["backfill".to_string(), format!("--{WORK_FLAG}")];
    let started = match exe {
        Some(exe) => super::spawn::detached_from(exe, &args),
        None => super::spawn::detached(&args),
    };
    started.map(|()| true).map_err(|e| {
        Failure::Operational(format!(
            "the backfill could not be started: {e}. \
             run `terminus ingest` to archive the tree in the foreground, or let the \
             next hook do it - the work is the same either way, and the store at {} \
             is untouched",
            data_dir.display()
        ))
    })
}

/// The detached child: one tree pass across [`WORKERS`] threads, and nothing on
/// stdout.
///
/// The pass takes the ingest lock before it opens the store, so a hook firing
/// during a backfill loses the race and exits 0 immediately with empty stdout
/// (D-18) - nothing here is needed for that, and nothing here may be added that
/// would weaken it. Nothing else about the invocation differs from the
/// sequential pass either: same lock, same recovery, same per-file
/// transactions, same single `runs` row, which is why a killed backfill resumes
/// exactly as phase 2's crash harness proves a killed pass does (D-22).
fn work(data_dir: &Path) -> Result<(), Failure> {
    use terminus_core::ingest::backfill;
    use terminus_core::ingest::pass::{self, PassOutcome};

    let summary = match backfill::run(data_dir, WORKERS)?.outcome {
        // Another pass is already running. Not a failure: the whole point of
        // one lock and no daemon.
        PassOutcome::LockHeld => return Ok(()),
        PassOutcome::Ran(summary) => summary,
    };

    // All three of this process's stdio are `Stdio::null()` when it was
    // spawned, so this reaches nobody in the ordinary case - the `runs` row is
    // the durable record and `terminus status` is where it is read. It is here
    // for the case where a person ran the flag by hand.
    for (path, reason) in summary.failures.iter().chain(&summary.unreadable) {
        eprintln!("{}", pass::note(path, reason));
    }
    Ok(())
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut work = false;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long(WORK_FLAG) => work = true,
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }
    Ok(Args { work })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The estimate over a tree that has never been ingested is the whole tree,
    /// and asking costs no store.
    #[test]
    fn an_unarchived_tree_estimates_every_byte_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        let projects = claude.join("projects").join("-tmp-project");
        std::fs::create_dir_all(&projects).unwrap();
        let mut written = 0u64;
        for name in ["session-basic.jsonl", "session-continuation.jsonl"] {
            let bytes = terminus_core::testkit::fixture_bytes(name);
            written += bytes.len() as u64;
            let uuid = format!("{:08x}-1111-4111-8111-111111111111", written);
            std::fs::write(projects.join(format!("{uuid}.jsonl")), &bytes).unwrap();
        }

        let data_dir = dir.path().join("data");
        let config = Config::from_parts(vec![claude], Vec::new());
        let estimate = estimate(&data_dir, &config).unwrap();

        assert_eq!(estimate.transcripts, 2);
        assert_eq!(estimate.unread_bytes, written);
        assert!(
            !data_dir.exists(),
            "estimating created the data directory it was asked about"
        );
    }

    /// An empty tree is zeroes rather than a failure: a machine that has never
    /// run Claude Code is an ordinary state.
    #[test]
    fn an_empty_tree_estimates_zero() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_parts(vec![dir.path().join("claude")], Vec::new());
        let estimate = estimate(&dir.path().join("data"), &config).unwrap();
        assert_eq!(estimate, Estimate::default());
        assert_eq!(duration(estimate.duration()), "under a second");
    }

    /// The measured rate, applied to the corpus it was measured over, returns
    /// the measurement itself - which is the only claim the printed time makes.
    ///
    /// The second half is the one worth having a test for: the estimate must
    /// NOT be the single-threaded rate divided by the worker count. That model
    /// was measured wrong (four workers bought 8%, not 4x), and an estimate
    /// four times short of the truth is worse than none.
    #[test]
    fn the_estimate_is_the_rate_the_pipeline_was_measured_at() {
        let whole = Estimate {
            transcripts: 3_120,
            unread_bytes: MEASURED_BYTES,
            unreadable: Vec::new(),
        };
        assert_eq!(
            whole.duration(),
            std::time::Duration::from_millis(MEASURED_MS)
        );

        /// D-23's single-threaded measurement, kept here and nowhere else: it
        /// is the number this estimate is deliberately not derived from.
        const SINGLE_THREADED: (u64, u64) = (1_181_116_006, 36_582);
        let divided = SINGLE_THREADED.1 * MEASURED_BYTES / SINGLE_THREADED.0 / WORKERS as u64;
        assert!(
            whole.duration().as_millis() as u64 > divided * 3,
            "the estimate is close to the single-threaded rate over {WORKERS} workers, \
             which is the model the corpus measurement refuted"
        );
    }
}
