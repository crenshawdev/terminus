//! AC1 at the process boundary: `verbatim observations` lists what a finalized
//! session did, with no model and no network on any path (OBS-01).
//!
//! Spawns rather than library calls. `verbatim-core`'s `tests/observe.rs`
//! asserts what the facts ARE; what is asserted here is that a real binary,
//! given a real store, writes them inside the `--json` envelope with the exit
//! code the CLI contract promises - and that the two ways this command can have
//! nothing to say (a machine that never ingested, a store older than the table)
//! are answers rather than a SQLite line escaping the envelope.
//!
//! The harness is a deliberate copy of `tests/cli.rs`'s, for the reason
//! `tests/decisions.rs` gives about its own: a shared `mod common` would be a
//! third file to keep in step, and these differ in what they assert.

use std::path::PathBuf;
use std::process::{Command, Output};

use rusqlite::Connection;
use serde_json::Value;
use verbatim_core::store::DB_FILE_NAME;

/// The session the transcript below belongs to, and the branch it ran on.
const SESSION: &str = "11111111-1111-4111-8111-111111111111";
const BRANCH: &str = "phase-7";

/// The command whose arguments are the whole of D-04: `entities` holds `cargo`,
/// so a listing carrying this string can only have come off the blob.
const COMMAND: &str = "cargo test -p verbatim-core --features testkit";

/// The commit subject, which `parse::Record` has no field for at all.
const SUBJECT: &str = "wire the observation step up";

/// The failure the session hit, as the transcript's own `tool_result` spells it.
const ERROR: &str = "Bash command failed";

/// How long ago the session's first turn was. Older than
/// `feedback::finalize::IDLE_HOURS`, so the pass that archives it also closes
/// it, which is the gate an observation is written behind.
const FIRST_TURN_SECONDS_AGO: i64 = 36_000;

/// Wall-clock seconds between the session's first turn and its last.
const DURATION_SECONDS: i64 = 100;

/// A second session, days older than the first, for the `--since` selector to
/// leave alone.
const OLDER_SESSION: &str = "22222222-2222-4222-8222-222222222222";
const OLDER_FIRST_TURN_SECONDS_AGO: i64 = 200_000;

/// The bound between the two sessions' last turns, in seconds ago.
const BETWEEN_SECONDS_AGO: i64 = 100_000;

/// Stored facts no recompute could produce, so a row that was rebuilt and a row
/// that was left alone are distinguishable byte for byte.
const SPOILED: &str = r#"{"this":"was hand-edited and must not survive a rebuild"}"#;

