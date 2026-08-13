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

// --- RCL-02: entities --------------------------------------------------------

/// Every `(kind, value_norm)` one turn carries, in `entities` order.
fn entities_of(conn: &Connection, turn_id: i64) -> Vec<(String, String)> {
    conn.prepare("SELECT kind, value_norm FROM entities WHERE turn_id = ?1 ORDER BY rowid")
        .unwrap()
        .query_map([turn_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn paths_of(conn: &Connection, turn_id: i64) -> Vec<String> {
    conn.prepare("SELECT path FROM paths WHERE turn_id = ?1 ORDER BY rowid")
        .unwrap()
        .query_map([turn_id], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The one turn whose record contains `needle`, or a failure naming how many
/// there were.
fn only_turn_containing(conn: &Connection, needle: &str) -> i64 {
    let hits = turns_whose_record_contains(conn, needle);
    assert_eq!(hits.len(), 1, "`{needle}` is in {} turns", hits.len());
    hits[0]
}

/// AC3's structured-versus-prose pair, which is what "entities are not
/// extracted from prose" has to mean to be worth anything.
#[test]
fn a_structured_path_becomes_an_entity_and_the_same_path_in_prose_does_not() {
    let bench = bench();
    let conn = bench.conn();

    let structured = only_turn_containing(&conn, "\"file_path\":\"docs/RETRY.md\"");
    assert!(entities_of(&conn, structured).contains(&("path".into(), "docs/RETRY.md".into())));
    assert_eq!(paths_of(&conn, structured), ["docs/RETRY.md"]);

    let prose = only_turn_containing(&conn, "docs/RETRY.md documents nothing");
    assert_ne!(prose, structured);
    assert_eq!(
        entities_of(&conn, prose),
        Vec::<(String, String)>::new(),
        "a path named in prose became an entity"
    );
    assert_eq!(paths_of(&conn, prose), Vec::<String>::new());
}

/// The `tool` kind and `turns.tool_name` are the same value read twice, so a
/// filter on either cannot disagree with the other (D-16).
#[test]
fn the_tool_entity_agrees_with_the_turn_column() {
    let bench = bench();
    let conn = bench.conn();

    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id, tool_name FROM turns WHERE tool_name IS NOT NULL ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(rows.len() >= 5, "the corpus must exercise several tools");

    for (id, tool_name) in &rows {
        let tools: Vec<String> = entities_of(&conn, *id)
            .into_iter()
            .filter(|(kind, _)| kind == "tool")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(tools, [tool_name.as_str()], "turn {id}");
    }

    // And the other direction: no turn carries a `tool` entity without the
    // column, which is what a second extraction site would look like.
    let orphans: i64 = conn
        .query_row(
            "SELECT count(*) FROM entities e JOIN turns t ON t.id = e.turn_id
             WHERE e.kind = 'tool' AND t.tool_name IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0);

    let grep = only_turn_containing(&conn, "\"pattern\":\"block\"");
    assert!(entities_of(&conn, grep).contains(&("tool".into(), "Grep".into())));
}

/// A `Bash` call leaves the program it ran, by basename.
#[test]
fn a_bash_call_leaves_the_program_it_ran() {
    let bench = bench();
    let conn = bench.conn();

    // Two turns, not one: `session-truncated.jsonl` is a byte prefix of
    // `session-basic.jsonl` that reaches past this record, so both sessions
    // hold it and both must extract the same thing.
    let bash = turns_whose_record_contains(&conn, "\"command\":\"cargo test -p verbatim-core\"");
    assert_eq!(bash.len(), 2);
    for id in bash {
        let entities = entities_of(&conn, id);
        assert!(
            entities.contains(&("command".into(), "cargo".into())),
            "turn {id}: {entities:?}"
        );
        assert!(entities.contains(&("tool".into(), "Bash".into())));
        // No path in that command line, and nothing invented for the
        // subcommand or the `-p` value.
        assert!(paths_of(&conn, id).is_empty());
    }
}

/// A `Grep` pattern and an `Edit` string give symbols; an ordinary word does
/// not. The shape test is the whole rule - both fields can hold prose.
#[test]
fn only_identifier_shaped_tokens_become_symbols() {
    let bench = bench();
    let conn = bench.conn();

    let grep = only_turn_containing(&conn, "\"pattern\":\"retryBudget\"");
    let symbols: Vec<String> = entities_of(&conn, grep)
        .into_iter()
        .filter(|(kind, _)| kind == "symbol")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(symbols, ["retryBudget"]);

    // The other Grep in the corpus searches for the ordinary word `block`, and
    // an ordinary word is not a symbol.
    let plain = only_turn_containing(&conn, "\"pattern\":\"block\"");
    assert!(entities_of(&conn, plain)
        .iter()
        .all(|(kind, _)| kind != "symbol"));
}

/// The rules on their own, where the boundary cases are cheap to state.
#[test]
fn the_entity_rules_are_functions_with_answers() {
    use verbatim_core::index::entity::{
        is_identifier_shaped, normalize_path, path_words, program_of,
    };

    assert_eq!(
        program_of("cargo test -p verbatim-core"),
        Some("cargo".into())
    );
    assert_eq!(program_of("/usr/bin/env python3 x.py"), Some("env".into()));
    assert_eq!(
        program_of("RUST_LOG=debug cargo build"),
        Some("cargo".into())
    );
    assert_eq!(program_of("  "), None);

    assert_eq!(
        path_words("cargo test -p verbatim-core"),
        Vec::<String>::new()
    );
    assert_eq!(path_words("cat src/main.rs"), ["src/main.rs"]);
    // A flag is skipped; a flag's VALUE is not, because telling the two apart
    // needs per-tool argv knowledge this has no way to have, and `--config
    // etc/x.toml` names a file exactly as much as a bare argument does.
    assert_eq!(
        path_words("rg --glob a/b.rs pattern crates/x/src"),
        ["a/b.rs", "crates/x/src"]
    );

    // A command line is shell syntax, and a path arrives welded to it. Each of
    // these stored the punctuation as part of the key, so a lookup for the real
    // path missed the turn that named it.
    assert_eq!(
        path_words("cd /data/code/verbatim && cargo build"),
        ["/data/code/verbatim"]
    );
    assert_eq!(path_words("cd /a/b; make"), ["/a/b"]);
    assert_eq!(path_words("cargo test 2>/dev/null"), ["/dev/null"]);
    assert_eq!(
        path_words("grep -rn foo src/x >>/tmp/out.log"),
        ["src/x", "/tmp/out.log"]
    );
    assert_eq!(path_words("(cd /a/b && ls) | wc -l"), ["/a/b"]);
    assert_eq!(path_words("git commit -m \"fix /a/b\""), ["/a/b"]);

    // Dropped outright: none of these is a path that happens to be unpopular,
    // and none is a key an exact-match lookup can ever be handed.
    assert_eq!(path_words("ls ${ROOT:-/home/john}/x"), Vec::<String>::new());
    // A substitution opened in one word and closed in the next is past what a
    // split can see: `$(dirname` is dropped for its `$`, and the rest is judged
    // on its own bytes. `a/b` is genuinely a path the command named; `/c` is
    // the fragment after the closing paren. Stated rather than asserted away -
    // separating them needs a shell parser, for 151 of 21,061 measured words.
    assert_eq!(path_words("cat $(dirname a/b)/c"), ["a/b", "/c"]);
    assert_eq!(path_words("rm src/*.rs"), Vec::<String>::new());
    // The brace list goes; the plain destination beside it stays.
    assert_eq!(path_words("cp src/{a,b}.rs /tmp"), ["/tmp"]);
    assert_eq!(
        path_words("curl https://example.com/x"),
        Vec::<String>::new()
    );
    // `2>&1` leaves two words, neither carrying a separator.
    assert_eq!(path_words("cargo build 2>&1"), Vec::<String>::new());

    // Kept exactly as written, minus quotes and a position suffix. Never
    // canonicalized: 52% of the corpus's cwd directories are gone.
    assert_eq!(
        normalize_path("\"src/main.rs\""),
        Some("src/main.rs".into())
    );
    assert_eq!(
        normalize_path("src/main.rs:42:9"),
        Some("src/main.rs".into())
    );
    assert_eq!(normalize_path("src/main.rs:42"), Some("src/main.rs".into()));
    assert_eq!(normalize_path("../a/b.rs"), Some("../a/b.rs".into()));
    assert_eq!(normalize_path("   "), None);

    assert!(is_identifier_shaped("SearchManager"));
    assert!(is_identifier_shaped("search_manager"));
    assert!(is_identifier_shaped("retryBudget"));
    assert!(!is_identifier_shaped("block"));
    assert!(!is_identifier_shaped("Search"));
    assert!(!is_identifier_shaped("_leading"));
}

// --- RCL-03: errors ----------------------------------------------------------

/// Every `error` value one turn carries, in `entities` order.
fn errors_of(conn: &Connection, turn_id: i64) -> Vec<String> {
    entities_of(conn, turn_id)
        .into_iter()
        .filter(|(kind, _)| kind == "error")
        .map(|(_, value)| value)
        .collect()
}

/// AC2 in both directions: the four variable parts collapse and a bare integer
/// does not.
///
/// The second half is the one that costs something to get right. Stripping bare
/// integers too was measured over a 400-file sample and scores *worse* on
/// recurrence - 39 recurring values against 46 - because it merges failures that
/// differ in a count, an index or an exit status into one entity nobody can tell
/// apart again.
#[test]
fn one_failure_seen_twice_is_one_value_and_a_different_integer_is_two() {
    let bench = bench();
    let conn = bench.conn();

    // The same panic in two sessions a week apart: different line, different
    // address, different timestamp, different job id.
    let a = only_turn_containing(&conn, "reader.rs:214:9");
    let b = only_turn_containing(&conn, "reader.rs:317:5");
    assert_eq!(
        errors_of(&conn, a),
        [
            // The `tool_result` block's own text, on `is_error: true`.
            "Bash command failed",
            // And the `stderr` beside it, normalized.
            "thread 'main' panicked at crates/verbatim-core/src/blob/reader.rs:<line> \
             assertion failed at <addr> on <ts> for job <uuid>",
        ]
    );
    assert_eq!(errors_of(&conn, a), errors_of(&conn, b));

    // Two failures whose only difference is a count. Same shape, same length,
    // no variable part any rule here touches - and they must stay apart.
    let seven = only_turn_containing(&conn, "7 previous errors");
    let nine = only_turn_containing(&conn, "9 previous errors");
    assert_eq!(
        errors_of(&conn, seven),
        ["error: could not compile verbatim-core due to 7 previous errors"],
        "a bare integer was stripped, or the stderr signal did not fire"
    );
    assert_ne!(errors_of(&conn, seven), errors_of(&conn, nine));

    // D-03's negative: `interrupted` says a call was cut short, not that it
    // failed, and an empty stderr is not a failure either.
    let interrupted = only_turn_containing(&conn, "\"interrupted\":true");
    assert!(record(&conn, interrupted).contains("the command was interrupted"));
    assert_eq!(
        errors_of(&conn, interrupted),
        Vec::<String>::new(),
        "an interrupted call was recorded as an error"
    );
}

/// The normalization on its own, where the boundary cases are cheap to state.
#[test]
fn the_error_normalization_is_a_function_with_answers() {
    use verbatim_core::index::entity::{normalize_error, MAX_ERROR_BYTES};

    let norm = |raw: &str| normalize_error(raw).unwrap_or_default();

    assert_eq!(
        norm("panicked at src/main.rs:42:9"),
        "panicked at src/main.rs:<line>"
    );
    assert_eq!(
        norm("panicked at src/main.rs:42"),
        "panicked at src/main.rs:<line>"
    );
    assert_eq!(norm("freed 0x7f3a1c2d4e00 twice"), "freed <addr> twice");
    assert_eq!(
        norm("at 2026-08-12T09:14:22.001Z and 2026-08-12T09:14:22+02:00"),
        "at <ts> and <ts>"
    );
    assert_eq!(
        norm("job 9f1c2b3a-4d5e-4f60-8a71-b2c3d4e5f601 failed"),
        "job <uuid> failed"
    );

    // Kept: a bare integer, a version, a token that merely contains hex.
    assert_eq!(norm("exit status 137"), "exit status 137");
    assert_eq!(norm("rustc 1.89.0 rejected it"), "rustc 1.89.0 rejected it");
    assert_eq!(norm("branch fix0xff"), "branch fix0xff");
    // A colon that is not a position: nothing precedes it to be a file.
    assert_eq!(norm("error: 42 things"), "error: 42 things");

    // One line, whatever the input's wrapping, so two occurrences that differ
    // only in whitespace are one value.
    assert_eq!(
        norm("  error:\n  two lines\t\tand a tab  "),
        "error: two lines and a tab"
    );

    // Nothing left is no entity at all, which is how a successful call's empty
    // stderr contributes none.
    assert_eq!(normalize_error(""), None);
    assert_eq!(normalize_error("   \n  "), None);

    // Bounded, on a character boundary: an entity value is a key, and a whole
    // test suite's stderr is a key that will never be looked up.
    let huge = "e".repeat(MAX_ERROR_BYTES * 3);
    assert_eq!(norm(&huge).len(), MAX_ERROR_BYTES);
    let wide = "é".repeat(MAX_ERROR_BYTES);
    assert!(norm(&wide).len() <= MAX_ERROR_BYTES);
}

// --- RCL-04: the per-turn cap ------------------------------------------------

/// A turn cannot write more entity rows than the cap, and the cut is positional
/// so a re-derive reproduces it exactly.
///
/// The reproducibility half is what makes the cap safe: AC3 asserts that
/// dropping the derived tables and rebuilding from the blobs alone reproduces
/// the same rows, which holds only because the cut depends on the record's own
/// bytes and on nothing the store knows. A cap applied by frequency or recency
/// would pass this test on a fresh store and fail it on a full one.
#[test]
fn a_turn_writes_no_more_entities_than_the_cap_and_the_same_ones_twice() {
    use verbatim_core::derive::{self, TurnRow};
    use verbatim_core::index::MAX_ENTITIES_PER_TURN;
    use verbatim_core::parse;

    let dir = tempfile::tempdir().unwrap();
    let store = verbatim_core::Store::open(dir.path()).unwrap();
    let conn = store.conn();
    conn.execute(
        "INSERT INTO sessions (session_key, session_no, blob) VALUES ('k', 0, x'00')",
        [],
    )
    .unwrap();

    // One `Bash` call naming three times the cap in files. The real corpus's
    // heaviest measured turn emits 146 entities against a p99 of 38, so this is
    // the shape of turn the cap exists for and not an invented one.
    let files: Vec<String> = (0..MAX_ENTITIES_PER_TURN * 3)
        .map(|n| format!("crates/x/src/f{n}.rs"))
        .collect();
    let line = serde_json::json!({
        "type": "assistant",
        "uuid": "u1",
        "timestamp": "2026-08-12T09:14:21.000Z",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_1",
                "name": "Bash",
                "input": { "command": format!("wc -l {}", files.join(" ")) }
            }]
        }
    })
    .to_string();
    let bytes = [line.as_bytes(), b"\n"].concat();
    let scan = parse::scan(&bytes);
    let (_, turn) = scan.turns().next().unwrap();

    let row = TurnRow {
        session_key: "k",
        session_no: 0,
        turn,
        stream_offset: 0,
        byte_len: line.len() as u64,
        record: line.as_bytes(),
        subtype: None,
        compact_metadata: None,
    };
    let id = derive::derive_turn(conn, row).unwrap();
    let first = entities_of(conn, id);

    assert_eq!(first.len(), MAX_ENTITIES_PER_TURN, "{first:?}");
    // In record-walk order, and the cut is simply where the walk ran out of
    // room: the tool, the program, then paths until the cap.
    assert_eq!(first[0], ("tool".into(), "Bash".into()));
    assert_eq!(first[1], ("command".into(), "wc".into()));
    assert_eq!(first[2], ("path".into(), "crates/x/src/f0.rs".into()));
    assert_eq!(
        first.last().unwrap().1,
        format!("crates/x/src/f{}.rs", MAX_ENTITIES_PER_TURN - 3)
    );
    // Every path entity still writes its `paths` row, so that table is capped
    // by the same cut rather than by one of its own.
    assert_eq!(paths_of(conn, id).len(), MAX_ENTITIES_PER_TURN - 2);

    derive::derive_turn(conn, row).unwrap();
    assert_eq!(
        entities_of(conn, id),
        first,
        "a re-derive of the same record wrote different rows"
    );
    assert_eq!(paths_of(conn, id).len(), MAX_ENTITIES_PER_TURN - 2);
}

/// `Read` nests its result path where `Write` and `Edit` do not, and reading
/// one of the two places dropped every `Read` result-side path in the archive.
#[test]
fn a_nested_result_path_becomes_an_entity_like_a_top_level_one() {
    use verbatim_core::index::entities;

    let paths = |record: serde_json::Value| -> Vec<String> {
        entities(&record)
            .into_iter()
            .filter(|e| e.kind == "path")
            .map(|e| e.value)
            .collect()
    };

    // `Read`: the path is at `toolUseResult.file.filePath`.
    assert_eq!(
        paths(serde_json::json!({
            "type": "user",
            "uuid": "u1",
            "timestamp": "2026-08-12T09:14:21.000Z",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": "1\tfn main(){}" }
            ]},
            "toolUseResult": { "type": "text", "file": {
                "filePath": "/a/b/main.rs", "content": "fn main(){}", "numLines": 1
            }}
        })),
        ["/a/b/main.rs"],
        "a Read result path is not extracted"
    );

    // The control the old code did handle: `Write`/`Edit` at the top level.
    assert_eq!(
        paths(serde_json::json!({
            "type": "user",
            "uuid": "u2",
            "timestamp": "2026-08-12T09:14:22.000Z",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_2", "content": "ok" }
            ]},
            "toolUseResult": { "filePath": "/a/b/new.rs" }
        })),
        ["/a/b/new.rs"]
    );

    // One value, however many keys carry it: `push` dedupes on (kind, value).
    assert_eq!(
        paths(serde_json::json!({
            "type": "user",
            "uuid": "u3",
            "timestamp": "2026-08-12T09:14:23.000Z",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_3", "content": "ok" }
            ]},
            "toolUseResult": { "filePath": "/a/b/x.rs", "file": { "filePath": "/a/b/x.rs" }}
        })),
        ["/a/b/x.rs"]
    );
}
