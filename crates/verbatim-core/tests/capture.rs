//! Capture mode: how much of each record the archive keeps (ING-07, AC7).
//!
//! The elision has to be true of two different things at once, and the tests
//! split on that line. Here in the first half it is a statement about one JSON
//! line: fewer bytes, still valid JSON, still the same record. In the second
//! half it is a statement about the store: strictly decreasing blobs, offsets
//! that still address what they say they address, and a `verify` and a
//! `reindex` that find nothing wrong.

#![cfg(feature = "testkit")]

use serde_json::Value;

use verbatim_core::capture::{self, ELIDED_FIELDS, ELISION_MARK, LEAN_THRESHOLD_BYTES};
use verbatim_core::config::CaptureMode;
use verbatim_core::parse::record::Record;
use verbatim_core::testkit;

const MODES: [CaptureMode; 3] = [CaptureMode::Full, CaptureMode::Lean, CaptureMode::Minimal];

/// The fixture's complete lines, without their newlines.
fn fixture_lines() -> Vec<Vec<u8>> {
    let bytes = testkit::fixture_bytes(testkit::CAPTURE_FIXTURE);
    assert_eq!(
        bytes.last(),
        Some(&b'\n'),
        "the fixture must end with a newline, like every real transcript"
    );
    bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

/// `Record::parse` is not `pub` in the way a test can reach directly, so the
/// classification is compared through a whole one-line scan.
fn classify(line: &[u8]) -> Record {
    let mut framed = line.to_vec();
    framed.push(b'\n');
    let scan = verbatim_core::parse::scan(&framed);
    assert_eq!(scan.records.len(), 1, "one line is one record");
    scan.records.into_iter().next().unwrap()
}

fn as_object(line: &[u8]) -> serde_json::Map<String, Value> {
    match serde_json::from_slice::<Value>(line) {
        Ok(Value::Object(o)) => o,
        other => panic!("an elided line must still be a JSON object, got {other:?}"),
    }
}

/// The fixture is the premise for everything below: two subtrees over the
/// threshold, two under, and two records with neither key.
#[test]
fn the_fixture_covers_every_arm_of_the_elision() {
    let lines = fixture_lines();
    assert_eq!(lines.len(), 6, "six records");

    let mut over = 0;
    let mut under = 0;
    let mut neither = 0;
    for line in &lines {
        let object = as_object(line);
        let mut carried = false;
        for field in ELIDED_FIELDS {
            let Some(value) = object.get(field) else {
                continue;
            };
            carried = true;
            let size = serde_json::to_vec(value).unwrap().len();
            if size > LEAN_THRESHOLD_BYTES {
                over += 1;
            } else {
                under += 1;
            }
        }
        if !carried {
            neither += 1;
        }
    }
    assert_eq!(over, 2, "one toolUseResult and one attachment over 8 KB");
    assert_eq!(under, 2, "one of each under it");
    assert_eq!(neither, 2, "a plain prompt and a plain assistant turn");
}

/// (a) Under `full` nothing moves - and nothing is even parsed, which is why the
/// default costs a store nothing at all.
#[test]
fn full_returns_every_line_byte_identical() {
    for line in fixture_lines() {
        let stored = capture::elide(&line, CaptureMode::Full);
        assert!(
            stored.as_ref() == line.as_slice(),
            "full must return the source bytes"
        );
        assert!(
            matches!(stored, std::borrow::Cow::Borrowed(_)),
            "full must not allocate, let alone re-serialize"
        );
    }
}

/// (b) Under `minimal`, every record carrying either key is shorter, is still
/// valid JSON, carries the mark - and is still the SAME turn.
#[test]
fn minimal_elides_both_keys_and_the_record_is_still_the_same_record() {
    let mut elided = 0;
    for line in fixture_lines() {
        let source = classify(&line);
        let stored = capture::elide(&line, CaptureMode::Minimal);
        let stored_record = classify(&stored);

        // The identity half, asserted for every line whether or not it shrank.
        assert_eq!(stored_record.record_type, source.record_type);
        assert_eq!(stored_record.session_id, source.session_id);
        assert_eq!(stored_record.subtype, source.subtype);
        assert_eq!(stored_record.is_turn(), source.is_turn());
        match (&stored_record.turn, &source.turn) {
            (Some(stored_turn), Some(source_turn)) => {
                assert_eq!(stored_turn.uuid, source_turn.uuid);
                assert_eq!(stored_turn.parent_uuid, source_turn.parent_uuid);
                assert_eq!(stored_turn.timestamp, source_turn.timestamp);
                assert_eq!(stored_turn.record_type, source_turn.record_type);
                assert_eq!(stored_turn.turn_seq, source_turn.turn_seq);
            }
            (None, None) => {}
            other => panic!("the turn appeared or vanished: {other:?}"),
        }

        let object = as_object(&line);
        let carried: Vec<&str> = ELIDED_FIELDS
            .iter()
            .copied()
            .filter(|field| object.contains_key(*field))
            .collect();
        if carried.is_empty() {
            assert!(
                stored.as_ref() == line.as_slice(),
                "a line carrying neither key must never be re-serialized"
            );
            continue;
        }

        elided += 1;
        assert!(
            stored.len() < line.len(),
            "minimal must shrink a record carrying {carried:?}: {} -> {}",
            line.len(),
            stored.len()
        );
        let stored_object = as_object(&stored);
        for field in carried {
            let value = stored_object.get(field).expect("the key itself must stay");
            let recorded = capture::elided_bytes(value)
                .unwrap_or_else(|| panic!("{field} carries no elision mark: {value}"));
            let original = serde_json::to_vec(&object[field]).unwrap().len();
            assert_eq!(
                recorded, original as u64,
                "{field}'s mark must record the bytes it stood in for"
            );
        }
    }
    assert_eq!(elided, 4, "four of the six records carry an elided key");
}

/// (c) Under `lean` only the LARGE subtrees go. A record whose tool result is
/// 77 bytes comes back byte-identical, which is the whole difference between
/// `lean` and `minimal`.
#[test]
fn lean_elides_only_what_is_over_the_threshold() {
    let mut shrank = 0;
    let mut untouched = 0;
    for line in fixture_lines() {
        let object = as_object(&line);
        let largest = ELIDED_FIELDS
            .iter()
            .filter_map(|field| object.get(*field))
            .map(|value| serde_json::to_vec(value).unwrap().len())
            .max()
            .unwrap_or(0);
        let stored = capture::elide(&line, CaptureMode::Lean);

        if largest > LEAN_THRESHOLD_BYTES {
            shrank += 1;
            assert!(stored.len() < line.len(), "an over-threshold subtree stays");
            let stored_object = as_object(&stored);
            let marked = ELIDED_FIELDS
                .iter()
                .filter_map(|field| stored_object.get(*field))
                .any(|value| capture::elided_bytes(value).is_some());
            assert!(marked, "the elision must say it happened");
        } else {
            untouched += 1;
            assert!(
                stored.as_ref() == line.as_slice(),
                "lean must leave an under-threshold record byte-identical: {} bytes",
                largest
            );
        }
    }
    assert_eq!(shrank, 2);
    assert_eq!(untouched, 4);
}

/// (d) Strictly decreasing, over the whole fixture, which is AC7's first half
/// stated about the lines before it is stated about the blobs.
#[test]
fn the_stored_bytes_strictly_decrease_from_full_to_lean_to_minimal() {
    let lines = fixture_lines();
    let total = |mode| {
        lines
            .iter()
            .map(|line| capture::elide(line, mode).len())
            .sum::<usize>()
    };
    let (full, lean, minimal) = (
        total(CaptureMode::Full),
        total(CaptureMode::Lean),
        total(CaptureMode::Minimal),
    );
    assert!(
        full > lean && lean > minimal,
        "full {full}, lean {lean}, minimal {minimal}"
    );
    assert_eq!(
        full,
        lines.iter().map(Vec::len).sum::<usize>(),
        "full is the source bytes"
    );
}

/// (e) A line that is not a JSON object is not this module's to rewrite. Under
/// every mode it keeps its bytes, because a scan that rewrote what it could not
/// classify is the one way an elision could lose a record.
#[test]
fn a_line_that_is_not_a_json_object_is_returned_untouched_under_every_mode() {
    let lines: [&[u8]; 6] = [
        b"not json at all",
        b"[1, 2, 3]",
        b"\"a bare string\"",
        b"42",
        b"{\"toolUseResult\": ",
        b"",
    ];
    for line in lines {
        for mode in MODES {
            let stored = capture::elide(line, mode);
            assert!(
                stored.as_ref() == line,
                "{mode:?} rewrote a line it cannot classify: {:?}",
                String::from_utf8_lossy(line)
            );
        }
    }
}

/// The mark is verbatim's own key and carries a count, so a reader can tell an
/// elision from an upstream field and can say what it cost.
#[test]
fn the_mark_is_recognized_only_in_its_exact_shape() {
    assert_eq!(
        capture::elided_bytes(&serde_json::json!({ELISION_MARK: 1024})),
        Some(1024)
    );
    for other in [
        serde_json::json!({ELISION_MARK: 1024, "stdout": ""}),
        serde_json::json!({ELISION_MARK: "1024"}),
        serde_json::json!({ELISION_MARK: -1}),
        serde_json::json!({"elided": 1024}),
        serde_json::json!({}),
        serde_json::json!("verbatimElided"),
        serde_json::json!(1024),
    ] {
        assert_eq!(
            capture::elided_bytes(&other),
            None,
            "{other} must not read as an elision mark"
        );
    }
}

/// An already-elided value is left alone rather than re-elided, so a re-ingest
/// of stored bytes cannot overwrite the recorded size with the size of the mark.
#[test]
fn an_already_elided_value_is_not_elided_again() {
    let line = fixture_lines()
        .into_iter()
        .find(|line| as_object(line).contains_key("toolUseResult"))
        .expect("the fixture carries one");
    let once = capture::elide(&line, CaptureMode::Minimal).into_owned();
    let twice = capture::elide(&once, CaptureMode::Minimal);
    assert!(
        twice.as_ref() == once.as_slice(),
        "a second pass must change nothing"
    );
}

/// The framing is the contract [`capture::elide_stream`] owes ingest: the same
/// number of lines, in the same order, each still terminated - because the
/// record count and the turn ordinals are read off exactly that.
#[test]
fn the_stream_keeps_its_framing_line_for_line() {
    let source = testkit::fixture_bytes(testkit::CAPTURE_FIXTURE);
    for mode in MODES {
        let stored = capture::elide_stream(&source, mode);
        assert_eq!(
            stored.iter().filter(|b| **b == b'\n').count(),
            source.iter().filter(|b| **b == b'\n').count(),
            "{mode:?} changed the line count"
        );
        assert_eq!(stored.last(), Some(&b'\n'), "{mode:?} lost the terminator");

        let source_scan = verbatim_core::parse::scan(&source);
        let stored_scan = verbatim_core::parse::scan(&stored);
        assert_eq!(
            stored_scan.records.len(),
            source_scan.records.len(),
            "{mode:?} changed the record count"
        );
        assert_eq!(
            stored_scan.turn_count(),
            source_scan.turn_count(),
            "{mode:?} changed the turn count"
        );
        for (stored_record, source_record) in stored_scan.records.iter().zip(&source_scan.records) {
            assert_eq!(stored_record.record_type, source_record.record_type);
            assert_eq!(
                stored_record.turn.as_ref().map(|t| &t.uuid),
                source_record.turn.as_ref().map(|t| &t.uuid),
                "{mode:?} moved a turn"
            );
            assert_eq!(
                stored_record.turn.as_ref().map(|t| t.turn_seq),
                source_record.turn.as_ref().map(|t| t.turn_seq),
                "{mode:?} renumbered a turn"
            );
        }
    }
}

/// An empty line has no record but it does have a byte, and the two coordinate
/// systems have to agree about it.
#[test]
fn an_empty_line_passes_through_the_stream() {
    let source: &[u8] = b"{\"type\":\"x\"}\n\n{\"type\":\"y\"}\n";
    for mode in MODES {
        assert!(
            capture::elide_stream(source, mode).as_ref() == source,
            "{mode:?} disturbed a stream it has no work in"
        );
    }
}

// --- The store half: what the archive holds after a pass ---------------------

use std::path::PathBuf;

use rusqlite::Connection;
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{blob, ingest, recall, reindex, verify, Config, Store};

/// One temp store fed one transcript under one mode.
struct Bench {
    _dir: tempfile::TempDir,
    _config_dir: tempfile::TempDir,
    data_dir: PathBuf,
    transcript: PathBuf,
    config: Config,
}

fn bench(mode: CaptureMode) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let transcript = testkit::copy_fixture_into(testkit::CAPTURE_FIXTURE, &work);

    let config_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        config_dir
            .path()
            .join(verbatim_core::config::CONFIG_FILE_NAME),
        format!("[capture]\nmode = \"{}\"\n", mode.as_str()),
    )
    .unwrap();
    let config = Config::load_from(config_dir.path()).expect("a capture table parses");
    assert_eq!(
        config.capture_mode(),
        mode,
        "the bench must ask for {mode:?}"
    );

    Bench {
        _dir: dir,
        _config_dir: config_dir,
        data_dir,
        transcript,
        config,
    }
}

