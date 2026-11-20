//! The summary ingest keeps: its tables equal what the committed rows give, on
//! every path ingest has and however a transcript is split across passes.
//!
//! `#![cfg(feature = "testkit")]` gates the file, so run it with
//! `--all-features` and read the test count in the output.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::types::Value as Cell;
use rusqlite::Connection;
use serde_json::{json, Value};
use terminus_core::config::Config;
use terminus_core::ingest::{self, backfill, pass, Outcome};
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

/// Workers for the backfill: more than one, so it is not the sequential pass.
const WORKERS: usize = 3;

/// A scratch world: a claude directory transcripts live under, and a root the
/// transcripts' `cwd` values sit beneath that nothing ever creates.
struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
    claude: PathBuf,
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir_in(testkit::temp_root()).unwrap();
        let root = dir.path().join("root");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        World {
            _dir: dir,
            root,
            claude,
        }
    }

    fn cwd(&self, relative: &str) -> String {
        self.root.join(relative).to_string_lossy().into_owned()
    }

    fn config(&self) -> Config {
        Config::from_parts(vec![self.claude.clone()], Vec::new())
    }

    fn data(&self, name: &str) -> PathBuf {
        self.claude.parent().unwrap().join(name)
    }

    /// `<claude>/projects/<folder>/<id>.jsonl`.
    fn main_path(&self, folder: &str, id: &str) -> PathBuf {
        self.claude
            .join("projects")
            .join(folder)
            .join(format!("{id}.jsonl"))
    }

    /// `<claude>/projects/<folder>/<id>/subagents/agent-<n>.jsonl`.
    fn sidecar_path(&self, folder: &str, id: &str, n: usize) -> PathBuf {
        self.claude
            .join("projects")
            .join(folder)
            .join(id)
            .join("subagents")
            .join(format!("agent-{n}.jsonl"))
    }

    /// Every transcript fixture, placed the way `backfill.rs`'s tree places them.
    fn place_fixtures(&self) {
        let project = self.claude.join("projects").join("-tmp-project-0");
        for (index, fixture) in testkit::TRANSCRIPT_FIXTURES.iter().enumerate() {
            let dest = if fixture.contains('/') {
                project
                    .join(format!("{index:08x}-2222-4222-8222-222222222222"))
                    .join("subagents")
                    .join(format!("agent-{index}.jsonl"))
            } else {
                project.join(format!("{index:08x}-1111-4111-8111-111111111111.jsonl"))
            };
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        }
    }

    /// The same store three ways: the tree pass, the backfill, and each file
    /// named on its own. Returns the three data directories in that order.
    fn archive_three_ways(&self) -> [PathBuf; 3] {
        let tree = self.data("tree");
        let filled = self.data("backfill");
        let named = self.data("named");
        match pass::run_with(&tree, &self.config()).unwrap() {
            pass::PassOutcome::Ran(_) => {}
            pass::PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
        let report = backfill::run_with(&filled, &self.config(), WORKERS).unwrap();
        assert!(matches!(report.outcome, pass::PassOutcome::Ran(_)));
        for path in terminus_core::discover::discover(&self.config()).transcripts {
            committed(ingest::run_with(&named, &path, &self.config()).unwrap());
        }
        [tree, filled, named]
    }
}

fn committed(outcome: Outcome) {
    assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");
}

fn open(data_dir: &Path) -> Connection {
    Connection::open(data_dir.join(DB_FILE_NAME)).unwrap()
}

/// One transcript line. A `None` cwd leaves the key out of the record.
fn line(cwd: Option<&str>, session: &str, n: usize, kind: &str, content: Value) -> String {
    let mut record = json!({
        "parentUuid": null,
        "isSidechain": false,
        "sessionId": session,
        "uuid": format!("{session}-{n}"),
        "timestamp": format!("2026-02-03T10:00:{n:02}.000Z"),
        "type": kind,
        "message": {"role": kind, "content": content},
    });
    if let Some(cwd) = cwd {
        record["cwd"] = json!(cwd);
    }
    format!("{record}\n")
}

