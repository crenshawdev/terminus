//! Session lineage through a real pass (ING-04, D-01, D-02, D-19).
//!
//! AC2 is the shape of this file: after a pass, the count of rows whose
//! `continues_from` equals their own session id must be zero while the count of
//! non-null `continues_from` values must not be.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::config::Config;
use verbatim_core::ingest::pass::{self, PassOutcome};
use verbatim_core::store::DB_FILE_NAME;

const PROJECT: &str = "-data-code-verbatim";

/// A Claude config directory whose `projects` tree the pass walks, plus a data
/// directory. No test process may resolve a real transcript root.
struct Tree {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
}

fn tree() -> Tree {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Tree {
        _dir: dir,
        data_dir,
        claude_dir,
    }
}

impl Tree {
    fn projects(&self) -> PathBuf {
        self.claude_dir.join("projects")
    }

    /// Write a transcript at `<projects>/<PROJECT>/<name>`, creating whatever
    /// directories the name implies.
    fn write(&self, name: &str, body: &str) -> PathBuf {
        let path = self.projects().join(PROJECT).join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path.canonicalize().unwrap()
    }

    /// A top-level transcript reporting `own` as its session id and, when
    /// `foreign` is set, that session as the one it continues from.
    fn transcript(&self, n: u8, own: &str, foreign: Option<&str>) -> PathBuf {
        self.write(&format!("{}.jsonl", uuid(n)), &record(n, own, foreign))
    }

    fn pass(&self) -> pass::Summary {
        let config = Config::from_parts(vec![self.claude_dir.clone()], Vec::new());
        match pass::run_with(&self.data_dir, &config).unwrap() {
            PassOutcome::Ran(summary) => summary,
            PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn continues_from(&self, path: &Path) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT continues_from FROM session_meta WHERE session_key = ?1",
                [path.to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
}

fn uuid(n: u8) -> String {
    format!("{n:08x}-1111-4111-8111-111111111111")
}

fn session(n: u8) -> String {
    format!("{n:08x}-2222-4222-8222-222222222222")
}

/// One turn record. `own` is `sessionId`, `foreign` is the snake_case
/// `session_id` - the file-level lineage signal, and a different field.
fn record(n: u8, own: &str, foreign: Option<&str>) -> String {
    let foreign = match foreign {
        Some(id) => format!("\"session_id\":\"{id}\","),
        None => String::new(),
    };
    format!(
        "{{\"type\":\"user\",\"uuid\":\"cccccccc-0000-4000-8000-0000000000{n:02x}\",\
          \"timestamp\":\"2026-08-12T10:00:{n:02}.000Z\",\"sessionId\":\"{own}\",{foreign}\
          \"cwd\":\"/data/code/verbatim\",\
          \"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hi\"}}]}}}}\n"
    )
}

/// D-01. 253 of the 433 real transcripts carrying a `session_id` carry only
/// their own, so an uncompared read makes one in five sessions continue from
/// itself.
#[test]
fn a_transcript_naming_only_its_own_session_id_continues_from_nothing() {
    let tree = tree();
    let own = session(1);
    let path = tree.transcript(1, &own, Some(&own));

    assert_eq!(tree.pass().files_committed, 1);
    assert_eq!(tree.continues_from(&path), None);
}

/// 11 real files carry two distinct ids. The foreign one is the link, and byte
/// order decides among survivors.
#[test]
fn a_transcript_carrying_both_ids_records_the_foreign_one() {
    let tree = tree();
    let own = session(1);
    let other = session(9);

    // Its own id first in byte order, the foreign one second: the rule is a
    // comparison, not a position.
    let path = tree.write(
        &format!("{}.jsonl", uuid(1)),
        &format!(
            "{}{}",
            record(1, &own, Some(&own)),
            record(2, &own, Some(&other))
        ),
    );

    assert_eq!(tree.pass().files_committed, 1);
    assert_eq!(tree.continues_from(&path), Some(other));
}

/// D-02: continuation is a fan-out. Session `ebd78c2b` is named as predecessor
/// by four separate later transcripts in one real project, so nothing may
/// assume a single successor.
#[test]
fn one_predecessor_named_by_three_successors_produces_three_links() {
    let tree = tree();
    let predecessor = session(1);
    tree.transcript(1, &predecessor, None);
    let successors: Vec<PathBuf> = (2..=4)
        .map(|n| tree.transcript(n, &session(n), Some(&predecessor)))
        .collect();

    assert_eq!(tree.pass().files_committed, 4);
    for path in &successors {
        assert_eq!(tree.continues_from(path).as_deref(), Some(&*predecessor));
    }

    let links: i64 = tree
        .conn()
        .query_row(
            "SELECT count(*) FROM session_meta WHERE continues_from = ?1",
            [&predecessor],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        links, 3,
        "a predecessor may be named by any number of files"
    );
}

/// D-19: 2 of the 173 real files naming a foreign session name one that was
/// aged out by `cleanupPeriodDays` and is not on disk at all. No foreign key,
/// no ingest-order requirement.
#[test]
fn a_predecessor_that_was_never_ingested_is_recorded_anyway() {
    let tree = tree();
    let missing = session(200);
    let path = tree.transcript(1, &session(1), Some(&missing));

    assert_eq!(tree.pass().files_committed, 1);
    assert_eq!(tree.continues_from(&path), Some(missing.clone()));

    let known: i64 = tree
        .conn()
        .query_row(
            "SELECT count(*) FROM session_meta WHERE session_id = ?1",
            [&missing],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(known, 0, "the link names a session the store does not hold");
}

/// AC2 over a tree: zero self-links, and more than zero links.
#[test]
fn over_a_tree_no_session_continues_from_itself_and_some_continue() {
    let tree = tree();
    let first = session(1);
    tree.transcript(1, &first, Some(&first)); // names only itself
    tree.transcript(2, &session(2), Some(&first)); // a real continuation
    tree.transcript(3, &session(3), None); // no signal at all
    tree.transcript(4, &session(4), Some(&first)); // the fan-out

    assert_eq!(tree.pass().files_committed, 4);

    let conn = tree.conn();
    let self_links: i64 = conn
        .query_row(
            "SELECT count(*) FROM session_meta WHERE continues_from = session_id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let links: i64 = conn
        .query_row(
            "SELECT count(*) FROM session_meta WHERE continues_from IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(self_links, 0, "no row may continue from its own session id");
    assert_eq!(links, 2);
}

/// The `parentUuid` fallback stays exactly what D-11 made it: message-level
/// threading, resolved through `turns` to the session holding that record, and
/// used only when no foreign `session_id` is present.
#[test]
fn the_parent_uuid_fallback_still_links_a_fork() {
    let tree = tree();
    let origin = session(1);
    tree.transcript(1, &origin, None);
    assert_eq!(tree.pass().files_committed, 1);

    let forked = tree.write(
        &format!("{}.jsonl", uuid(2)),
        &format!(
            "{{\"type\":\"user\",\"uuid\":\"dddddddd-0000-4000-8000-000000000001\",\
              \"parentUuid\":\"cccccccc-0000-4000-8000-000000000001\",\
              \"timestamp\":\"2026-08-12T11:00:00.000Z\",\"sessionId\":\"{}\"}}\n",
            session(2)
        ),
    );

    assert_eq!(tree.pass().files_committed, 1);
    assert_eq!(
        tree.continues_from(&forked),
        Some(origin),
        "the fallback must name a session, not a message uuid"
    );
}
