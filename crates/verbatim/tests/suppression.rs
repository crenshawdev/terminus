//! What injection refuses to say twice (INJ-04, INJ-05, AC4, AC5).
//!
//! At the process boundary, like `tests/prompt.rs` and for the same reason: the
//! property under test is what the harness receives, and "the second prompt
//! wrote nothing" is a claim about bytes on a pipe. What it *chose* is asserted
//! in `verbatim-core`; what is here is the answer Claude Code actually gets,
//! plus the file a person can open afterwards to see why.
//!
//! The three suppressions fail in three different ways, so each gets a test and
//! each gets a control that fires. A suppression test with no control is
//! indistinguishable from a retrieval that never worked - the whole phase can
//! be silent for free.
//!
//! Every spawn sets `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` **and**
//! `CLAUDE_CONFIG_DIR` at temporary directories: every hook spawns a bare
//! `verbatim ingest` of its own, which walks the *configured* roots, so a spawn
//! that set only the data directory would walk the developer's live `~/.claude`
//! tree.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use verbatim_core::testkit;

/// The fixture whose `Edit` stores an absolute path under `project-alpha`, and
/// the relative spelling of that same file.
const FIXTURE: &str = "session-edits.jsonl";
const RELATIVE: &str = "crates/gizmo/lantern.rs";

/// A transcript path that names no file: what a payload carries when the
/// session's own file is not the point of the test.
///
/// Not an empty string and not a fabricated absolute path, but a name under the
/// bench's own work directory - a path that cannot canonicalize is exactly the
/// case D-15 says suppresses nothing, and this keeps that case honest without
/// reaching outside the temporary tree.
const ABSENT: &str = "no-such-session.jsonl";

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
        let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
        command
            .args(args)
            .current_dir(&self.work)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// Archive the rooted fixture through the binary's own ingest path, to
    /// completion, and answer with the transcript the archive now keys on.
    ///
    /// Before any hook runs, always: a hook spawns a detached ingest of its
    /// own, and an explicit ingest racing that one for the lock would answer
    /// `LockHeld` and archive nothing at all.
    fn ingest_fixture(&self) -> PathBuf {
        let path = testkit::copy_rooted_fixture_into(FIXTURE, &self.work, &self.root);
        self.ingest(&path);
        path
    }

    fn ingest(&self, path: &Path) {
        let out = self
            .command(&["ingest", path.to_str().unwrap()])
            .output()
            .expect("spawn the ingest");
        assert!(
            out.status.success(),
            "ingest {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The project directory the fixture's `cwd` names.
    fn project(&self) -> PathBuf {
        testkit::fixture_project(FIXTURE, &self.root)
    }

    /// Write one session of assistant records into the same project and archive
    /// it, at timestamps later than the fixture's so it is the LAST session.
    fn archive(&self, name: &str, day: &str, records: Vec<Value>) -> PathBuf {
        let session = format!("{:0>8}-0000-4000-8000-000000000000", name.len());
        let mut body = String::new();
        for (n, message) in records.into_iter().enumerate() {
            let record = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": self.project().to_string_lossy(),
                "sessionId": session,
                "type": message["role"],
                "uuid": format!("{session}-{n}"),
                "timestamp": format!("{day}T10:{:02}:00.000Z", n),
                "requestId": format!("req_{n}"),
                "message": message,
            });
            body.push_str(&record.to_string());
            body.push('\n');
        }
        let path = self.work.join(name);
        std::fs::write(&path, body).unwrap();
        self.ingest(&path);
        path
    }

    /// One hook invocation: the payload on stdin, then EOF, the way Claude Code
    /// writes it.
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

    /// One `UserPromptSubmit` under a named session, with a named transcript.
    fn prompt(&self, session: &str, transcript: &Path, prompt: &str) -> Output {
        self.hook(
            "UserPromptSubmit",
            &serde_json::json!({
                "session_id": session,
                "transcript_path": transcript.to_str().unwrap(),
                "cwd": self.project().to_str().unwrap(),
                "hook_event_name": "UserPromptSubmit",
                "prompt": prompt,
            }),
        )
    }

    /// One `SessionStart` under a named session.
    fn session_start(&self, session: &str, source: &str) -> Output {
        self.hook(
            "SessionStart",
            &serde_json::json!({
                "session_id": session,
                "transcript_path": self.work.join(ABSENT).to_str().unwrap(),
                "cwd": self.project().to_str().unwrap(),
                "hook_event_name": "SessionStart",
                "source": source,
            }),
        )
    }

    /// One session's injection scratch, as JSON.
    ///
    /// Read as bytes off the disk and parsed here rather than through
    /// `state::State`: AC4 asks for a reason a person can read out of the file,
    /// and a deserializer that maps `"already_injected"` back onto the enum it
    /// came from would pass on a file spelling it `3`.
    fn state(&self, session: &str) -> Value {
        let path = self
            .data_dir
            .join(verbatim_core::inject::state::DIR_NAME)
            .join(format!("{session}.json"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        serde_json::from_slice(&bytes).expect("the state file is JSON")
    }
}

/// A turn that says something and stores nothing: prose, which emits no entity.
fn says(role: &str, text: &str) -> Value {
    serde_json::json!({
        "role": role,
        "model": "claude-opus-5",
        "content": [{"type": "text", "text": text}],
    })
}

/// An `Edit` of one file: the `path` entity a prompt can reach structurally.
fn edits(file: &str) -> Value {
    serde_json::json!({
        "role": "assistant",
        "model": "claude-opus-5",
        "content": [{
            "type": "tool_use",
            "id": "toolu_supp01",
            "name": "Edit",
            "input": {
                "file_path": file,
                "old_string": "    let beam = beaconWaver(seed);",
                "new_string": "    let beam = beaconSteady(seed);",
            },
        }],
    })
}

/// The one JSON object on a hook's stdout, and the context inside it.
fn injected(out: &Output, event: &str) -> String {
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
        Some(event),
        "{document}"
    );
    inner
        .get("additionalContext")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no additionalContext: {document}"))
        .to_owned()
}

