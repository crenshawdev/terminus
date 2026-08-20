//! `verbatim backfill`: archive the history already on disk, without the user
//! waiting for it (ING-11, AC8).
//!
//! # The estimate comes first, and it is read-only
//!
//! A user who has just installed verbatim is about to hand it a gigabyte of
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
//! use ([`crate::cmd::spawn`], D-03), so `verbatim backfill` returns in
//! milliseconds with a gigabyte still to archive. There is no log file by
//! design: the `runs` row the pass writes is the record, and `verbatim status`
//! is where it is read.
//!
//! Resumability needs nothing here (D-22). It is the per-file watermarks and
//! the per-file transactions phase 2 already ships, which is why a killed
//! backfill converges on the same store as an uninterrupted one without this
//! module holding any state of its own.

use std::collections::HashMap;
use std::path::Path;

use verbatim_core::{discover, Config};

use super::read::{self, Opened};
use super::Failure;

/// How many threads the backfill parses and compresses on (D-11).
///
/// Fixed here, in one place, and deliberately small: the ceiling is SQLite's
/// single writer rather than the core count, and compression is the only stage
/// that scales with threads. It is also what the estimate divides the measured
/// single-threaded rate by, so the number a user is shown and the number of
/// workers that turn up are the same fact.
pub const WORKERS: usize = 4;

/// D-23's measured rate: the full real corpus, 1.1 GB across 2,248 files,
/// ingested single-threaded in 36,582 ms on 2026-08-13.
///
/// A measurement rather than a guess, and it is stated as one in the output.
/// The two numbers are kept side by side rather than pre-divided so the
/// provenance survives: a later measurement replaces a pair that can be read
/// off a `runs` row.
const MEASURED_BYTES: u64 = 1_181_116_006;
const MEASURED_MS: u64 = 36_582;

/// What a backfill would have to read, before it reads any of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Estimate {
    /// Transcripts discovery yields under the configured roots.
    pub transcripts: usize,
    /// Bytes past the stored watermarks: the work that is actually left.
    pub unread_bytes: u64,
}

impl Estimate {
    /// How long that is likely to take, from [`MEASURED_BYTES`] over
    /// [`MEASURED_MS`], divided across [`WORKERS`].
    pub fn duration(&self) -> std::time::Duration {
        let ms = u128::from(self.unread_bytes) * u128::from(MEASURED_MS)
            / u128::from(MEASURED_BYTES)
            / WORKERS as u128;
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

/// The estimate as the three lines a user reads.
pub fn print_estimate(estimate: &Estimate) {
    println!(
        "{} transcript(s), {} not yet archived",
        estimate.transcripts,
        bytes(estimate.unread_bytes)
    );
    println!(
        "about {} across {WORKERS} workers - an estimate from a measured rate \
         ({} in {}, single-threaded), not a promise",
        duration(estimate.duration()),
        bytes(MEASURED_BYTES),
        duration(std::time::Duration::from_millis(MEASURED_MS)),
    );
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
/// `verbatim ingest`, so nothing is granted by knowing it. It is the argument
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
    start(&data_dir)
}

/// Hand the work to a process that outlives this one, and return (AC8,
/// ING-11).
///
/// Through the same double fork the hooks use (D-03), so a backfill started
/// from a shell that is then closed, or from a hook Claude Code later kills,
/// keeps going. Nothing is waited for: the point of the command is that the
/// shell comes back.
pub fn start(data_dir: &Path) -> Result<(), Failure> {
    super::spawn::detached(&["backfill", &format!("--{WORK_FLAG}")]).map_err(|e| {
        Failure::Operational(format!(
            "the backfill could not be started: {e}. \
             run `verbatim ingest` to archive the tree in the foreground, or let the \
             next hook do it - the work is the same either way, and the store at {} \
             is untouched",
            data_dir.display()
        ))
    })
}

/// The detached child: one ordinary tree pass, and nothing on stdout.
///
/// The pass takes the ingest lock before it opens the store, so a hook firing
/// during a backfill loses the race and exits 0 immediately with empty stdout
/// (D-18) - nothing here is needed for that, and nothing here may be added that
/// would weaken it.
fn work(data_dir: &Path) -> Result<(), Failure> {
    use verbatim_core::ingest::pass::{self, PassOutcome};

    let summary = match pass::run(data_dir)? {
        // Another pass is already running. Not a failure: the whole point of
        // one lock and no daemon.
        PassOutcome::LockHeld => return Ok(()),
        PassOutcome::Ran(summary) => summary,
    };

    // All three of this process's stdio are `Stdio::null()` when it was
    // spawned, so this reaches nobody in the ordinary case - the `runs` row is
    // the durable record and `verbatim status` is where it is read. It is here
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
            let bytes = verbatim_core::testkit::fixture_bytes(name);
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
    /// the measurement divided by the worker count - which is the only claim
    /// the printed time makes.
    #[test]
    fn the_estimate_is_the_measured_rate_over_the_workers() {
        let whole = Estimate {
            transcripts: 2_248,
            unread_bytes: MEASURED_BYTES,
        };
        assert_eq!(
            whole.duration(),
            std::time::Duration::from_millis(MEASURED_MS / WORKERS as u64)
        );
    }
}