/// A typed prompt and a reply, alternating, `turns` records long.
fn talk(cwd: Option<&str>, session: &str, turns: usize) -> String {
    (0..turns)
        .map(|n| {
            if n % 2 == 0 {
                line(
                    cwd,
                    session,
                    n,
                    "user",
                    json!([{"type": "text", "text": "ask"}]),
                )
            } else {
                line(
                    cwd,
                    session,
                    n,
                    "assistant",
                    json!([{"type": "text", "text": "say"}]),
                )
            }
        })
        .collect()
}

/// One transcript line from a base of `sessionId`, `uuid` and `timestamp`, with
/// `fields` laid over it. A field set to null is left out of the line, which is
/// how a record loses its `uuid` or its `timestamp`.
fn rec(session: &str, n: usize, fields: Value) -> String {
    let mut record = json!({
        "sessionId": session,
        "uuid": format!("{session}-{n}"),
        "timestamp": format!("2026-02-03T10:00:{n:02}.000Z"),
    });
    for (key, value) in fields.as_object().unwrap() {
        record[key] = value.clone();
    }
    record.as_object_mut().unwrap().retain(|_, v| !v.is_null());
    format!("{record}\n")
}

/// `base` with every field of `extra` laid over it.
fn with(mut base: Value, extra: Value) -> Value {
    for (key, value) in extra.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

fn said(kind: &str, text: &str) -> Value {
    json!({"type": kind, "message": {"role": kind, "content": [{"type": "text", "text": text}]}})
}

/// The one summary row whose key names `id`.
fn session_row(conn: &Connection, id: &str) -> Vec<Cell> {
    let mut statement = conn
        .prepare("SELECT * FROM session_summary WHERE session_key LIKE ?1")
        .unwrap();
    let width = statement.column_count();
    let mut rows: Vec<Vec<Cell>> = statement
        .query_map([format!("%{id}%")], |r| {
            (0..width).map(|i| r.get::<_, Cell>(i)).collect()
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 1, "rows for {id}: {rows:?}");
    rows.pop().unwrap()
}

fn column(table: &str, name: &str, conn: &Connection) -> usize {
    let statement = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
    let position = statement.column_names().position(|c| c == name);
    position.unwrap_or_else(|| panic!("{table} has no column {name}"))
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Every column of every row of the two summary tables, ordered by key.
fn summary_rows(conn: &Connection) -> Vec<Vec<Vec<Cell>>> {
    ["project_summary", "session_summary"]
        .iter()
        .map(|table| {
            let mut statement = conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let width = statement.column_count();
            statement
                .query_map([], |r| (0..width).map(|i| r.get::<_, Cell>(i)).collect())
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
        .collect()
}

fn project_row(conn: &Connection, project: Option<&str>, pre: Option<&str>) -> Option<(i64, i64)> {
    conn.query_row(
        "SELECT session_count, turn_count FROM project_summary
         WHERE project IS ?1 AND project_pre_worktree IS ?2",
        rusqlite::params![project, pre],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

#[test]
fn the_summary_reconciles_on_every_path_ingest_has() {
    let world = World::new();
    world.place_fixtures();

    let folder = "-tmp-written";
    let repo = world.cwd("repo");
    let worktree = world.cwd("repo/.claude/worktrees/wt");
    // Two main sessions that never name a `cwd`: three turns between them.
    write(
        &world.main_path(folder, "aaaaaaaa-0000-4000-8000-000000000001"),
        &talk(None, "a1", 2),
    );
    write(
        &world.main_path(folder, "aaaaaaaa-0000-4000-8000-000000000002"),
        &talk(None, "a2", 1),
    );
    // A main session of three turns and its sidecar of two, on one `cwd`.
    let main = "bbbbbbbb-0000-4000-8000-000000000001";
    write(&world.main_path(folder, main), &talk(Some(&repo), "b1", 3));
    write(
        &world.sidecar_path(folder, main, 1),
        &talk(Some(&repo), "b1", 2),
    );
    // A worktree of that repository: its own pair.
    write(
        &world.main_path(folder, "cccccccc-0000-4000-8000-000000000001"),
        &talk(Some(&worktree), "c1", 2),
    );

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap_or_else(|e| panic!("{}: {e}", data.display()));

        let nulls: i64 = conn
            .query_row(
                "SELECT count(*) FROM project_summary
                 WHERE project IS NULL AND project_pre_worktree IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nulls, 1, "{}", data.display());
        assert_eq!(project_row(&conn, None, None), Some((2, 3)));
        assert_eq!(project_row(&conn, Some(&repo), None), Some((2, 5)));
        assert_eq!(
            project_row(&conn, Some(&repo), Some(&worktree)),
            Some((1, 2))
        );
    }
}

/// Archive `prefix`, then `whole` over it, at one path in one store; and `whole`
/// once in another. Returns the two connections, two-pass first.
fn two_passes_and_one(world: &World, prefix: &str, whole: &str) -> (Connection, Connection) {
    let path = world.main_path("-tmp-split", "dddddddd-0000-4000-8000-000000000001");
    assert!(whole.starts_with(prefix) && prefix.ends_with('\n'));
    let (two, one) = (world.data("two"), world.data("one"));
    write(&path, prefix);
    committed(ingest::run_with(&two, &path, &world.config()).unwrap());
    write(&path, whole);
    committed(ingest::run_with(&two, &path, &world.config()).unwrap());
    committed(ingest::run_with(&one, &path, &world.config()).unwrap());
    (open(&two), open(&one))
}

#[test]
fn a_project_learnt_on_the_second_pass_moves_the_whole_count_off_the_null_pair() {
    let world = World::new();
    let late = world.cwd("late");
    let prefix = talk(None, "d1", 2);
    let whole = format!(
        "{prefix}{}",
        talk(Some(&late), "d1", 5)
            .split_inclusive('\n')
            .skip(2)
            .collect::<String>()
    );
    let (two, one) = two_passes_and_one(&world, &prefix, &whole);

    testkit::check_summary(&two).unwrap();
    testkit::check_summary(&one).unwrap();
    assert_eq!(summary_rows(&two), summary_rows(&one));
    assert_eq!(project_row(&two, None, None), None, "a stale null pair");
    assert_eq!(project_row(&two, Some(&late), None), Some((1, 5)));
}

#[test]
fn a_tail_that_opens_in_a_worktree_leaves_no_row_for_the_repository_alone() {
    let world = World::new();
    let repo = world.cwd("repo");
    let worktree = world.cwd("repo/.claude/worktrees/wt");
    let prefix = talk(Some(&repo), "d1", 2);
    let tail: String = talk(Some(&worktree), "d1", 5)
        .split_inclusive('\n')
        .skip(2)
        .collect();
    let (two, one) = two_passes_and_one(&world, &prefix, &format!("{prefix}{tail}"));

    testkit::check_summary(&two).unwrap();
    testkit::check_summary(&one).unwrap();
    assert_eq!(project_row(&two, Some(&repo), None), None, "a stale pair");
    assert_eq!(
        project_row(&two, Some(&repo), Some(&worktree)),
        Some((1, 5))
    );
    // `session_meta` itself differs between the two stores here: the one pass
    // saw the repository first and never the worktree's key.
    assert_eq!(project_row(&one, Some(&repo), None), Some((1, 5)));
}

const FOLDER: &str = "-tmp-session-rows";

#[test]
fn a_session_with_no_turn_still_has_a_row_holding_nothing() {
    let world = World::new();
    let id = "eeeeeeee-0000-4000-8000-000000000001";
    // Every record lacks a `uuid` or a `timestamp`, so none is a turn.
    let text = [
        rec(
            "e1",
            0,
            json!({"type": "user", "uuid": null, "message": {"content": "hi"}}),
        ),
        rec("e1", 1, json!({"type": "assistant", "timestamp": null})),
    ]
    .concat();
    write(&world.main_path(FOLDER, id), &text);

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();
        let row = session_row(&conn, id);
        assert_eq!(
            row[column("session_summary", "turn_count", &conn)],
            Cell::Integer(0)
        );
        assert_eq!(
            row[column("session_summary", "has_typed_prompt", &conn)],
            Cell::Integer(0)
        );
        for name in ["title", "branch", "last_turn_at"] {
            assert_eq!(
                row[column("session_summary", name, &conn)],
                Cell::Null,
                "{name}"
            );
        }
    }
}

#[test]
fn a_sidecar_has_no_row_and_leaves_its_parents_as_it_was() {
    let world = World::new();
    let id = "ffffffff-0000-4000-8000-000000000001";
    let cwd = world.cwd("repo");
    let data = world.data("store");
    let main = world.main_path(FOLDER, id);
    write(&main, &talk(Some(&cwd), "f1", 3));
    committed(ingest::run_with(&data, &main, &world.config()).unwrap());
    let before = session_row(&open(&data), id);

    let sidecar = world.sidecar_path(FOLDER, id, 1);
    write(&sidecar, &talk(Some(&world.cwd("elsewhere")), "f1", 4));
    committed(ingest::run_with(&data, &sidecar, &world.config()).unwrap());

    let conn = open(&data);
    testkit::check_summary(&conn).unwrap();
    assert_eq!(session_row(&conn, id), before);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM session_summary", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "the sidecar got a row");
}

#[test]
fn only_a_prompt_a_person_typed_turns_the_typed_flag_on() {
    let world = World::new();
    let scripted = [rec(
        "g1",
        0,
        json!({"type": "user", "entrypoint": "sdk-cli", "message": {"content": "ask"}}),
    )]
    .concat();
    let tool_result = [rec(
        "g2",
        0,
        json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "t", "content": "out"}]}}),
    )]
    .concat();
    let typed = [rec("g3", 0, said("user", "please do the thing"))].concat();
    let ids = [
        ("11111111-0000-4000-8000-000000000001", scripted, 0),
        ("11111111-0000-4000-8000-000000000002", tool_result, 0),
        ("11111111-0000-4000-8000-000000000003", typed, 1),
    ];
    for (id, text, _) in &ids {
        write(&world.main_path(FOLDER, id), text);
    }

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();
        for (id, _, flag) in &ids {
            let at = column("session_summary", "has_typed_prompt", &conn);
            assert_eq!(session_row(&conn, id)[at], Cell::Integer(*flag), "{id}");
        }
    }
}

