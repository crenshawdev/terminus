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
use verbatim_core::config::REDACTED;
use verbatim_core::testkit;

/// The fixture whose `cwd` names `project-alpha` beneath the test's own root.
const FIXTURE: &str = "session-recall.jsonl";

/// The fixture whose `cwd` names `project-gamma`, and whose last `user` record
/// is a `<task-notification>` envelope rather than a prompt.
///
/// The byte-identity test seeds from this one and not from [`FIXTURE`] (D-15).
/// `session-recall.jsonl`'s only `user` record is a plain text block, so the
/// turn the brief quotes is the same one under INJ-07's rule and under the rule
/// it replaced - a test on it cannot observe this phase's change at all, and
/// two identical briefs of the wrong turn would satisfy byte identity just as
/// well as two right ones.
const MOVED_FIXTURE: &str = "session-envelope.jsonl";

/// The one `user` record of [`MOVED_FIXTURE`] a person typed, which is what the
/// brief must quote.
const TYPED: &str = "wire up the quince exporter";

/// The two it did not: the tool result's output and the envelope's text. Either
/// one in the brief is the defect INJ-07 exists to close.
const NOT_TYPED: [&str; 2] = ["press.toml", "background scan of the modules directory"];

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
/// Two halves are falsifiable, and both have to be: two empty briefs are
/// byte-identical, and so are two briefs quoting the wrong turn.
///
/// Dates are rounded to the day (INJ-02), so no `NN:NN` may appear anywhere in
/// the output - not in the rendered date, not carried in from a quoted turn.
/// And the store is seeded from [`MOVED_FIXTURE`], whose last `user` record is
/// a harness envelope, so the bytes have to carry the prompt somebody typed and
/// neither of the two records nobody did (INJ-07).
#[test]
fn two_session_starts_against_an_unchanged_store_are_byte_identical() {
    let bench = bench();
    bench.ingest(MOVED_FIXTURE);
    let payload = payload(&bench.project(MOVED_FIXTURE));

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

    assert!(
        text.contains(TYPED),
        "the brief does not quote the prompt somebody typed: {text}"
    );
    for absent in NOT_TYPED {
        assert!(
            !text.contains(absent),
            "the brief carries {absent:?}, out of a user record nobody typed: {text}"
        );
    }
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
///
/// This one stays on [`FIXTURE`] while the byte-identity test above moved to
/// [`MOVED_FIXTURE`]: the numbers printed here have been measured against
/// `session-recall.jsonl` since phase 5, and re-pointing them at a different
/// session would silently make every earlier reading incomparable.
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

// ---------------------------------------------------------------------------
// AC1 and AC4: the brief's quote under the redaction knob (PRIV-03)

impl Bench {
    /// `hook`, with extra environment variables set on the spawn.
    ///
    /// Its own method rather than a parameter on `hook` because every other
    /// spawn's claim is about the environment `command` builds; carrying
    /// anything else is the exception under test and should read like one.
    fn hook_with_env(&self, event: &str, payload: &Value, env: &[(&str, &str)]) -> Output {
        let mut command = self.command(&["hook", event]);
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = command
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

/// The fixture whose `cwd` names `project-delta` and whose turns each carry one
/// credential shape with no surrounding name-keyed context.
const SECRETS_FIXTURE: &str = "session-secrets.jsonl";

/// The `Cookie` value in the last turn of that fixture a person typed, which is
/// the turn the brief quotes.
const COOKIE_SENTINEL: &str = "sid-VBEGRESS-qcrumb-1f9";

/// The fragment every planted sentinel in that fixture shares.
///
/// Asserted on rather than the one value under test: the brief quotes two turns
/// and counts a whole session, and one surviving sentinel of any shape is a
/// leak.
const SENTINEL_MARK: &str = "VBEGRESS";

/// PRIV-03's knob, written the only way anything can turn it on.
///
/// A file in the config directory the spawn points at, and no environment
/// variable (D-07): the hook loads `Config` in-process, so a variable it could
/// inherit would be a way to turn this off from outside the config file.
const KNOB_ON: &str = "[privacy]\nredact_recall = true\n";

/// The brief a `SessionStart` run emitted, out of the payload the harness reads.
///
/// Asserts the frame as it goes - exit 0, exactly one object on stdout, the
/// event name the harness rejects a mismatch on - so a test below can be about
/// the text alone.
fn additional_context(output: &Output, label: &str) -> String {
    assert!(
        output.status.success(),
        "{label} exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("{label} wrote nothing to stdout"));
    assert_eq!(
        lines.next(),
        None,
        "{label} wrote more than the one object: {text:?}"
    );
    let document: Value = serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("{label} is not one JSON object ({e}): {text:?}"));
    let inner = document
        .get("hookSpecificOutput")
        .unwrap_or_else(|| panic!("{label} carries no hookSpecificOutput: {document}"));
    assert_eq!(
        inner.get("hookEventName").and_then(Value::as_str),
        Some("SessionStart"),
        "{label}: {document}"
    );
    inner
        .get("additionalContext")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{label} carries no additionalContext: {document}"))
        .to_owned()
}

/// AC1's fourth output, asserted where Claude Code reads it: the brief's quoted
/// prompt carries the planted `Cookie` value with no `verbatim.toml`, and the
/// marker instead once the knob is on.
///
/// Both halves are needed. A build that filtered nothing passes the first and a
/// build that emitted an empty brief passes the second, so the same spawn is
/// run twice over one store with nothing changed between them but a file on
/// disk - and the second run's frame is asserted too, because a hook that
/// crashed rather than filtered would also carry no sentinel.
#[test]
fn the_knob_filters_the_briefs_quoted_prompt_at_the_hook_boundary() {
    let bench = bench();
    bench.ingest(SECRETS_FIXTURE);
    let payload = payload(&bench.project(SECRETS_FIXTURE));

    let default = bench.hook("SessionStart", &payload);
    let context = additional_context(&default, "the run with no verbatim.toml");
    assert!(
        context.contains(COOKIE_SENTINEL),
        "the premise: the brief really does quote a turn carrying a credential:\n{context}"
    );

    std::fs::write(bench.config_dir.join("verbatim.toml"), KNOB_ON).unwrap();

    let filtered = bench.hook("SessionStart", &payload);
    let context = additional_context(&filtered, "the run with the knob on");
    assert!(
        context.contains(REDACTED),
        "the marker names what went:\n{context}"
    );
    assert!(
        !context.contains(SENTINEL_MARK),
        "a sentinel survived into the brief:\n{context}"
    );
    assert!(
        !String::from_utf8_lossy(&filtered.stdout).contains(SENTINEL_MARK),
        "a sentinel survived somewhere else in the payload:\n{}",
        String::from_utf8_lossy(&filtered.stdout)
    );
}

/// AC4's hook half: no environment variable reaches past the knob.
///
/// D-07 chose a per-invocation CLI flag over an environment variable precisely
/// because the hook loads `Config` in-process and would inherit one. The
/// variables spelled here are the three a reader of `--raw` would reach for:
/// bare, `VERBATIM_`-prefixed, and the config key itself.
#[test]
fn no_environment_variable_reaches_past_the_knob_on_the_session_start_brief() {
    let bench = bench();
    bench.ingest(SECRETS_FIXTURE);
    let payload = payload(&bench.project(SECRETS_FIXTURE));
    std::fs::write(bench.config_dir.join("verbatim.toml"), KNOB_ON).unwrap();

    let output = bench.hook_with_env(
        "SessionStart",
        &payload,
        &[
            ("RAW", "1"),
            ("VERBATIM_RAW", "1"),
            ("VERBATIM_REDACT_RECALL", "false"),
        ],
    );
    let context = additional_context(&output, "the run carrying raw-looking variables");
    assert!(context.contains(REDACTED), "{context}");
    assert!(
        !context.contains(SENTINEL_MARK),
        "an environment variable unfiltered the brief:\n{context}"
    );
}
