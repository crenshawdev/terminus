//! RCL-01 through RCL-04: what a turn leaves behind in the index.
//!
//! Every test here runs against a store holding the whole fixture corpus, built
//! by the ordinary ingest path, so what is asserted is what a real pass wrote
//! and not what a helper decided to insert.

#![cfg(feature = "testkit")]

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{ingest, testkit};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
}

/// A store with every transcript fixture ingested.
fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();

    for fixture in testkit::TRANSCRIPT_FIXTURES {
        let rooted = testkit::ROOTED_FIXTURES.iter().any(|(f, _)| f == fixture);
        let path = if rooted {
            testkit::copy_rooted_fixture_into(fixture, &work, &root)
        } else {
            testkit::copy_fixture_into(fixture, &work)
        };
        match ingest::run(&data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{fixture}: {other:?}"),
        }
    }

    Bench {
        _dir: dir,
        data_dir,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }
}

/// The turn ids one FTS query returns, in id order.
fn matching(conn: &Connection, query: &str) -> Vec<i64> {
    conn.prepare("SELECT rowid FROM turns_fts WHERE turns_fts MATCH ?1 ORDER BY rowid")
        .unwrap()
        .query_map([query], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// One turn's record bytes, read back out of its session blob.
fn record(conn: &Connection, turn_id: i64) -> String {
    let (bytes, _) = testkit::read_turn(conn, turn_id);
    String::from_utf8(bytes).unwrap()
}

/// How many archived turns carry `needle` anywhere in their raw record.
///
/// The control every negative assertion below needs: "zero hits" proves nothing
/// unless the bytes were there to be hit.
fn turns_whose_record_contains(conn: &Connection, needle: &str) -> Vec<i64> {
    conn.prepare("SELECT id FROM turns ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .filter(|id| record(conn, *id).contains(needle))
        .collect()
}

/// D-01, the negative half: no JSON key and no record-type literal reaches the
/// index, however many records carry it.
///
/// `toolUseResult` is the named case - a key on 4,858 records of the measured
/// sample - but the failure was general. Phase 1 indexed
/// `String::from_utf8_lossy(record)`, so every key, every field value and every
/// record type was a searchable term, and BM25 ranked turns by how much
/// scaffolding they carried.
#[test]
fn no_json_key_and_no_record_type_literal_is_indexed() {
    let bench = bench();
    let conn = bench.conn();

    for key in [
        "toolUseResult",
        "parentUuid",
        "isSidechain",
        "sessionId",
        "gitBranch",
        "requestId",
        "tool_use_id",
    ] {
        assert!(
            !turns_whose_record_contains(&conn, key).is_empty(),
            "no archived turn carries `{key}`, so a zero-hit assertion proves nothing"
        );
        assert_eq!(
            matching(&conn, key),
            Vec::<i64>::new(),
            "the JSON key `{key}` is searchable"
        );
    }

    // The record-type literal, which is the case that made free-text search
    // return most of the archive: `MATCH 'assistant'` used to return every
    // assistant record on the strength of `"type":"assistant"` alone.
    let assistants = turns_whose_record_contains(&conn, "\"type\":\"assistant\"");
    assert!(
        assistants.len() > 5,
        "the fixture corpus has assistant turns"
    );
    assert_eq!(matching(&conn, "assistant"), Vec::<i64>::new());

    // And a field VALUE outside the four projected subtrees is no more
    // searchable than a key: `gitBranch: "restart"` is on nearly every fixture
    // record and is not something the turn said.
    assert!(!turns_whose_record_contains(&conn, "restart").is_empty());
    assert_eq!(matching(&conn, "restart"), Vec::<i64>::new());
}

/// D-01, the positive half: each of the four projected subtrees is reachable,
/// and a phrase a turn actually said still finds it.
#[test]
fn every_projected_subtree_is_searchable() {
    let bench = bench();
    let conn = bench.conn();

    // A `message.content` text block.
    let phrase = matching(&conn, "\"now run the fixture test\"");
    assert_eq!(phrase.len(), 1, "a phrase from a message text block");
    assert!(record(&conn, phrase[0]).contains("now run the fixture test"));

    // A `system` record's TOP-LEVEL `content`, which is where that record type
    // puts its text rather than under `message`. Two hits, not one:
    // `session-truncated.jsonl` is a byte prefix of `session-basic.jsonl` and
    // both are archived, so every turn before the cut is in the store twice.
    let system = matching(&conn, "\"ran cargo build\"");
    assert_eq!(system.len(), 2);
    for id in &system {
        assert!(record(&conn, *id).contains("\"type\":\"system\""));
    }

    // A `tool_use` block's name and the string leaves of its `input`.
    assert!(!matching(&conn, "Grep").is_empty());
    assert!(!matching(&conn, "retryBudget").is_empty());

    // A top-level `toolUseResult`, which is 41% of the corpus by bytes.
    let stderr = matching(&conn, "panicked");
    assert_eq!(stderr.len(), 2, "one flagged failure in each error session");

    // A top-level `attachment` object's string leaves.
    let attachment = matching(&conn, "BRIEF");
    assert_eq!(attachment.len(), 2);
    for id in &attachment {
        assert!(record(&conn, *id).contains("\"type\":\"attachment\""));
    }

    // A `tool_result` block's own content, which is not the same field.
    assert_eq!(matching(&conn, "\"Bash command failed\"").len(), 2);
}

/// The fixed query set is only worth comparing across a rebuild if every entry
/// in it matches something. Two of the five did not until the projection
/// landed - they matched JSON scaffolding, which is nobody's query.
#[test]
fn every_fixed_query_matches_a_real_turn() {
    let bench = bench();
    let conn = bench.conn();
    for query in testkit::FIXED_QUERIES {
        assert!(
            !matching(&conn, query).is_empty(),
            "the fixed query `{query}` matches no fixture turn"
        );
    }
}

/// A record the parser could not read projects to nothing, never to its bytes.
///
/// Unreachable for a real turn - D-03 classifies one only when `uuid` and
/// `timestamp` parsed out of it - and asserted anyway, because the fallback's
/// whole job is to be the one place JSON scaffolding could come back.
#[test]
fn an_unparseable_record_projects_to_nothing() {
    use verbatim_core::derive::{self, TurnRow};
    use verbatim_core::parse;

    let dir = tempfile::tempdir().unwrap();
    let store = verbatim_core::Store::open(dir.path()).unwrap();
    let conn = store.conn();
    conn.execute(
        "INSERT INTO sessions (session_key, session_no, blob) VALUES ('k', 0, x'00')",
        [],
    )
    .unwrap();

    let good = br#"{"type":"user","uuid":"u1","timestamp":"t","message":{"role":"user","content":[{"type":"text","text":"vorpal"}]}}"#;
    let scan = parse::scan(&[good.as_slice(), b"\n"].concat());
    let (_, turn) = scan.turns().next().unwrap();

    let broken =
        br#"{"type":"user","uuid":"u2","timestamp":"t","message":{"content":[{"text":"vorpal"#;
    let id = derive::derive_turn(
        conn,
        TurnRow {
            session_key: "k",
            session_no: 0,
            turn,
            stream_offset: 0,
            byte_len: broken.len() as u64,
            record: broken,
            subtype: None,
            compact_metadata: None,
        },
    )
    .unwrap();

    assert_eq!(
        matching(conn, "vorpal"),
        Vec::<i64>::new(),
        "an unparseable record reached the index"
    );
    let body: i64 = conn
        .query_row(
            "SELECT count(*) FROM turns_fts WHERE rowid = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(body, 1, "the row itself must still exist, empty");
}

// --- RCL-01: expansion -------------------------------------------------------

/// AC1's first half, and D-13's measured claim: case splitting is the rule that
/// changes recall.
///
/// The second assertion is the falsifying one. `MATCH 'worker'` passes with no
/// expansion written at all, because `unicode61` splits on `/` by itself - a
/// phase that shipped only path expansion would look half correct on this
/// criterion and be none of it. `MATCH 'manager'` is the half that can only
/// pass if the camel rule ran.
#[test]
fn a_component_query_finds_the_whole_token() {
    let bench = bench();
    let conn = bench.conn();

    let whole = matching(&conn, "SearchManager");
    assert_eq!(whole.len(), 1, "one turn carries `SearchManager`");
    assert_eq!(
        matching(&conn, "manager"),
        whole,
        "the camel component does not find the token, so no expansion was written"
    );
    // The other component of the same token. Not an equality: a sidecar turn
    // says "search the tree for block framing" in prose, so `search` is a word
    // the corpus already has and expansion adds this turn to its hits.
    assert!(matching(&conn, "search").contains(&whole[0]));
    assert!(record(&conn, whole[0]).contains("SearchManager"));

    let pathy = matching(&conn, "worker");
    assert_eq!(pathy.len(), 1);
    assert!(record(&conn, pathy[0]).contains("src/worker/S.ts"));

    // The same rule over a tool input rather than a message text block.
    let camel_input = matching(&conn, "retryBudget");
    assert!(!camel_input.is_empty());
    assert!(matching(&conn, "budget").len() > camel_input.len());
}

/// The rules on their own, where a boundary case is cheap to state.
#[test]
fn the_expansion_rules_are_functions_with_answers() {
    use verbatim_core::index::expand::{case_components, expansion_tokens, separator_components};

    assert_eq!(case_components("SearchManager"), ["Search", "Manager"]);
    assert_eq!(case_components("searchManager"), ["search", "Manager"]);
    assert_eq!(case_components("HTTPServer"), ["HTTP", "Server"]);
    assert_eq!(case_components("S3Client"), ["S3", "Client"]);
    // Digits stay attached: nothing in this phase specifies bare-integer
    // handling, and splitting them puts every version number in twice.
    assert_eq!(case_components("v2"), ["v2"]);
    assert_eq!(case_components("plain"), ["plain"]);
    assert_eq!(case_components(""), [""]);

    assert_eq!(
        separator_components("src/worker/S.ts").collect::<Vec<_>>(),
        ["src", "worker", "S", "ts"]
    );
    assert_eq!(
        separator_components("search_manager_new").collect::<Vec<_>>(),
        ["search", "manager", "new"]
    );

    // Nothing already reachable is emitted a second time: `unicode61` splits on
    // every separator, so the snake, kebab and path rules add no bytes (D-13),
    // and the case rule adds only what is genuinely new.
    assert!(expansion_tokens("search_manager_new").is_empty());
    assert!(expansion_tokens("src/worker/S.ts").is_empty());
    assert!(expansion_tokens("--setting-sources").is_empty());
    assert_eq!(expansion_tokens("SearchManager"), ["Search", "Manager"]);
    // Deduplicated per body, and in text order, which is what makes a rebuild
    // reproduce a byte-identical row.
    assert_eq!(
        expansion_tokens("SearchManager and SearchManager again"),
        ["Search", "Manager"]
    );
    assert_eq!(
        expansion_tokens("searchManager Search"),
        ["Manager"],
        "`Search` is already a token of the body"
    );
}