/// Nothing at all on stdout, and exit 0.
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

/// The turn ids one field of a state file holds.
fn ids(state: &Value, field: &str) -> Vec<i64> {
    state
        .get(field)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no {field} array: {state}"))
        .iter()
        .map(|id| id.as_i64().expect("a turn id is an integer"))
        .collect()
}

/// The suppressions recorded, as `(turn_id, reason)`.
fn suppressed(state: &Value) -> Vec<(i64, String)> {
    state
        .get("suppressed")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no suppressed array: {state}"))
        .iter()
        .map(|entry| {
            (
                entry["turn_id"].as_i64().expect("a turn id"),
                entry["reason"].as_str().expect("a reason").to_owned(),
            )
        })
        .collect()
}

/// AC4, end to end: the same entity-bearing prompt twice under one session is
/// answered once, and the file says which turn was held back and why.
#[test]
fn the_same_prompt_twice_in_one_session_is_answered_once() {
    const SESSION: &str = "aaaaaaaa-1111-4000-8000-000000000001";

    let bench = bench();
    bench.ingest_fixture();
    let absent = bench.work.join(ABSENT);
    let asked = format!("what changed in {RELATIVE}");

    let first = injected(&bench.prompt(SESSION, &absent, &asked), "UserPromptSubmit");
    assert!(
        first.contains("lanternFlicker"),
        "the first prompt did not get the edit that stored the path: {first}"
    );

    silent(
        &bench.prompt(SESSION, &absent, &asked),
        "the same prompt again in the same session",
    );

    let state = bench.state(SESSION);
    let injected_ids = ids(&state, "injected");
    assert_eq!(injected_ids.len(), 1, "{state}");
    assert!(
        first.contains(&format!("turn {}", injected_ids[0])),
        "the state file names a turn the injection did not: {first}"
    );
    assert_eq!(
        suppressed(&state),
        vec![(injected_ids[0], "already_injected".to_string())],
        "the suppressed turn and its reason are not in the file: {state}"
    );

    // The control, and the falsifier for the whole file: another session has
    // been given nothing, so the same prompt is answered there.
    let other = injected(
        &bench.prompt("aaaaaaaa-1111-4000-8000-000000000002", &absent, &asked),
        "UserPromptSubmit",
    );
    assert_eq!(other, first, "the suppression is not per session");
}

