//! AC8 / ING-11: `verbatim backfill` returns the shell before the archive is
//! full, and costs a hook firing beside it nothing.
//!
//! The three properties here are the ones a user experiences and a unit test
//! cannot reach, because all three are about a second process:
//!
//! 1. the command prints its estimate and returns in under 100 ms **with the
//!    store still empty**, which is what makes "returns immediately" a fact
//!    rather than a fast pass over a small tree;
//! 2. the work really does continue afterwards - without this the first
//!    assertion is satisfied by a command that does nothing at all;
//! 3. an `ingest` launched while the backfill runs loses the lock race and exits
//!    0 in under 50 ms with empty stdout, exactly as `tests/lock_race.rs`
//!    asserts for two ingests (D-18).
//!
//! The tree is deliberately larger than the fixture set: a backfill over three
//! small transcripts finishes inside the 100 ms budget, and every assertion
//! about "still empty at that instant" would then be a coin flip. A megabyte of
//! transcript makes the window real.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use verbatim_core::ingest::{self, Attempt};
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::testkit;

/// AC8's budget for handing the shell back.
const RETURN_BUDGET: Duration = Duration::from_millis(100);

/// ING-02's budget for a contended run, the same one `tests/lock_race.rs` uses.
const CONTENDED_BUDGET: Duration = Duration::from_millis(50);

/// How long a backfill of this tree is allowed to take before the test gives up
/// on it. Generous: this is a debug build and the point is convergence, not
/// speed.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(120);

/// Every directory a spawned `verbatim` may resolve, all of them temporary.
///
/// The same rule `tests/crash.rs` states: a bare pass walks the *configured*
/// roots, so a spawn that set only `VERBATIM_DATA_DIR` would walk the
/// developer's real `~/.claude` and archive 2,000 private transcripts into a
/// temp directory on every `cargo test`. All three variables, every time.
pub struct Dirs {
    _dir: tempfile::TempDir,
    data: PathBuf,
    config: PathBuf,
    claude: PathBuf,
}

