//! The relevance injection at the process boundary: the bytes Claude Code
//! reads, and the silence it reads most of the time (INJ-03, AC3, AC7).
//!
//! Spawns rather than library calls, for the reason `tests/recall_cli.rs`
//! gives: the properties under test are the stdout bytes and the exit code, and
//! neither exists inside a library call. What the injection SAYS and which turn
//! it chose are asserted in `terminus-core`'s `tests/inject_prompt.rs`, against
//! stores that file built; what is here is only what the harness sees.
//!
//! The harness below is deliberately a copy of `tests/brief.rs`'s rather than a
//! shared module: a `mod common` would be a third file to keep in step, and the
//! two differ in what they seed and what they send.
//!
//! Every spawn sets `TERMINUS_DATA_DIR`, `TERMINUS_CONFIG_DIR` **and**
//! `CLAUDE_CONFIG_DIR` at temporary directories. Every hook spawns a bare
//! `terminus ingest` of its own, which walks the *configured* roots, so a spawn
//! that set only the data directory would walk the developer's live `~/.claude`
//! tree.
//!
//! The store is seeded from `session-edits.jsonl` through
//! `testkit::copy_rooted_fixture_into` under a root the test owns, never
//! through a fixture that hardcodes `/data/code/verbatim`: AC3 is about a
//! prompt naming a file RELATIVE to the payload's `cwd` finding the ABSOLUTE
//! spelling a past session stored, and both have to be something this test
//! built.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use terminus_core::testkit;

/// The fixture whose `Edit` stores an absolute path under `project-alpha`, and
/// the relative spelling of that same file.
const FIXTURE: &str = "session-edits.jsonl";
const RELATIVE: &str = "crates/gizmo/lantern.rs";

/// Every directory a spawned `terminus` may touch, all of them temporary.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_terminus"));
        command
            .args(args)
            .current_dir(&self.work)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// Archive the rooted fixture through the binary's own ingest path, to
    /// completion, so every hook's own spawned pass finds nothing new.
    fn ingest(&self) {
        let path = testkit::copy_rooted_fixture_into(FIXTURE, &self.work, &self.root);
        let out = self
            .command(&["ingest", path.to_str().unwrap()])
            .output()
            .expect("spawn the ingest");
        assert!(
            out.status.success(),
            "ingest {FIXTURE}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The project directory the fixture's `cwd` names.
    fn project(&self) -> PathBuf {
        testkit::fixture_project(FIXTURE, &self.root)
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

    /// One `UserPromptSubmit`, with the prompt the test is about.
    fn prompt(&self, prompt: &str) -> Output {
        self.hook("UserPromptSubmit", &payload(&self.project(), prompt))
    }
}

/// The `UserPromptSubmit` payload, of the shape
/// `tests/fixtures/hooks/user-prompt-submit.json` carries.
fn payload(cwd: &std::path::Path, prompt: &str) -> Value {
    serde_json::json!({
        "session_id": "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55",
        "transcript_path": "/home/user/.claude/projects/-p/0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55.jsonl",
        "cwd": cwd.to_str().unwrap(),
        "prompt_id": "7c1d4b90-33af-4e05-9a6c-2f8e5b71c0a4",
        "hook_event_name": "UserPromptSubmit",
        "prompt": prompt,
        "session_title": "phase 5: context injection",
    })
}

/// The one JSON object on a hook's stdout, and the context inside it.
///
/// The event name is checked here rather than at each call site because the
/// harness throws `Hook returned incorrect event name` on a mismatch, which is
/// a red banner on the user's prompt rather than a wrong injection (D-01).
fn injected(out: &Output) -> String {
    assert!(
        out.status.success(),
        "exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_else(|| panic!("nothing on stdout"));
    assert_eq!(lines.next(), None, "more than one object: {text:?}");

    let document: Value =
        serde_json::from_str(first).unwrap_or_else(|e| panic!("not one JSON object ({e}): {text}"));
    let inner = document
        .get("hookSpecificOutput")
        .unwrap_or_else(|| panic!("no hookSpecificOutput: {document}"));
    assert_eq!(
        inner.get("hookEventName").and_then(Value::as_str),
        Some("UserPromptSubmit"),
        "{document}"
    );
    inner
        .get("additionalContext")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no additionalContext: {document}"))
        .to_owned()
}

/// Nothing at all on stdout, and exit 0: the answer most prompts get.
fn silent(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{what} wrote {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// AC3, end to end. The prompt names the file the way a user types it and the
/// turn that stored it absolute comes back; the same prompt without the path
/// writes nothing.
#[test]
fn a_prompt_naming_a_file_gets_the_turn_that_edited_it() {
    let bench = bench();
    bench.ingest();

    let context = injected(&bench.prompt(&format!("what changed in {RELATIVE}")));
    assert!(
        context.contains("lanternFlicker"),
        "the injected turn is not the edit that stored the path: {context}"
    );
    assert!(
        context.contains("turn "),
        "nothing names the turn `recall_get` would take: {context}"
    );

    silent(&bench.prompt("what changed"), "the prompt with no path");
}

/// AC7: a prompt whose only match is free text writes nothing.
///
/// Two shapes of it, because they fail differently. The first names nothing
/// structural at all and never opens the store; the second names an
/// identifier-shaped word the archive really does carry - in prose, where it is
/// no entity - so the search returns a hit and the threshold refuses it.
#[test]
fn a_prompt_that_matches_only_free_text_writes_nothing() {
    let bench = bench();
    bench.ingest();

    silent(
        &bench.prompt("how should we think about this in general"),
        "a prompt naming nothing structural",
    );
    silent(
        &bench.prompt("was the gizmo module ever any good"),
        "a prompt matching only prose",
    );
}

/// D-16: the configured budget reaches the process, in characters.
///
/// Written to `terminus.toml` and asserted on the spawned binary's own stdout,
/// because "settings must actually reach the query" is the requirement rather
/// than a detail (`DESIGN-BRIEF.md:239`).
#[test]
fn no_injection_exceeds_the_configured_budget() {
    const BUDGET: usize = 120;

    let bench = bench();
    bench.ingest();
    std::fs::write(
        bench.config_dir.join("terminus.toml"),
        format!("[injection]\nprompt_chars = {BUDGET}\n"),
    )
    .unwrap();

    let context = injected(&bench.prompt(&format!("what changed in {RELATIVE}")));
    assert!(
        context.chars().count() <= BUDGET,
        "{} characters against a configured budget of {BUDGET}: {context}",
        context.chars().count()
    );
    assert!(context.contains("turn "), "{context}");
}