#[test]
fn a_long_title_holding_a_key_is_stored_byte_for_byte() {
    let world = World::new();
    let id = "22222222-0000-4000-8000-000000000001";
    let title = format!(
        "{} sk-FAKEFAKEFAKEFAKEFAKEFAKEFAKE0000 {}",
        "start".repeat(10),
        "end".repeat(10)
    );
    assert!(title.chars().count() > 80);
    let text = [
        rec("h1", 0, said("user", "ask")),
        rec(
            "h1",
            1,
            json!({"type": "custom-title", "customTitle": title, "uuid": null, "timestamp": null}),
        ),
    ]
    .concat();
    write(&world.main_path(FOLDER, id), &text);

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();
        let at = column("session_summary", "title", &conn);
        assert_eq!(session_row(&conn, id)[at], Cell::Text(title.clone()));
    }
}

#[test]
fn no_column_of_either_table_holds_prompt_or_reply_text() {
    const MARKER: &str = "zorblax-marker-7731";
    let world = World::new();
    let id = "33333333-0000-4000-8000-000000000001";
    let cwd = world.cwd("repo");
    let text = [
        rec(
            "i1",
            0,
            with(said("user", &format!("{MARKER} first ask")), json!({"cwd": cwd, "gitBranch": "feature-x"})),
        ),
        rec("i1", 1, said("assistant", &format!("{MARKER} first answer"))),
        rec("i1", 2, said("user", &format!("{MARKER} second ask"))),
        rec("i1", 3, said("assistant", &format!("{MARKER} second answer"))),
        rec("i1", 4, json!({"type": "ai-title", "aiTitle": "A plain title", "uuid": null, "timestamp": null})),
    ]
    .concat();
    write(&world.main_path(FOLDER, id), &text);

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();

        // The row exists and says what the stored rows say, so an empty table
        // cannot pass the absence below.
        let row = session_row(&conn, id);
        let at = |name| row[column("session_summary", name, &conn)].clone();
        assert_eq!(at("title"), Cell::Text("A plain title".into()));
        assert_eq!(at("branch"), Cell::Text("feature-x".into()));
        assert_eq!(
            at("last_turn_at"),
            Cell::Text("2026-02-03T10:00:03.000Z".into())
        );
        assert_eq!(at("has_typed_prompt"), Cell::Integer(1));
        assert_eq!(at("turn_count"), Cell::Integer(4));

        for rows in summary_rows(&conn) {
            for cell in rows.into_iter().flatten() {
                let held = match cell {
                    Cell::Text(text) => text.into_bytes(),
                    Cell::Blob(bytes) => bytes,
                    _ => continue,
                };
                assert!(
                    !held.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
                    "a summary column holds prompt or reply text"
                );
            }
        }
    }
}

