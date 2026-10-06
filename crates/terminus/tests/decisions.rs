//! AC1 end to end: three prompt shapes through the real hook, and the rows they
//! leave behind after an ingest (FEED-01).
//!
//! Spawns rather than library calls. What is under test is the seam the product
//! actually runs - a hook process that writes a file on a thread it abandons,
//! and a separate ingest process that moves that file into SQLite (D-01) - and
//! neither half of that exists inside a library call. `terminus-core`'s
//! `tests/feedback.rs` asserts what a record CONTAINS; what is here is that a
//! real hook leaves one and a real pass drains it, while stdout stays the
//! one-object-or-nothing protocol.
//!
//! The harness is a deliberate copy of `tests/prompt.rs`'s, for the reason that
//! file gives: a shared `mod common` would be a third file to keep in step, and
//! these differ in what they assert.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::Value;
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

/// The fixture whose `Edit` stores an absolute path under `project-alpha`, and
/// the relative spelling of that same file.
const FIXTURE: &str = "session-edits.jsonl";
const RELATIVE: &str = "crates/gizmo/lantern.rs";

/// The three prompts AC1 names, in the order they are fed.
///
/// One injects; one names an identifier-shaped word the archive carries only in
/// prose, so the search answers and the threshold refuses what it found; and one
/// names nothing path-shaped or identifier-shaped at all, so it never opens the
/// store.
const INJECTS: &str = "what changed in crates/gizmo/lantern.rs";
const REFUSED: &str = "is widgetFactory still the one we settled on";
const UNASKED: &str = "how should we think about this in general";

