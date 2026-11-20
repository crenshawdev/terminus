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
