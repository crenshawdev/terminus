//! The dropped-turn set, over a store built by the ordinary ingest path
//! (INJ-05, D-07).
//!
//! `session-compacted.jsonl` is what makes this a test rather than a
//! restatement: its four records are shaped so that the three readings of a
//! `compactMetadata` object give three different answers. `preservedMessages`
//! lists the second and third uuids, `allUuids` lists the first three, and
//! `preservedSegment.headUuid` is the first. So the right reading returns the
//! first record's turn and both wrong ones return nothing at all - which means
//! "the dropped set is not empty" is the whole assertion, and a fixture whose
//! readings agreed would have proved nothing.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::inject::compaction;
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{ingest, testkit};

/// The fixture whose last record is a `compact_boundary`.
const FIXTURE: &str = "session-compacted.jsonl";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    session_key: String,
}

/// A store holding the compacted fixture, ingested the ordinary way.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let transcript = testkit::copy_fixture_into(FIXTURE, &work);

    let session_key = match ingest::run(&data_dir, &transcript).unwrap() {
        ingest::Outcome::Committed(pass) => pass.session_key,
        other => panic!("{FIXTURE}: {other:?}"),
    };
    Bench {
        _dir: dir,
        data_dir,
        session_key,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// Every `(uuid, turn id)` of the archived session, in transcript order.
    fn turns(&self) -> Vec<(String, i64)> {
        self.conn()
            .prepare("SELECT uuid, id FROM turns WHERE session_key = ?1 ORDER BY turn_seq")
            .unwrap()
            .query_map([&self.session_key], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The turn id one uuid was archived as.
    fn turn_of(&self, uuid: &str) -> i64 {
        self.turns()
            .into_iter()
            .find(|(stored, _)| stored == uuid)
            .unwrap_or_else(|| panic!("{uuid} is not in the archive"))
            .1
    }

    /// The stored `compactMetadata`, as the object the reader parses.
    fn metadata(&self) -> serde_json::Value {
        let bytes: Vec<u8> = self
            .conn()
            .query_row("SELECT metadata FROM compaction_boundaries", [], |r| {
                r.get(0)
            })
            .expect("the fixture archived a boundary");
        serde_json::from_slice(&bytes).expect("the stored metadata is JSON")
    }

    /// The uuids one list of the metadata names.
    fn listed(&self, field: &str) -> Vec<String> {
        self.metadata()["preservedMessages"][field]
            .as_array()
            .unwrap_or_else(|| panic!("no preservedMessages.{field}"))
            .iter()
            .map(|uuid| uuid.as_str().expect("a uuid").to_owned())
            .collect()
    }

    /// The dropped set of a session that HAS a boundary, which every case here
    /// but the last one is about.
    fn dropped(&self) -> Vec<i64> {
        compaction::dropped(&self.conn(), &self.session_key)
            .expect("the fixture archived a boundary")
            .into_iter()
            .collect()
    }
}

/// D-07: the complement of `preservedMessages.uuids`, and not one of the three
/// readings that look like it.
#[test]
fn the_dropped_set_is_what_preserved_messages_does_not_name() {
    let bench = bench();
    let turns = bench.turns();
    assert_eq!(turns.len(), 4, "the fixture archived {turns:?}");

    let preserved = bench.listed("uuids");
    let expected: Vec<i64> = turns
        .iter()
        .take(turns.len() - 1) // never the boundary's own turn
        .filter(|(uuid, _)| !preserved.contains(uuid))
        .map(|(_, id)| *id)
        .collect();
    assert_eq!(expected.len(), 1, "the fixture drops {expected:?}");

    assert_eq!(bench.dropped(), expected);

    // The two wrong readings, spelled out: each returns nothing over this
    // fixture, so the assertion above is about which list was read and not just
    // about a query that ran.
    let all: Vec<String> = bench.listed("allUuids");
    let complement_of_all: Vec<&(String, i64)> = turns
        .iter()
        .take(turns.len() - 1)
        .filter(|(uuid, _)| !all.contains(uuid))
        .collect();
    assert!(
        complement_of_all.is_empty(),
        "allUuids does not name every pre-boundary turn, so reading it would \
         have been distinguishable for the wrong reason: {complement_of_all:?}"
    );
    let head = bench.metadata()["preservedSegment"]["headUuid"]
        .as_str()
        .expect("a headUuid")
        .to_owned();
    assert_eq!(
        head, turns[0].0,
        "headUuid is not the first record, so \"everything before it\" is not \
         empty and this fixture does not separate the readings"
    );
}

/// The boundary record is a turn like any other, and it is not dropped: it is
/// the one record of the compaction the model certainly still has.
#[test]
fn the_boundary_turn_is_never_in_its_own_dropped_set() {
    let bench = bench();
    let turns = bench.turns();
    let boundary = turns.last().expect("the fixture has turns").1;

    assert!(!bench.dropped().contains(&boundary));
    for uuid in bench.listed("uuids") {
        assert!(
            !bench.dropped().contains(&bench.turn_of(&uuid)),
            "{uuid} is preserved and was offered as dropped"
        );
    }
}

/// Every unreadable metadata is an empty set and never an error: the format is
/// one undocumented example wide, and a reader that threw would turn a change
/// upstream into a prompt that fails.
#[test]
fn a_boundary_that_cannot_be_read_drops_nothing() {
    let bench = bench();
    assert_eq!(bench.dropped().len(), 1, "the control did not fire");

    for (what, metadata) in [
        ("bytes that are not JSON", b"not json at all".to_vec()),
        ("JSON that is not an object", b"[1,2,3]".to_vec()),
        ("an object with no preservedMessages", b"{}".to_vec()),
        (
            "uuids that are not an array",
            br#"{"preservedMessages":{"uuids":"all of them"}}"#.to_vec(),
        ),
        (
            "an empty uuid list",
            br#"{"preservedMessages":{"uuids":[]}}"#.to_vec(),
        ),
    ] {
        bench
            .conn()
            .execute(
                "UPDATE compaction_boundaries SET metadata = ?1",
                [&metadata],
            )
            .unwrap();
        assert!(
            bench.dropped().is_empty(),
            "{what} yielded a dropped set: {:?}",
            bench.dropped()
        );
    }

    bench
        .conn()
        .execute("UPDATE compaction_boundaries SET metadata = NULL", [])
        .unwrap();
    assert!(bench.dropped().is_empty(), "a null metadata dropped turns");
}

/// A session nothing ever compacted, which is nearly every session: no
/// boundary row, no dropped set, no error.
///
/// `None` rather than an empty set, and the two are not the same answer. An
/// empty set means a boundary was read and dropped nothing this build can
/// name; `None` means no boundary is there yet, which on the prompt path is a
/// debt to carry rather than one to settle (D-08).
#[test]
fn a_session_with_no_boundary_is_not_a_session_that_dropped_nothing() {
    let bench = bench();
    assert_eq!(
        compaction::dropped(&bench.conn(), "/no/such/session.jsonl"),
        None
    );

    let conn = bench.conn();
    conn.execute("UPDATE compaction_boundaries SET metadata = x'6e6f'", [])
        .unwrap();
    assert_eq!(
        compaction::dropped(&conn, &bench.session_key),
        Some(std::collections::BTreeSet::new()),
        "an unreadable boundary is still a boundary"
    );

    conn.execute("DELETE FROM compaction_boundaries", [])
        .unwrap();
    assert_eq!(compaction::dropped(&conn, &bench.session_key), None);
}