/// `(id, stream_offset, byte_len)` of the turn at `turn_seq`, off its `turns` row.
fn turn_locator(conn: &Connection, id: &str, turn_seq: i64) -> Value {
    let (turn, offset, len): (i64, i64, i64) = conn
        .query_row(
            "SELECT id, stream_offset, byte_len FROM turns
             WHERE session_key LIKE ?1 AND turn_seq = ?2",
            rusqlite::params![format!("%{id}%"), turn_seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    json!([turn, offset, len])
}

fn text_of(row: &[Cell], conn: &Connection, name: &str) -> String {
    match &row[column("session_summary", name, conn)] {
        Cell::Text(text) => text.clone(),
        other => panic!("{name} is {other:?}"),
    }
}

#[test]
fn the_row_locates_the_newest_twelve_typed_prompts_and_a_reply_that_is_only_a_tool_call() {
    let world = World::new();
    let id = "44444444-0000-4000-8000-000000000001";
    let mut text = String::new();
    let mut n = 0;
    // Fourteen typed prompts, each followed by a tool result (a `user` record
    // nobody typed) and an assistant answer: turn sequences 0, 3, 6 ... 39.
    for i in 0..14 {
        text += &rec("j1", n, said("user", &format!("typed prompt {i}")));
        text += &rec(
            "j1",
            n + 1,
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t", "content": "out"}]}}),
        );
        text += &rec("j1", n + 2, said("assistant", &format!("answer {i}")));
        n += 3;
    }
    // The session ends on an assistant turn holding only a tool call.
    text += &rec(
        "j1",
        n,
        json!({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "t9", "name": "Bash", "input": {"command": "ls"}}]}}),
    );
    write(&world.main_path(FOLDER, id), &text);

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();
        let row = session_row(&conn, id);
        // Prompts 13 down to 2, at turn sequences 39, 36 ... 6.
        let expected: Vec<Value> = (2..14)
            .rev()
            .map(|i| turn_locator(&conn, id, 3 * i))
            .collect();
        assert_eq!(expected.len(), 12);
        let held: Value = serde_json::from_str(&text_of(&row, &conn, "prompts")).unwrap();
        assert_eq!(held, Value::Array(expected));

        let last = turn_locator(&conn, id, 42);
        let at = |name| row[column("session_summary", name, &conn)].clone();
        assert_eq!(
            vec![
                at("reply_id"),
                at("reply_stream_offset"),
                at("reply_byte_len")
            ],
            last.as_array()
                .unwrap()
                .iter()
                .map(|v| Cell::Integer(v.as_i64().unwrap()))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn a_session_that_was_never_answered_locates_its_prompts_and_no_reply() {
    let world = World::new();
    let id = "55555555-0000-4000-8000-000000000001";
    let text = [
        rec("k1", 0, said("user", "first ask")),
        rec("k1", 1, said("user", "second ask")),
    ]
    .concat();
    write(&world.main_path(FOLDER, id), &text);

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();
        let row = session_row(&conn, id);
        let held: Value = serde_json::from_str(&text_of(&row, &conn, "prompts")).unwrap();
        assert_eq!(
            held,
            json!([turn_locator(&conn, id, 1), turn_locator(&conn, id, 0)])
        );
        for name in ["reply_id", "reply_stream_offset", "reply_byte_len"] {
            assert_eq!(
                row[column("session_summary", name, &conn)],
                Cell::Null,
                "{name}"
            );
        }
    }
}