impl Dirs {
    fn new() -> Dirs {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let data = root.join("data");
        let config = root.join("config");
        let claude = root.join("claude");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        Dirs {
            _dir: dir,
            data,
            config,
            claude,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
        command
            .args(args)
            .env("VERBATIM_DATA_DIR", &self.data)
            .env("VERBATIM_CONFIG_DIR", &self.config)
            .env("CLAUDE_CONFIG_DIR", &self.claude);
        command
    }

    fn db(&self) -> PathBuf {
        self.data.join(DB_FILE_NAME)
    }

    /// Sessions archived right now.
    ///
    /// Zero for a store that does not exist yet, and zero for one that cannot
    /// be read at this instant: the backfill is writing to it from another
    /// process, and this is a progress reading rather than an assertion about
    /// the store's health. Every test here that needs the store to be sound
    /// reads it again once the backfill has finished.
    fn sessions(&self) -> i64 {
        if !self.db().exists() {
            return 0;
        }
        let Ok(conn) = Connection::open(self.db()) else {
            return 0;
        };
        conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Block until the store holds `expected` sessions, or fail.
    fn wait_for_sessions(&self, expected: i64) {
        let deadline = Instant::now() + COMPLETION_TIMEOUT;
        while self.sessions() < expected {
            assert!(
                Instant::now() < deadline,
                "the detached backfill never finished: {} of {expected} sessions after {:?}",
                self.sessions(),
                COMPLETION_TIMEOUT
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            self.sessions(),
            expected,
            "the backfill archived a different number of sessions than the tree holds"
        );
    }

    /// Block until something else holds the ingest lock, so a contended run is
    /// contended rather than lucky.
    fn wait_for_the_lock(&self) {
        let deadline = Instant::now() + COMPLETION_TIMEOUT;
        loop {
            match ingest::lock::try_acquire(&self.data) {
                // Held by the detached backfill: exactly the state to race
                // against.
                Ok(Attempt::Held) => return,
                // Free, or the data directory is not there yet. Either way the
                // backfill has not reached it; the guard is dropped
                // immediately so it is never this test that holds it.
                Ok(Attempt::Acquired(guard)) => drop(guard),
                Err(_) => {}
            }
            assert!(
                Instant::now() < deadline,
                "the detached backfill never took the ingest lock"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// A transcript large enough that archiving it cannot finish inside the budgets
/// this file asserts. Built from the fixtures, so its records are real ones.
fn big_transcript(dir: &Path, name: &str, repeats: usize) -> PathBuf {
    let mut bytes = Vec::new();
    for _ in 0..repeats {
        bytes.extend_from_slice(&testkit::fixture_bytes("session-basic.jsonl"));
        bytes.extend_from_slice(&testkit::fixture_bytes("session-large-record.jsonl"));
    }
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    path
}

/// A transcript tree of four sessions across two projects, one of them a
/// sidecar at the depth real ones sit at, and one of them a megabyte.
///
/// Returns how many sessions a completed backfill must produce.
fn build_tree(claude: &Path) -> i64 {
    let projects = claude.join("projects");
    let one = projects.join("-tmp-backfill-one");
    let two = projects.join("-tmp-backfill-two");

    // Sorts first, so the walk spends its first hundreds of milliseconds here.
    big_transcript(&one, "11111111-1111-4111-8111-111111111111.jsonl", 8);
    place(
        &one,
        "22222222-2222-4222-8222-222222222222.jsonl",
        "session-basic.jsonl",
    );
    place(
        &one,
        "33333333-3333-4333-8333-333333333333/subagents/agent-a.jsonl",
        "subagents/agent-alpha.jsonl",
    );
    place(
        &two,
        "44444444-4444-4444-8444-444444444444.jsonl",
        "session-continuation.jsonl",
    );
    4
}

fn place(project: &Path, name: &str, fixture: &str) -> PathBuf {
    let dest = project.join(name);
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
    dest
}

/// AC8's first half: the estimate, then the shell, then the work.
#[test]
fn backfill_prints_an_estimate_returns_at_once_and_finishes_behind_the_shell() {
    let dirs = Dirs::new();
    let sessions = build_tree(&dirs.claude);

    let started = Instant::now();
    let output = dirs.command(&["backfill"]).output().unwrap();
    let elapsed = started.elapsed();
    // Read before anything else is asserted: every microsecond spent on the
    // assertions below is a microsecond the detached child spends archiving.
    let archived_on_return = dirs.sessions();

    assert!(
        output.status.success(),
        "backfill exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < RETURN_BUDGET,
        "backfill took {elapsed:?} to return, over the {RETURN_BUDGET:?} budget"
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(&format!("{sessions} transcript(s)")),
        "the estimate must name how many transcripts there are: {stdout}"
    );
    assert!(
        stdout.contains("not yet archived"),
        "the estimate must name a byte total: {stdout}"
    );
    assert!(
        stdout.contains("an estimate from a measured rate"),
        "the estimate must say it is an estimate from a measured rate: {stdout}"
    );

    assert_eq!(
        archived_on_return, 0,
        "the shell came back with the work already done, so it never proved it was detached"
    );

    // The control. Without it, everything above is satisfied by a command that
    // printed two lines and archived nothing.
    dirs.wait_for_sessions(sessions);
}

/// D-18: a hook firing during a backfill costs the hook nothing.
///
/// The same property `tests/lock_race.rs` asserts for two ingests, against the
/// process the user actually has running for minutes.
#[test]
fn an_ingest_during_a_backfill_loses_the_race_and_exits_at_once() {
    let dirs = Dirs::new();
    let sessions = build_tree(&dirs.claude);

    let output = dirs.command(&["backfill"]).output().unwrap();
    assert!(output.status.success());

    // Race the backfill only once it is provably mid-pass. Polling for the
    // lock is what makes this a contended run rather than a hope: an ingest
    // that arrived first would take the lock, walk the whole tree and blow the
    // budget for a reason that is not the one under test.
    dirs.wait_for_the_lock();

    let started = Instant::now();
    let contended = dirs.command(&["ingest"]).output().unwrap();
    let elapsed = started.elapsed();

    assert!(
        contended.status.success(),
        "the contended ingest exited {:?}: {}",
        contended.status.code(),
        String::from_utf8_lossy(&contended.stderr)
    );
    assert!(
        elapsed < CONTENDED_BUDGET,
        "the contended ingest took {elapsed:?}, over the {CONTENDED_BUDGET:?} budget"
    );
    assert!(
        contended.stdout.is_empty(),
        "the contended ingest wrote to stdout: {:?}",
        String::from_utf8_lossy(&contended.stdout)
    );

    // And the backfill it lost to still finishes.
    dirs.wait_for_sessions(sessions);
}
