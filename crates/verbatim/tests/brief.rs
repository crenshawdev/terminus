//! The resume brief at the process boundary: the bytes Claude Code reads, and
//! how long it waited for them (INJ-01, INJ-02, AC1, AC2).
//!
//! Spawns rather than library calls, for the reason `tests/recall_cli.rs`
//! gives: the properties under test are the stdout bytes, the exit code and the
//! wall clock, and none of the three exists inside a library call. What the
//! brief SAYS is asserted in `verbatim-core`'s `tests/inject_brief.rs`, against
//! stores that file built; what is here is only what the harness sees.
//!
//! The harness below is deliberately a copy of `tests/inject.rs`'s rather than
//! a shared module: a `mod common` would be a third file to keep in step, and
//! the two differ in what they seed and how many times they spawn.
//!
//! Every spawn sets `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` **and**
//! `CLAUDE_CONFIG_DIR` at temporary directories. Every hook spawns a bare
//! `verbatim ingest` of its own, which walks the *configured* roots, so a spawn
//! that set only the data directory would walk the developer's live `~/.claude`
//! tree - and, worse here, would archive new sessions between two runs that are
//! asserted to be byte-identical.
//!
//! The store is seeded by running one ingest to completion BEFORE any hook
//! runs, through `testkit::copy_rooted_fixture_into` under a root the test owns
//! (`FIXTURE_ROOT_TOKEN`), never through the fixtures that hardcode
//! `/data/code/verbatim`: a project key has to be something this test built, or
//! the assertion is true only on a checkout at that literal path.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::Instant;

use serde_json::Value;
use verbatim_core::testkit;

/// The fixture whose `cwd` names `project-alpha` beneath the test's own root.
const FIXTURE: &str = "session-recall.jsonl";

/// Every directory a spawned `verbatim` may touch, all of them temporary.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    /// The root the rooted fixtures' `cwd` values were substituted with, and so
    /// the parent of every project directory a payload can name.
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        config_dir,
        claude_dir,
        root,
    }
}

impl Bench {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
        command
            .args(args)
            .current_dir(&self.work)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// Archive one rooted fixture through the binary's own ingest path, to
    /// completion, so that every hook's own spawned pass finds nothing new and
    /// the store is unchanged across the runs below.
    fn ingest(&self, fixture: &str) {
        let path = testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root);
        let out = self
            .command(&["ingest", path.to_str().unwrap()])
            .output()
            .expect("spawn the ingest");
        assert!(
            out.status.success(),
            "ingest {fixture}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The project directory one rooted fixture's `cwd` names.
    fn project(&self, fixture: &str) -> PathBuf {
        testkit::fixture_project(fixture, &self.root)
    }

    /// One hook invocation: the payload on stdin, then EOF, the way Claude Code
    /// writes it (`stdin.write(payload + "\n"); stdin.end()`).
    fn hook(&self, event: &str, payload: &Value) -> Output {
        let mut child = self
            .command(&["hook", event])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the hook");
        let line = format!("{payload}\n");
        child
            .stdin
            .take()
            .expect("the hook's stdin")
            .write_all(line.as_bytes())
            .expect("write the payload");
        child.wait_with_output().expect("wait for the hook")
    }
}

/// The `SessionStart` payload, of the shape `tests/fixtures/hooks/*.json` carry.
fn payload(cwd: &std::path::Path) -> Value {
    serde_json::json!({
        "session_id": "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55",
        "transcript_path": "/home/user/.claude/projects/-p/0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55.jsonl",
        "cwd": cwd.to_str().unwrap(),
        "hook_event_name": "SessionStart",
        "source": "startup",
    })
}

/// The first `NN:NN` in a string, which is what a time of day looks like once
/// the date has been rounded to the day.
///
/// Scanned over characters rather than bytes: the brief quotes turns, a turn is
/// any UTF-8 the transcript carried, and a byte window would split a multi-byte
/// character and compare nonsense.
fn time_of_day(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .windows(5)
        .find(|w| {
            w[0].is_ascii_digit()
                && w[1].is_ascii_digit()
                && w[2] == ':'
                && w[3].is_ascii_digit()
                && w[4].is_ascii_digit()
        })
        .map(|w| w.iter().collect())
}

/// AC2. Two `SessionStart` runs against an unchanged store, byte for byte.
///
/// The stated rationale is the Anthropic prefix cache, which claude-mem busts
/// every sixty seconds with minute-granularity timestamps and which nothing
/// local can observe. The property being built is the byte identity itself, and
/// it stands on its own: a brief that changes while the archive does not is a
/// brief nobody can reason about.
///
/// The time-of-day half is the falsifiable one. Dates are rounded to the day
/// (INJ-02), so no `NN:NN` may appear anywhere in the output - not in the
/// rendered date, not carried in from a quoted turn.
#[test]
fn two_session_starts_against_an_unchanged_store_are_byte_identical() {
    let bench = bench();
    bench.ingest(FIXTURE);
    let payload = payload(&bench.project(FIXTURE));

    let first = bench.hook("SessionStart", &payload);
    let second = bench.hook("SessionStart", &payload);

    for (n, output) in [&first, &second].into_iter().enumerate() {
        assert!(
            output.status.success(),
            "run {n} exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty(), "run {n} emitted no brief");
    }

    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&second.stdout),
        "the brief changed while the archive did not"
    );