/// D-15: a turn of the session the user is looking at is never injected, and
/// the payload's own `transcript_path` is how that session is recognized.
///
/// The two halves differ in one field. The suppressed run names the transcript
/// the archive keyed the fixture on; the control names a file that is not
/// there, which is the case D-15 says suppresses nothing.
#[test]
fn a_turn_of_the_session_on_screen_is_never_injected() {
    const SESSION: &str = "bbbbbbbb-1111-4000-8000-000000000001";

    let bench = bench();
    let transcript = bench.ingest_fixture();
    let asked = format!("what changed in {RELATIVE}");

    silent(
        &bench.prompt(SESSION, &transcript, &asked),
        "a prompt whose only candidate is a turn of its own session",
    );

    let state = bench.state(SESSION);
    assert_eq!(ids(&state, "injected"), Vec::<i64>::new(), "{state}");
    let refused = suppressed(&state);
    assert_eq!(refused.len(), 1, "{state}");
    assert_eq!(refused[0].1, "visible_in_session", "{state}");

    let control = injected(
        &bench.prompt(
            "bbbbbbbb-1111-4000-8000-000000000002",
            &bench.work.join(ABSENT),
            &asked,
        ),
        "UserPromptSubmit",
    );
    assert!(
        control.contains(&format!("turn {}", refused[0].0)),
        "the suppressed turn is not the one the same prompt otherwise gets: {control}"
    );
}

/// INJ-04's third suppression: the resume brief quoted it, so the prompt does
/// not repeat it back.
///
/// The archived session's LAST assistant turn has to be the entity-bearing one,
/// because that is the turn the brief quotes - a fixture whose last turn is
/// prose would make this test pass with the brief carrying nothing at all.
#[test]
fn a_turn_the_brief_already_quoted_is_not_injected_again() {
    const SESSION: &str = "cccccccc-1111-4000-8000-000000000001";

    let bench = bench();
    bench.ingest_fixture();
    let beacon = bench.project().join("crates/gizmo/beacon.rs");
    bench.archive(
        "session-beacon.jsonl",
        "2026-08-15",
        vec![
            says("user", "steady the beacon"),
            edits(beacon.to_str().unwrap()),
        ],
    );

    let brief = injected(&bench.session_start(SESSION, "resume"), "SessionStart");
    assert!(
        brief.contains("beacon"),
        "the brief did not quote the edit, so nothing is being suppressed: {brief}"
    );
    let quoted = ids(&bench.state(SESSION), "brief");
    assert!(!quoted.is_empty(), "the brief recorded no turns");

    let asked = "what changed in crates/gizmo/beacon.rs";
    silent(
        &bench.prompt(SESSION, &bench.work.join(ABSENT), asked),
        "a prompt whose only candidate is a turn the brief quoted",
    );

    let state = bench.state(SESSION);
    let refused = suppressed(&state);
    assert!(
        refused
            .iter()
            .any(|(_, reason)| reason == "carried_by_brief"),
        "no turn was refused for having been in the brief: {state}"
    );
    for (turn_id, _) in &refused {
        assert!(quoted.contains(turn_id), "{turn_id} was not in the brief");
    }

    // The control: a session that never saw the brief is answered.
    let control = injected(
        &bench.prompt(
            "cccccccc-1111-4000-8000-000000000002",
            &bench.work.join(ABSENT),
            asked,
        ),
        "UserPromptSubmit",
    );
    assert!(
        control.contains("beaconWaver"),
        "the prompt does not reach the edit at all: {control}"
    );
}