/// An assistant record calling one writing tool on one path.
fn edit(cwd: &str, session: &str, n: usize, tool: &str, path: &str) -> String {
    rec(
        session,
        n,
        json!({"type": "assistant", "cwd": cwd, "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": format!("t{n}"), "name": tool, "input": {"file_path": path}}]}}),
    )
}

fn edited(row: &[Cell], conn: &Connection) -> Value {
    serde_json::from_str(&text_of(row, conn, "edited_files")).unwrap()
}

#[test]
fn the_row_keeps_every_in_project_edit_whole_and_most_recent_first() {
    let world = World::new();
    let id = "66666666-0000-4000-8000-000000000001";
    let cwd = world.cwd("repo");
    let long = format!("{cwd}/{}long.rs", "deep/".repeat(30));
    assert!(long.len() > 120);
    let mut text = rec("l1", 0, with(said("user", "go"), json!({"cwd": cwd})));
    let mut calls = vec![("Edit", format!("{cwd}/src/early.rs"))];
    for i in 1..=8 {
        let tool = if i % 2 == 1 { "Write" } else { "Edit" };
        calls.push((tool, format!("{cwd}/src/a{i}.rs")));
    }
    calls.push(("Write", long.clone()));
    calls.push(("Edit", "docs/notes.md".to_owned()));
    calls.push(("Write", world.cwd("scratch/tmp.txt")));
    calls.push(("Edit", format!("{cwd}/src/early.rs")));
    for (n, (tool, path)) in calls.iter().enumerate() {
        text += &edit(&cwd, "l1", n + 1, tool, path);
    }
    write(&world.main_path(FOLDER, id), &text);

    // Eleven in-project files, none cut, clipped or capped; the file edited
    // first and last leads, the file outside every root is absent.
    let mut expected = vec![json!("src/early.rs"), json!("docs/notes.md")];
    expected.push(json!(format!("{}long.rs", "deep/".repeat(30))));
    expected.extend((1..=8).rev().map(|i| json!(format!("src/a{i}.rs"))));

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();
        assert_eq!(
            edited(&session_row(&conn, id), &conn),
            Value::Array(expected.clone())
        );
    }
}