impl Bench {
    fn ingest(&self) -> ingest::Pass {
        match ingest::run_with(&self.data_dir, &self.transcript, &self.config)
            .unwrap_or_else(|e| panic!("ingest: {e}"))
        {
            ingest::Outcome::Committed(pass) => pass,
            other => panic!("expected a committed pass, got {other:?}"),
        }
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// `(compressed blob bytes, uncompressed stream bytes)`.
    fn sizes(&self) -> (i64, i64) {
        self.conn()
            .query_row(
                "SELECT length(s.blob), m.uncompressed_len
                 FROM sessions s JOIN session_meta m USING (session_key)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn stream(&self) -> Vec<u8> {
        let bytes: Vec<u8> = self
            .conn()
            .query_row("SELECT blob FROM sessions", [], |r| r.get(0))
            .unwrap();
        blob::read_all(&bytes).unwrap()
    }

    fn mode(&self) -> Option<String> {
        self.conn()
            .query_row("SELECT capture_mode FROM session_meta", [], |r| r.get(0))
            .unwrap()
    }

    /// `(turn id, stream_offset, byte_len)` for every turn, in order.
    fn turns(&self) -> Vec<(i64, i64, i64)> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT id, stream_offset, byte_len FROM turns ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows
    }
}

/// AC7, at the store: the same transcript into three stores leaves strictly
/// fewer archived bytes each time, compressed AND uncompressed.
#[test]
fn the_three_modes_store_strictly_decreasing_blobs() {
    let mut compressed = Vec::new();
    let mut uncompressed = Vec::new();
    for mode in MODES {
        let bench = bench(mode);
        bench.ingest();
        let (blob_bytes, stream_bytes) = bench.sizes();
        assert_eq!(bench.mode().as_deref(), Some(mode.as_str()));
        compressed.push(blob_bytes);
        uncompressed.push(stream_bytes);
    }

    assert!(
        compressed[0] > compressed[1] && compressed[1] > compressed[2],
        "length(sessions.blob) must strictly decrease: {compressed:?}"
    );
    assert!(
        uncompressed[0] > uncompressed[1] && uncompressed[1] > uncompressed[2],
        "the stored stream must strictly decrease: {uncompressed:?}"
    );
    assert_eq!(
        uncompressed[0],
        testkit::fixture_bytes(testkit::CAPTURE_FIXTURE).len() as i64,
        "full stores the transcript"
    );
}

/// `full` is still the byte-for-byte mode, said about this fixture as well as
/// about `session-basic.jsonl` over in `tests/ingest.rs`.
#[test]
fn full_still_reproduces_the_transcript_byte_for_byte() {
    let bench = bench(CaptureMode::Full);
    let pass = bench.ingest();
    let source = std::fs::read(&bench.transcript).unwrap();

    assert!(bench.stream() == source, "AC7, for the capture fixture");
    assert_eq!(pass.watermark, source.len() as u64);
    assert_eq!(bench.mode().as_deref(), Some("full"));
}

/// (b) Every turn row addresses the STORED stream. Read the range each row
/// claims and it is that record's line - which is the property elision could
/// most easily have broken, because the file and the blob stopped agreeing.
#[test]
fn every_turn_reads_back_the_stored_line_at_its_recorded_range() {
    for mode in MODES {
        let bench = bench(mode);
        bench.ingest();
        let conn = bench.conn();
        let stream = bench.stream();

        let turns = bench.turns();
        assert_eq!(turns.len(), 6, "{mode:?}: every fixture record is a turn");

        let mut marked = 0;
        for (id, offset, len) in turns {
            let (bytes, _blocks) = testkit::read_turn(&conn, id);
            assert_eq!(bytes.len(), len as usize, "{mode:?}: turn {id} byte_len");
            assert!(
                bytes == stream[offset as usize..(offset + len) as usize],
                "{mode:?}: turn {id} does not sit where its row says"
            );

            let object = as_object(&bytes);
            let elided: Vec<u64> = ELIDED_FIELDS
                .iter()
                .filter_map(|field| object.get(*field))
                .filter_map(capture::elided_bytes)
                .collect();
            marked += elided.len();
            for bytes_elided in elided {
                assert!(bytes_elided > 0, "{mode:?}: a mark with no size");
            }
        }
        let expected = match mode {
            CaptureMode::Full => 0,
            CaptureMode::Lean => 2,
            CaptureMode::Minimal => 4,
        };
        assert_eq!(
            marked, expected,
            "{mode:?}: wrong number of elided subtrees"
        );
    }
}

/// The recall path, not just the raw blob: `recall::get` hands back the stored
/// line for a turn in an elided store, and it is the same bytes the row claims.
#[test]
fn recall_get_returns_the_stored_line_from_an_elided_store() {
    let bench = bench(CaptureMode::Minimal);
    bench.ingest();
    let conn = bench.conn();
    let ids: Vec<i64> = bench.turns().into_iter().map(|(id, _, _)| id).collect();

    let scope = recall::scope::Scope::Everything;
    let fetched = recall::get::records(&conn, &Config::default(), &scope, &ids).unwrap();
    assert_eq!(fetched.records.len(), ids.len(), "{:?}", fetched.absent);

    for record in &fetched.records {
        assert!(!record.body_evicted, "nothing here is evicted");
        let body = record.body.as_ref().expect("a stored body");
        let (direct, _) = testkit::read_turn(&conn, record.turn_id);
        assert!(body == &direct, "recall disagreed with the turn row");
        // And it is JSON, which is what an elided line still has to be.
        as_object(body);
    }
}

/// (c) An elided store is a HEALTHY store. `verify` checksums what is stored,
/// so a well-formed elided stream has nothing wrong with it - and if this ever
/// reported a finding, retention's evicted-session arm and a genuinely corrupt
/// blob would become indistinguishable.
#[test]
fn verify_finds_nothing_wrong_with_an_elided_store() {
    for mode in MODES {
        let bench = bench(mode);
        bench.ingest();
        let store = Store::open(&bench.data_dir).unwrap();
        let report = verify::verify(&store).unwrap();
        assert!(
            report.failures.is_empty(),
            "{mode:?}: {:?}",
            report.failures
        );
    }
}

/// (d) A rebuild derives the same rows from the stored bytes alone - which is
/// STOR-04's claim, now about a stream that is not the transcript.
#[test]
fn a_reindex_of_an_elided_store_reproduces_every_derived_row() {
    let bench = bench(CaptureMode::Lean);
    bench.ingest();

    let before_turns = bench.turns();
    let before_queries = testkit::query_set_json(&bench.conn());
    let before_digest = testkit::archive_digest(&bench.conn());

    let mut store = Store::open(&bench.data_dir).unwrap();
    let rebuilt = reindex::reindex(&mut store).unwrap();
    assert!(rebuilt.failed.is_empty(), "{:?}", rebuilt.failed);
    assert_eq!(rebuilt.sessions, 1);
    assert_eq!(rebuilt.turns, before_turns.len());
    drop(store);

    assert_eq!(
        bench.turns(),
        before_turns,
        "the rebuild moved a turn id, offset or length"
    );
    assert_eq!(
        testkit::query_set_json(&bench.conn()),
        before_queries,
        "the rebuild changed what the index answers"
    );
    assert_eq!(
        testkit::archive_digest(&bench.conn()),
        before_digest,
        "the rebuild touched the archive"
    );
}
