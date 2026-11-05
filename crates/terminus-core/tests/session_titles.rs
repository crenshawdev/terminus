//! Which title a session keeps, through the ordinary ingest path and a rebuild.
//!
//! The title records here are the shape real transcripts carry: `type`,
//! `sessionId` and the title, with no `uuid` and no `timestamp`, so they never
//! become turns.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use terminus_core::retention::{self, Selection};
use terminus_core::store::{Store, DB_FILE_NAME};
use terminus_core::{ingest, reindex};

const SESSION: &str = "33333333-3333-4333-8333-333333333333";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    transcript: PathBuf,
    key: String,
}

fn ai(title: &str) -> String {
    serde_json::json!({"type": "ai-title", "sessionId": SESSION, "aiTitle": title}).to_string()
}

fn custom(title: &str) -> String {
    serde_json::json!({"type": "custom-title", "sessionId": SESSION, "customTitle": title})
        .to_string()
}

fn prompt(cwd: &Path) -> String {
    serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "cwd": cwd.to_string_lossy(),
        "sessionId": SESSION,
        "type": "user",
        "uuid": "cccccccc-0000-4000-8000-000000000001",
        "timestamp": "2026-08-12T10:00:00.000Z",
        "message": {"role": "user", "content": [{"type": "text", "text": "the prompt"}]},
    })
    .to_string()
}

/// A session of one prompt and these title records, ingested.
fn bench(titles: &[String]) -> Bench {
    let dir = tempfile::tempdir_in(terminus_core::testkit::temp_root()).unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let transcript = work.join(format!("{SESSION}.jsonl"));
    let mut lines = vec![prompt(&dir.path().join("project"))];
    lines.extend_from_slice(titles);
    std::fs::write(&transcript, lines.join("\n") + "\n").unwrap();
    let key = match ingest::run(&data_dir, &transcript).unwrap() {
        ingest::Outcome::Committed(pass) => pass.session_key,
        other => panic!("{other:?}"),
    };
    Bench {
        _dir: dir,
        data_dir,
        transcript,
        key,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn title(&self) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT title FROM session_titles WHERE session_key = ?1",
                [&self.key],
                |r| r.get(0),
            )
            .ok()
    }

    /// Add one record to the transcript and ingest the tail.
    fn append(&self, line: &str) {
        let mut body = std::fs::read_to_string(&self.transcript).unwrap();
        body.push_str(line);
        body.push('\n');
        std::fs::write(&self.transcript, body).unwrap();
        match ingest::run(&self.data_dir, &self.transcript).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_session_with_no_title_record_has_no_row() {
    assert_eq!(bench(&[]).title(), None);
}

#[test]
fn the_newest_generated_title_wins() {
    let bench = bench(&[ai("first"), ai("second")]);
    assert_eq!(bench.title().as_deref(), Some("second"));
}

/// Claude Code keeps writing the generated title after a person sets their own,
/// and the person's is the one to keep.
#[test]
fn a_generated_title_never_replaces_one_a_person_set() {
    let bench = bench(&[ai("generated"), custom("mine"), ai("generated again")]);
    assert_eq!(bench.title().as_deref(), Some("mine"));

    // Across passes too, which is a different statement from the one above.
    bench.append(&ai("generated in a later pass"));
    assert_eq!(bench.title().as_deref(), Some("mine"));
    bench.append(&custom("renamed"));
    assert_eq!(bench.title().as_deref(), Some("renamed"));
}

#[test]
fn a_blank_title_replaces_nothing() {
    let bench = bench(&[ai("kept"), ai("   "), custom("")]);
    assert_eq!(bench.title().as_deref(), Some("kept"));
}

/// A store from before this table gets its titles off the archive, which is the
/// whole reason the table is derived.
#[test]
fn a_rebuild_fills_the_title_from_the_archive() {
    let bench = bench(&[ai("generated"), custom("mine"), ai("generated again")]);
    bench
        .conn()
        .execute_batch("DELETE FROM session_titles")
        .unwrap();
    assert_eq!(bench.title(), None, "the premise");

    let rebuilt = reindex::reindex(&mut Store::open(&bench.data_dir).unwrap()).unwrap();
    assert_eq!(rebuilt.preserved, 0);
    assert_eq!(bench.title().as_deref(), Some("mine"));
}

/// Eviction takes the bytes a title could be rebuilt from, so its row stays, and
/// the rebuild that skips the session leaves it standing.
#[test]
fn an_evicted_session_keeps_its_title_and_a_deleted_one_loses_it() {
    let evicted = bench(&[ai("kept through eviction")]);
    let mut store = Store::open(&evicted.data_dir).unwrap();
    let applied = retention::apply(
        &mut store,
        &Selection {
            evict: vec![evicted.key.clone()],
            ..Selection::default()
        },
    );
    assert_eq!(applied.evicted, vec![evicted.key.clone()]);
    let rebuilt = reindex::reindex(&mut store).unwrap();
    assert_eq!(rebuilt.preserved, 1, "the rebuild took the wrong path");
    assert_eq!(evicted.title().as_deref(), Some("kept through eviction"));

    let deleted = bench(&[ai("gone with the session")]);
    std::fs::remove_file(&deleted.transcript).unwrap();
    let mut store = Store::open(&deleted.data_dir).unwrap();
    let applied = retention::apply(
        &mut store,
        &Selection {
            delete: vec![deleted.key.clone()],
            ..Selection::default()
        },
    );
    assert!(applied.errors().is_empty(), "{:?}", applied.errors());
    assert_eq!(deleted.title(), None);
}