    let text = String::from_utf8_lossy(&first.stdout);
    assert_eq!(
        time_of_day(&text),
        None,
        "the brief carries a time of day: {text}"
    );
}

// ---------------------------------------------------------------------------
// AC1: the brief inside its wall budget, over a hundred runs

/// AC1. A hundred `SessionStart` spawns against a store with history: one
/// object each, inside the configured budget, at p99 under the wall budget.
///
/// The 10 ms is the same number `tests/hook.rs` asserts for the four events
/// that write nothing, and D-11 says it now has to absorb a read-only store
/// open, the two counts, the last-session lookup and one blob decompression on
/// top of the 0.65-0.76 ms that file measured in a debug build. That headroom
/// is the whole reason the budget did not have to move.
///
/// The percentiles are printed as well as asserted: a regression that says
/// "p99 12.4 ms against 10 ms" is a number the next run can be compared
/// against, where a bare failed assertion is not.
#[test]
fn a_hundred_session_starts_stay_inside_the_budget_and_the_wall_clock() {
    const RUNS: usize = 100;
    const WALL_MS: f64 = 10.0;
    /// The `[injection] brief_chars` this test configures, well above the
    /// brief the fixture produces: what is measured here is the wall clock,
    /// and `verbatim-core`'s `tests/inject_brief.rs` is where a budget that
    /// binds is asserted.
    const BUDGET: usize = 1_200;

    let bench = bench();
    bench.ingest(FIXTURE);
    std::fs::write(
        bench.config_dir.join("verbatim.toml"),
        format!("[injection]\nbrief_chars = {BUDGET}\n"),
    )
    .unwrap();
    let payload = payload(&bench.project(FIXTURE));

    let mut millis = Vec::with_capacity(RUNS);
    for run in 0..RUNS {
        let started = Instant::now();
        let output = bench.hook("SessionStart", &payload);
        millis.push(started.elapsed().as_secs_f64() * 1000.0);

        assert!(
            output.status.success(),
            "run {run} exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );

        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        let first = lines
            .next()
            .unwrap_or_else(|| panic!("run {run} wrote nothing to stdout"));
        assert_eq!(
            lines.next(),
            None,
            "run {run} wrote more than the one object: {text:?}"
        );
        let document: Value = serde_json::from_str(first)
            .unwrap_or_else(|e| panic!("run {run} is not one JSON object ({e}): {text:?}"));

        let inner = document
            .get("hookSpecificOutput")
            .unwrap_or_else(|| panic!("run {run} carries no hookSpecificOutput: {document}"));
        assert_eq!(
            inner.get("hookEventName").and_then(Value::as_str),
            Some("SessionStart"),
            "the harness throws `Hook returned incorrect event name` on a \
             mismatch: {document}"
        );
        let context = inner
            .get("additionalContext")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("run {run} carries no additionalContext: {document}"));
        assert!(
            context.chars().count() <= BUDGET,
            "run {run} is {} characters against a configured budget of {BUDGET}",
            context.chars().count()
        );
    }

    millis.sort_by(f64::total_cmp);
    let p50 = millis[RUNS / 2 - 1];
    let p99 = millis[(RUNS * 99) / 100 - 1];
    println!("SessionStart brief: p50 {p50:.2} ms, p99 {p99:.2} ms over {RUNS} runs");
    assert!(p99 < WALL_MS, "p99 {p99:.2} ms is over {WALL_MS} ms");
}