/// Every directory a spawned `verbatim` may touch, all of them temporary.
///
/// The config directory and the Claude directory are as load-bearing as the
/// data directory: a spawn that set only `VERBATIM_DATA_DIR` would resolve the
/// developer's real config and walk the live `~/.claude` tree. No test process
/// may resolve a real transcript root.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    /// Holds no `verbatim.toml`, so the loader yields defaults.
    config_dir: PathBuf,
    /// `<claude_dir>/projects` is the only tree a spawn can reach.
    claude_dir: PathBuf,
    /// Where the transcript's `cwd` points: somewhere this test owns.
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::create_dir_all(root.join("project-alpha")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        config_dir,
        claude_dir,
        root,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_verbatim"))
            .args(args)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn verbatim")
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// Put one transcript into the tree a pass walks, with a chosen clock.
    fn place(&self, session: &str, first_seconds_ago: i64) {
        let dir = self.claude_dir.join("projects").join("project-alpha");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{session}.jsonl")),
            self.transcript(session, first_seconds_ago),
        )
        .unwrap();
    }

    /// Archive whatever is in the tree through the binary's own bare `ingest`.
    ///
    /// Bare and not `ingest <path>`: the pass is what runs `feedback::outcomes`
    /// and therefore what sets `session_meta.is_final`, which is the gate the
    /// observation step reads its sessions through.
    fn ingest(&self) {
        let out = self.run(&["ingest"]);
        assert!(
            out.status.success(),
            "the tree pass failed: {}",
            stderr(&out)
        );
    }

    /// The one session every fact assertion in this file is about.
    fn ingest_the_finalized_session(&self) {
        self.place(SESSION, FIRST_TURN_SECONDS_AGO);
        self.ingest();
    }

    /// The `mechanical` column of one session's row, as stored.
    fn stored(&self, session: &str) -> String {
        self.conn()
            .query_row(
                "SELECT o.mechanical FROM observations o WHERE o.session_id = ?1",
                [session],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("no observation for {session}: {e}"))
    }

    /// Overwrite one session's stored facts with something no recompute could
    /// produce, so "this row was rebuilt" and "this row was left alone" are
    /// distinguishable afterwards.
    fn spoil(&self, session: &str) {
        let changed = self
            .conn()
            .execute(
                "UPDATE observations SET mechanical = ?2 WHERE session_id = ?1",
                rusqlite::params![session, SPOILED],
            )
            .unwrap();
        assert_eq!(changed, 1, "the premise: {session} has a row to spoil");
    }

    /// The file the session's `Edit` touched, spelled the way it is stored.
    fn edited(&self) -> String {
        self.root
            .join("project-alpha/crates/gizmo/lantern.rs")
            .to_string_lossy()
            .into_owned()
    }

    /// One session carrying every OBS-01 fact: an edit, two tools, a command
    /// with arguments, a failure, a branch, a commit, six turns and a duration.
    fn transcript(&self, session: &str, first_seconds_ago: i64) -> String {
        let cwd = self
            .root
            .join("project-alpha")
            .to_string_lossy()
            .into_owned();
        let mut body = String::new();
        let mut push = |seconds_ago: i64, n: usize, content: Value, kind: &str, extra: Value| {
            let mut record = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": cwd.clone(),
                "sessionId": session,
                "gitBranch": BRANCH,
                "type": kind,
                "uuid": format!("{session}-{n}"),
                "timestamp": ts(seconds_ago),
                "message": {"role": if kind == "user" { "user" } else { "assistant" },
                            "model": "claude-opus-5",
                            "content": content},
            });
            if let Value::Object(fields) = extra {
                for (key, value) in fields {
                    record.as_object_mut().unwrap().insert(key, value);
                }
            }
            body.push_str(&record.to_string());
            body.push('\n');
        };
        let first = first_seconds_ago;

        push(
            first,
            0,
            serde_json::json!([{"type": "text", "text": "swap the flicker helper for the steady one"}]),
            "user",
            Value::Null,
        );
        push(
            first - 1,
            1,
            serde_json::json!([{
                "type": "tool_use", "id": "toolu_1", "name": "Edit",
                "input": {"file_path": self.edited(),
                          "old_string": "lanternFlicker(seed)",
                          "new_string": "lanternSteady(seed)"},
            }]),
            "assistant",
            Value::Null,
        );
        push(
            first - 2,
            2,
            serde_json::json!([{
                "type": "tool_use", "id": "toolu_2", "name": "Bash",
                "input": {"command": COMMAND},
            }]),
            "assistant",
            Value::Null,
        );
        push(
            first - 3,
            3,
            serde_json::json!([{"tool_use_id": "toolu_2", "type": "tool_result",
                                "content": ERROR, "is_error": true}]),
            "user",
            serde_json::json!({"toolUseResult": {"stdout": "", "stderr": ERROR}}),
        );
        push(
            first - 4,
            4,
            serde_json::json!([{
                "type": "tool_use", "id": "toolu_3", "name": "Bash",
                "input": {"command": format!("git commit -m \"{SUBJECT}\"")},
            }]),
            "assistant",
            Value::Null,
        );
        push(
            first - DURATION_SECONDS,
            5,
            serde_json::json!([{"tool_use_id": "toolu_3", "type": "tool_result",
                                "content": "1 file changed", "is_error": false}]),
            "user",
            Value::Null,
        );
        body
    }
}

/// A UTC timestamp `seconds_ago` from now, in the shape a transcript writes.
///
/// SQLite computes it because SQLite is what the idle rule compares against: a
/// formatter written here could differ from the one under test in exactly the
/// way that would make the comparison pass for the wrong reason.
fn ts(seconds_ago: i64) -> String {
    Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)",
            [format!("-{seconds_ago} seconds")],
            |r| r.get(0),
        )
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one JSON document a `--json` run wrote to stdout.
fn document(output: &Output) -> Value {
    let text = stdout(output);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_else(|| panic!("stdout was empty"));
    assert_eq!(
        lines.next(),
        None,
        "stdout carried more than the document: {text:?}"
    );
    serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {text:?}"))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("a fact list is an array")
        .iter()
        .map(|item| item.as_str().unwrap_or_default().to_owned())
        .collect()
}