#[test]
fn a_fixtures_edit_is_kept_relative_to_its_project() {
    let world = World::new();
    let copied = testkit::copy_rooted_fixture_into(
        "session-edits.jsonl",
        &world.claude.parent().unwrap().join("rooted"),
        &world.root,
    );
    let id = "77777777-0000-4000-8000-000000000001";
    let dest = world.main_path(FOLDER, id);
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::copy(copied, &dest).unwrap();

    for data in world.archive_three_ways() {
        let conn = open(&data);
        testkit::check_summary(&conn).unwrap();
        assert_eq!(
            edited(&session_row(&conn, id), &conn),
            json!(["crates/gizmo/lantern.rs"])
        );
    }
}

#[test]
fn a_file_edited_in_the_prefix_and_again_in_the_tail_reads_the_same_as_one_pass() {
    let world = World::new();
    let cwd = world.cwd("repo");
    let prefix = [
        rec("m1", 0, with(said("user", "go"), json!({"cwd": cwd}))),
        edit(&cwd, "m1", 1, "Edit", &format!("{cwd}/src/a.rs")),
        edit(&cwd, "m1", 2, "Write", &format!("{cwd}/src/b.rs")),
    ]
    .concat();
    let whole = format!(
        "{prefix}{}{}",
        edit(&cwd, "m1", 3, "Edit", &format!("{cwd}/src/c.rs")),
        edit(&cwd, "m1", 4, "Edit", &format!("{cwd}/src/a.rs")),
    );
    let (two, one) = two_passes_and_one(&world, &prefix, &whole);

    testkit::check_summary(&two).unwrap();
    testkit::check_summary(&one).unwrap();
    assert_eq!(summary_rows(&two), summary_rows(&one));
    for conn in [&two, &one] {
        let held: String = conn
            .query_row("SELECT edited_files FROM session_summary", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, r#"["src/a.rs","src/c.rs","src/b.rs"]"#);
    }
}