/// The word the threshold-refused prompt asks about: identifier-shaped, and
/// carried by a turn that stored no entity at all.
const PROSE_WORD: &str = "widgetFactory";

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

    /// Archive the rooted fixture through the binary's own ingest path.
    fn ingest_fixture(&self) {
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

    /// Archive one session of prose in the same project.
    ///
    /// Prose stores no entity whatever it names (RCL-02), which is what makes a
    /// prompt asking about [`PROSE_WORD`] a search that finds a hit and a
    /// threshold that refuses it - the non-fire AC1 asks for, rather than a
    /// prompt that simply matched nothing.
    fn archive_prose(&self) {
        let cwd = testkit::fixture_project(FIXTURE, &self.root);
        let record = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "cwd": cwd.to_str().unwrap(),
            "sessionId": "77777777-7777-4777-8777-777777777777",
            "type": "assistant",
            "uuid": "77777777-0000-4000-8000-000000000001",
            "timestamp": "2026-08-14T10:00:00.000Z",
            "requestId": "req_1",
            "message": {
                "role": "assistant",
                "model": "claude-opus-5",
                "content": [{
                    "type": "text",
                    "text": format!("the {PROSE_WORD} helper is still the one we settled on"),
                }],
            },
        });
        let path = self.work.join("prose.jsonl");
        std::fs::write(&path, format!("{record}\n")).unwrap();
        let out = self
            .command(&["ingest", path.to_str().unwrap()])
            .output()
            .expect("spawn the ingest");
        assert!(
            out.status.success(),
            "ingest the prose session: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// One `UserPromptSubmit`: the payload on stdin, then EOF, the way Claude
    /// Code writes it.
    fn prompt(&self, prompt: &str) -> Output {
        let mut child = self
            .command(&["hook", "UserPromptSubmit"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the hook");
        let payload = serde_json::json!({
            "session_id": "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55",
            "transcript_path": "/home/user/.claude/projects/-p/0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55.jsonl",
            "cwd": testkit::fixture_project(FIXTURE, &self.root).to_str().unwrap(),
            "hook_event_name": "UserPromptSubmit",
            "prompt": prompt,
        });
        child
            .stdin
            .take()
            .expect("the hook's stdin")
            .write_all(format!("{payload}\n").as_bytes())
            .expect("write the payload");
        child.wait_with_output().expect("wait for the hook")
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).expect("open the store")
    }

    /// Run `terminus ingest` until the store holds `want` decisions.
    ///
    /// A loop rather than one call, because every hook spawns a detached ingest
    /// of its own (ING-02): one of those may hold the lock when this runs, and a
    /// pass that lost the lock race exits 0 having drained nothing. Waiting for
    /// the count is what makes the assertion about the drain rather than about
    /// which process got there first.
    fn drained(&self, want: i64) -> i64 {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let out = self.command(&["ingest"]).output().expect("spawn the pass");
            assert!(
                out.status.success(),
                "ingest: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let count: i64 = self
                .conn()
                .query_row("SELECT count(*) FROM decisions", [], |r| r.get(0))
                .unwrap();
            if count >= want || Instant::now() > deadline {
                return count;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The one row this prompt left, as the columns AC1 names.
    fn row(&self, prompt: &str) -> Row {
        self.conn()
            .query_row(
                "SELECT ts, cwd, watermark_session_no, chars_injected,
                        spellings, candidates, injected, suppressed, thresholds
                 FROM decisions WHERE prompt = ?1",
                [prompt],
                |r| {
                    Ok(Row {
                        ts: r.get(0)?,
                        cwd: r.get(1)?,
                        watermark: r.get(2)?,
                        chars_injected: r.get(3)?,
                        spellings: json(r.get::<_, String>(4)?),
                        candidates: json(r.get::<_, String>(5)?),
                        injected: json(r.get::<_, String>(6)?),
                        suppressed: json(r.get::<_, String>(7)?),
                        thresholds: json(r.get::<_, String>(8)?),
                    })
                },
            )
            .unwrap_or_else(|e| panic!("no decisions row for {prompt:?}: {e}"))
    }
}

struct Row {
    ts: String,
    cwd: String,
    watermark: Option<i64>,
    chars_injected: i64,
    spellings: Value,
    candidates: Value,
    injected: Value,
    suppressed: Value,
    thresholds: Value,
}

fn json(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

/// How many elements a JSON array column carries.
fn len(value: &Value) -> usize {
    value
        .as_array()
        .unwrap_or_else(|| panic!("not an array: {value}"))
        .len()
}

/// AC1: three prompts, three rows, and stdout stayed the protocol on all three.
#[test]
fn three_prompt_shapes_each_leave_one_decision_row() {
    let bench = bench();
    bench.ingest_fixture();
    bench.archive_prose();

    let injected = bench.prompt(INJECTS);
    let refused = bench.prompt(REFUSED);
    let unasked = bench.prompt(UNASKED);

    // The protocol first: one JSON object or nothing at all, and exit 0 either
    // way. A decision record that cost a prompt its answer would be the one
    // failure this whole design is arranged against.
    let context = one_object(&injected, "the prompt naming a stored path");
    assert!(
        context.contains("lanternFlicker"),
        "the injected turn is not the edit that stored the path: {context}"
    );
    silent(&refused, "the prompt refused by the threshold");
    silent(&unasked, "the prompt naming nothing structural");

    assert_eq!(
        bench.drained(3),
        3,
        "three prompts did not become three rows"
    );

    // The prompt that injected.
    let row = bench.row(INJECTS);
    assert_eq!(
        row.ts.len(),
        24,
        "not the archive's timestamp shape: {}",
        row.ts
    );
    assert_eq!(
        row.cwd,
        testkit::fixture_project(FIXTURE, &bench.root)
            .to_str()
            .unwrap()
    );
    assert!(row.watermark.is_some(), "the store was open; no watermark");
    assert_eq!(row.chars_injected as usize, context.chars().count());
    assert_eq!(len(&row.injected), 1, "{}", row.injected);
    assert!(row.injected[0]["chars"].as_i64().unwrap() > 0);
    assert!(
        row.spellings
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().ends_with(RELATIVE)),
        "{}",
        row.spellings
    );
    let matched = &row.candidates[0]["matched_on"][0];
    assert_eq!(matched["kind"], "path", "{}", row.candidates);
    assert!(matched["value"].as_str().unwrap().ends_with(RELATIVE));
    assert_eq!(len(&row.suppressed), 0, "{}", row.suppressed);
    // The values in force, logged per decision because they are compile-time
    // constants (D-08).
    assert_eq!(row.thresholds["max_turns"], 3, "{}", row.thresholds);
    assert_eq!(row.thresholds["entity_rank"], 3, "{}", row.thresholds);
    assert_eq!(row.thresholds["co_occurring"], 2, "{}", row.thresholds);
    assert_eq!(row.thresholds["prompt_chars"], 4000, "{}", row.thresholds);

    // The prompt the threshold refused: it reached the index, the index
    // answered, and nothing fired.
    let row = bench.row(REFUSED);
    assert!(row.watermark.is_some(), "the store was open; no watermark");
    assert_eq!(
        row.spellings[0], PROSE_WORD,
        "the prompt asked about something else: {}",
        row.spellings
    );
    assert_eq!(
        len(&row.candidates),
        1,
        "the search found nothing, so the threshold is not what refused it: {}",
        row.candidates
    );
    assert_eq!(row.candidates[0]["entity_count"], 0, "{}", row.candidates);
    assert_eq!(len(&row.injected), 0, "{}", row.injected);
    assert_eq!(row.chars_injected, 0);

    // The prompt that never opened the store: no spelling, no candidate, and a
    // watermark the drain stamped rather than one the prompt took.
    let row = bench.row(UNASKED);
    assert_eq!(len(&row.spellings), 0, "{}", row.spellings);
    assert_eq!(len(&row.candidates), 0, "{}", row.candidates);
    assert!(row.watermark.is_some(), "the drain stamped no watermark");
    assert_eq!(row.chars_injected, 0);

    // The files are gone, and a further pass adds nothing.
    assert_eq!(bench.drained(3), 3, "a second pass drained a record twice");
}

/// The one JSON object on a hook's stdout, and the context inside it.
fn one_object(out: &Output, what: &str) -> String {
    assert!(
        out.status.success(),
        "{what} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("{what} wrote nothing on stdout"));
    assert_eq!(
        lines.next(),
        None,
        "{what} wrote more than one object: {text:?}"
    );

    let document: Value = serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("{what} did not write one JSON object ({e}): {text}"));
    document["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: no additionalContext: {document}"))
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