/// AC1: every fact the requirement names, through the envelope, exit 0.
#[test]
fn a_finalized_session_is_listed_with_every_obs_01_fact() {
    let bench = bench();
    bench.ingest_the_finalized_session();

    let out = bench.run(&["observations", "--project", "*", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let value = document(&out);
    assert_eq!(value["command"], "observations");
    assert_eq!(value["ok"], true, "{value}");
    let listed = value["data"]["observations"]
        .as_array()
        .unwrap_or_else(|| panic!("no observations array: {value}"));
    assert_eq!(listed.len(), 1, "{value}");

    let row = &listed[0];
    assert_eq!(row["session_id"], SESSION, "{row}");
    assert!(row["generated_at"].is_string(), "{row}");
    // The judgment half is present and null, so a consumer reads one shape
    // whether or not a provider has ever answered.
    for column in [
        "status",
        "model",
        "prompt_version",
        "topic",
        "outcome",
        "decisions",
        "learned",
        "unresolved",
        "raw",
        "tokens",
    ] {
        assert_eq!(row[column], Value::Null, "{column} is not null yet: {row}");
    }

    let facts = &row["mechanical"];
    assert!(
        facts.is_object(),
        "the stored document came back as a string: {row}"
    );
    assert!(
        strings(&facts["files_modified"]).contains(&bench.edited()),
        "the edited file is not on the modified list: {facts}"
    );
    let tools = strings(&facts["tools"]);
    assert!(tools.contains(&"Edit".to_owned()), "{facts}");
    assert!(tools.contains(&"Bash".to_owned()), "{facts}");
    assert!(
        strings(&facts["commands"]).contains(&COMMAND.to_owned()),
        "the command lost its arguments: {facts}"
    );
    assert!(
        strings(&facts["errors"]).iter().any(|e| e.contains(ERROR)),
        "no error came back: {facts}"
    );
    assert_eq!(
        strings(&facts["commits"]),
        vec![SUBJECT.to_owned()],
        "{facts}"
    );
    assert_eq!(facts["branch"], BRANCH, "{facts}");
    assert_eq!(facts["turns"], 6, "{facts}");
    assert_eq!(facts["duration_seconds"], DURATION_SECONDS, "{facts}");
    assert_eq!(facts["compactions"], 0, "{facts}");

    // And the same facts a person sees, on stdout, without the flag.
    let human = bench.run(&["observations", "--project", "*"]);
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    let text = stdout(&human);
    for expected in [BRANCH, COMMAND, SUBJECT, ERROR, "6 turn(s)", "1m 40s"] {
        assert!(
            text.contains(expected),
            "the human listing never names {expected:?}: {text}"
        );
    }
    assert!(text.contains(&bench.edited()), "{text}");
}

/// A machine that has never ingested: an empty answer with a reason, exit 0,
/// and not one byte created where the store would go.
#[test]
fn an_empty_data_directory_is_a_reason_and_creates_nothing() {
    let bench = bench();

    let out = bench.run(&["observations", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let value = document(&out);
    assert_eq!(value["command"], "observations");
    assert_eq!(value["ok"], true, "{value}");
    assert!(
        value["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no verbatim store")),
        "{value}"
    );
    assert_eq!(
        value["data"]["observations"],
        serde_json::json!([]),
        "{value}"
    );

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory"
    );
}

/// A store written before this build carries no `observations` table. That is a
/// reason naming the table and exit 0 - never `no such table: observations`
/// escaping the envelope inside an operational failure that never learned
/// `--json` was asked for.
#[test]
fn a_store_older_than_the_table_names_it_rather_than_naming_sqlite() {
    let bench = bench();
    bench.ingest_the_finalized_session();
    bench
        .conn()
        .execute("DROP TABLE observations", [])
        .expect("the premise: the table was there to drop");

    let out = bench.run(&["observations", "--project", "*", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    let reason = value["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("observations"),
        "the reason does not name the table: {value}"
    );
    assert_eq!(
        value["data"]["observations"],
        serde_json::json!([]),
        "{value}"
    );
    assert!(
        !stdout(&out).contains("no such table"),
        "a SQLite message reached stdout: {}",
        stdout(&out)
    );
}

/// AC7: `--since` rebuilds the rows after the bound and leaves every earlier
/// row byte for byte what it was.
///
/// Both rows are spoiled first, so "the earlier row still holds the hand-edited
/// bytes" is a clause that can fail: a regenerate that ignored its selector
/// would rewrite both and this would catch it, and one that selected nothing
/// would leave both spoiled and the first assertion would catch that.
#[test]
fn regenerate_rebuilds_the_selected_rows_and_leaves_the_rest_untouched() {
    let bench = bench();
    bench.place(SESSION, FIRST_TURN_SECONDS_AGO);
    bench.place(OLDER_SESSION, OLDER_FIRST_TURN_SECONDS_AGO);
    bench.ingest();

    bench.spoil(SESSION);
    bench.spoil(OLDER_SESSION);

    let since = ts(BETWEEN_SECONDS_AGO);
    let out = bench.run(&["observations", "regenerate", "--since", &since, "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let value = document(&out);
    assert_eq!(value["command"], "observations regenerate", "{value}");
    assert_eq!(value["ok"], true, "{value}");
    let data = &value["data"];
    assert_eq!(data["since"], since.as_str(), "{data}");
    assert_eq!(data["prompt_version"], Value::Null, "{data}");
    assert_eq!(data["selected"], 1, "{data}");
    assert_eq!(data["regenerated"], 1, "{data}");
    assert_eq!(data["notes"], serde_json::json!([]), "{data}");

    let rebuilt: Value = serde_json::from_str(&bench.stored(SESSION)).unwrap();
    assert!(
        strings(&rebuilt["commands"]).contains(&COMMAND.to_owned()),
        "the selected row was not recomputed: {rebuilt}"
    );
    assert_eq!(
        bench.stored(OLDER_SESSION),
        SPOILED,
        "a row outside the selector was rewritten"
    );
}

/// The selector narrows conjunctively, and `--prompt-version` narrows to
/// nothing while the column is null - which is the honest answer, not an empty
/// selector that rebuilds everything.
#[test]
fn a_prompt_version_no_row_carries_selects_no_row() {
    let bench = bench();
    bench.ingest_the_finalized_session();
    bench.spoil(SESSION);

    let out = bench.run(&[
        "observations",
        "regenerate",
        "--prompt-version",
        "v1",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let data = &document(&out)["data"];
    assert_eq!(data["prompt_version"], "v1", "{data}");
    assert_eq!(data["selected"], 0, "{data}");
    assert_eq!(data["regenerated"], 0, "{data}");
    assert_eq!(
        bench.stored(SESSION),
        SPOILED,
        "a selector that matched nothing still rebuilt a row"
    );

    // And with no selector at all, that same row IS rebuilt: the premise of the
    // assertion above is that the row was reachable and simply not selected.
    let out = bench.run(&["observations", "regenerate", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(document(&out)["data"]["regenerated"], 1);
    assert_ne!(bench.stored(SESSION), SPOILED);
}

/// D-18's misuse arm: an unknown flag and an unknown second word both exit 2
/// with nothing on stdout.
///
/// The second word matters more than the flag. `dispatch` matches single-word
/// names, so a `observations` that did not consume its own second word would
/// answer `no-such-verb` with a full listing and exit 0.
#[test]
fn an_unknown_flag_and_an_unknown_verb_are_both_misuse() {
    let bench = bench();
    bench.ingest_the_finalized_session();

    for argv in [
        vec!["observations", "regenerate", "--definitely-not-a-flag"],
        vec!["observations", "no-such-verb"],
        vec!["observations", "no-such-verb", "--json"],
        vec!["observations", "--definitely-not-a-flag"],
        // The verb comes first or not at all, so this is misuse rather than one
        // of the two commands chosen silently.
        vec!["observations", "--json", "regenerate"],
    ] {
        let out = bench.run(&argv);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`{}` should be misuse: {}",
            argv.join(" "),
            stderr(&out)
        );
        assert_eq!(
            stdout(&out),
            "",
            "`{}` printed to stdout on misuse",
            argv.join(" ")
        );
    }
}

/// A malformed `--since` is misuse, not a plausible wrong set rebuilt in
/// silence: every string orders against every stored timestamp, so an
/// unvalidated bound would compare cleanly and report success.
#[test]
fn a_malformed_since_is_misuse_and_rebuilds_nothing() {
    let bench = bench();
    bench.ingest_the_finalized_session();
    bench.spoil(SESSION);

    let out = bench.run(&["observations", "regenerate", "--since", "last tuesday"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert_eq!(
        bench.stored(SESSION),
        SPOILED,
        "a refused bound still wrote"
    );
}
