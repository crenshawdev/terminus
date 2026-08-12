//! Scanning a transcript into records, turns and a resume offset.
//!
//! The three decisions under test are D-03 (what is a turn), D-02 (turn order
//! comes from bytes, never from the clock) and D-14 (the resume offset stops at
//! the last newline). D-12's tolerance is here too: a `.jsonl` in the tree that
//! is not a transcript must scan without erroring.

#![cfg(feature = "testkit")]

use verbatim_core::parse::{self, TURN_TYPES};
use verbatim_core::testkit::{self, Rng};

/// The eleven record types that carry no turn (D-03).
const STATE_TYPES: [&str; 11] = [
    "last-prompt",
    "ai-title",
    "mode",
    "permission-mode",
    "file-history-snapshot",
    "queue-operation",
    "bridge-session",
    "file-history-delta",
    "agent-name",
    "agent-setting",
    "frame-link",
];

#[test]
fn only_the_four_turn_types_become_turns() {
    let bytes = testkit::fixture_bytes("session-basic.jsonl");
    let scan = parse::scan(&bytes);

    // Every line is a record; only some are turns.
    assert_eq!(scan.records.len(), 19, "one record per line of the fixture");

    for record in &scan.records {
        let kind = record.record_type.as_deref().expect("every line has a type");
        if record.is_turn() {
            assert!(
                TURN_TYPES.contains(&kind),
                "{kind} became a turn but is not a turn type"
            );
        } else {
            assert!(
                STATE_TYPES.contains(&kind),
                "{kind} produced no turn but is not a state type"
            );
        }
    }

    // All four turn types are exercised, and all eleven state types are seen.
    let mut turn_kinds: Vec<&str> = scan
        .turns()
        .map(|(_, t)| t.record_type.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    turn_kinds.sort_unstable();
    let mut expected = TURN_TYPES.to_vec();
    expected.sort_unstable();
    assert_eq!(turn_kinds, expected);

    let state_kinds: std::collections::BTreeSet<&str> = scan
        .records
        .iter()
        .filter(|r| !r.is_turn())
        .map(|r| r.record_type.as_deref().unwrap())
        .collect();
    assert_eq!(state_kinds.len(), 11, "all eleven state types are present");
    for kind in STATE_TYPES {
        assert!(state_kinds.contains(kind), "{kind} missing from the scan");
    }
}

/// D-02. Two adjacent turns in the fixture have decreasing timestamps; byte
/// order decides `turn_seq`, so the earlier-in-file record keeps the lower one.
#[test]
fn turn_seq_follows_byte_order_even_when_the_clock_goes_backwards() {
    let bytes = testkit::fixture_bytes("session-basic.jsonl");
    let scan = parse::scan(&bytes);

    let turns: Vec<_> = scan.turns().map(|(r, t)| (r.offset, t.clone())).collect();
    assert!(turns.len() >= 2);

    // turn_seq is dense, starts at 0, and increases with the byte offset.
    for (index, (offset, turn)) in turns.iter().enumerate() {
        assert_eq!(turn.turn_seq, index as i64, "turn_seq must be dense");
        if index > 0 {
            assert!(*offset > turns[index - 1].0, "turns must be in byte order");
        }
    }

    // The pair the fixture exists to pin: a later record with an earlier clock.
    let decreasing: Vec<_> = turns
        .windows(2)
        .filter(|w| w[1].1.timestamp < w[0].1.timestamp)
        .collect();
    assert_eq!(
        decreasing.len(),
        1,
        "the fixture carries exactly one backwards pair"
    );
    let pair = decreasing[0];
    assert_eq!(
        pair[1].1.turn_seq,
        pair[0].1.turn_seq + 1,
        "the later-in-file record keeps the higher turn_seq despite the earlier timestamp"
    );
}

/// The fields later tasks read, taken off the fixture rather than asserted in
/// the abstract: `parentUuid` chains, `sessionId` is the file's own, and the
/// one `tool_use` record reports its tool.
#[test]
fn a_turn_carries_the_fields_ingest_and_derive_need() {
    let bytes = testkit::fixture_bytes("session-basic.jsonl");
    let scan = parse::scan(&bytes);
    let turns: Vec<_> = scan.turns().collect();

    assert_eq!(turns[0].1.parent_uuid, None, "the first turn threads nothing");
    for window in turns.windows(2) {
        assert_eq!(
            window[1].1.parent_uuid.as_deref(),
            Some(window[0].1.uuid.as_str()),
            "the fixture's turns form one parentUuid chain"
        );
    }

    let tools: Vec<&str> = turns
        .iter()
        .filter_map(|(_, t)| t.tool_name.as_deref())
        .collect();
    assert_eq!(tools, vec!["Bash"], "one tool_use record in the fixture");

    for (record, _) in &turns {
        assert_eq!(
            record.session_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(record.cwd.as_deref(), Some("/data/code/verbatim"));
        assert_eq!(record.git_branch.as_deref(), Some("restart"));
        // D-11: this fixture continues nothing.
        assert_eq!(record.foreign_session_id, None);
    }
}

/// D-11: the file-level lineage signal is the snake_case `session_id`, and it
/// is not the record's own `sessionId`.
#[test]
fn a_continuation_reports_a_foreign_session_id_distinct_from_its_own() {
    let bytes = testkit::fixture_bytes("session-continuation.jsonl");
    let scan = parse::scan(&bytes);
    assert!(scan.turn_count() > 0);

    for record in &scan.records {
        assert_eq!(
            record.foreign_session_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(
            record.session_id.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
    }
}

/// D-12. `journal.jsonl` is valid JSON with none of the transcript identity
/// fields. It yields records, no turns, and no error of any kind.
#[test]
fn a_non_transcript_jsonl_yields_no_turns_and_no_error() {
    let bytes = testkit::fixture_bytes(testkit::NON_TRANSCRIPT_FIXTURE);
    let scan = parse::scan(&bytes);

    assert_eq!(scan.records.len(), 3);
    assert_eq!(scan.turn_count(), 0);
    assert_eq!(scan.resume_offset, bytes.len() as u64);
    for record in &scan.records {
        assert!(!record.is_turn());
        assert_eq!(record.record_type.as_deref(), Some("journal"));
        assert_eq!(record.session_id, None);
    }
}

/// A line that is not JSON at all is tolerated: the scan continues, the record
/// is still accounted for, and the bytes are still in the range the blob takes.
#[test]
fn an_unparseable_line_does_not_abort_the_scan() {
    let mut bytes = testkit::fixture_bytes("session-basic.jsonl");
    let good = parse::scan(&bytes).turn_count();
    bytes.extend_from_slice(b"this is not json at all\n");
    bytes.extend_from_slice(b"{\"type\":\"user\",\"uuid\":\"u\",\"timestamp\":\"t\"}\n");

    let scan = parse::scan(&bytes);
    assert_eq!(scan.resume_offset, bytes.len() as u64);
    assert_eq!(
        scan.turn_count(),
        good + 1,
        "the record after the bad line is still classified"
    );
    let bad = &scan.records[scan.records.len() - 2];
    assert!(!bad.is_turn());
    assert_eq!(bad.record_type, None);
    assert_eq!(bad.len, "this is not json at all".len() as u64);
}

/// D-03 requires *both* identity fields. A turn-typed record missing either one
/// is not a turn, and the fixtures cannot show that on their own: every
/// turn-typed record in them carries both.
#[test]
fn a_turn_type_without_both_identity_fields_is_not_a_turn() {
    let bytes = concat!(
        r#"{"type":"user","uuid":"u1","timestamp":"2026-01-01T00:00:00Z"}"#,
        "\n",
        r#"{"type":"user","uuid":"u2"}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-01-01T00:00:01Z"}"#,
        "\n",
        r#"{"type":"user","uuid":null,"timestamp":"2026-01-01T00:00:02Z"}"#,
        "\n",
    );
    let scan = parse::scan(bytes.as_bytes());

    assert_eq!(scan.records.len(), 4);
    assert_eq!(scan.turn_count(), 1, "only the complete record is a turn");
    assert!(scan.records[0].is_turn());
    for record in &scan.records[1..] {
        assert!(
            !record.is_turn(),
            "half an identity made a turn: {record:?}"
        );
    }
}

/// D-14. The one fixture with no trailing newline: the resume offset stops at
/// the last complete record and the partial one is left for the next pass.
#[test]
fn the_resume_offset_stops_at_the_last_newline_not_at_end_of_file() {
    let bytes = testkit::fixture_bytes(testkit::TRUNCATED_FIXTURE);
    let scan = parse::scan(&bytes);

    let expected = bytes.iter().rposition(|b| *b == b'\n').unwrap() as u64 + 1;
    assert_eq!(scan.resume_offset, expected);
    assert!(
        scan.resume_offset < bytes.len() as u64,
        "the fixture is cut mid-record, so the resume offset is short of the file"
    );
    assert_eq!(bytes[scan.resume_offset as usize - 1], b'\n');

    // No record covers the partial tail.
    let last = scan.records.last().unwrap();
    assert_eq!(last.offset + last.len + 1, scan.resume_offset);
}

/// A range holding no newline makes no progress at all, rather than archiving
/// half a record.
#[test]
fn a_range_with_no_newline_resumes_where_it_started() {
    let scan = parse::scan_from(b"{\"type\":\"user\"", 4096, 7);
    assert_eq!(scan.resume_offset, 4096);
    assert!(scan.records.is_empty());
}

/// A tail read numbers its turns on from the ones already stored, and the
/// offsets it reports are absolute stream coordinates.
#[test]
fn a_tail_scan_continues_the_turn_numbering_and_the_offsets() {
    let bytes = testkit::fixture_bytes("session-basic.jsonl");
    let cut = testkit::fixture_bytes(testkit::TRUNCATED_FIXTURE).len() as u64;
    let head = parse::scan(&bytes[..cut as usize]);
    let tail = parse::scan_from(
        &bytes[head.resume_offset as usize..],
        head.resume_offset,
        head.turn_count() as i64,
    );

    let whole = parse::scan(&bytes);
    let joined: Vec<_> = head
        .records
        .iter()
        .chain(tail.records.iter())
        .cloned()
        .collect();
    assert_eq!(joined, whole.records, "head + tail equals a single pass");
    assert_eq!(tail.resume_offset, bytes.len() as u64);
}

/// D-14 as a property, over 200 pseudorandom cuts. Whatever the truncation
/// point, the resume offset lands immediately after a `\n` at or before it, and
/// what was parsed is a prefix of the complete file's parse.
#[test]
fn truncating_anywhere_resumes_after_a_newline_and_parses_a_prefix() {
    let bytes = testkit::fixture_bytes("session-basic.jsonl");
    let whole = parse::scan(&bytes);
    let mut rng = Rng(testkit::seed(
        "truncating_anywhere_resumes_after_a_newline_and_parses_a_prefix",
    ));

    for _ in 0..200 {
        let cut = rng.below(bytes.len() as u64 + 1) as usize;
        let scan = parse::scan(&bytes[..cut]);

        assert!(
            scan.resume_offset <= cut as u64,
            "resume {} ran past the cut at {cut}",
            scan.resume_offset
        );
        if scan.resume_offset == 0 {
            assert!(
                !bytes[..cut].contains(&b'\n'),
                "resumed at 0 with a newline available before {cut}"
            );
        } else {
            assert_eq!(
                bytes[scan.resume_offset as usize - 1],
                b'\n',
                "resume offset must land immediately after a newline"
            );
            assert!(
                !bytes[scan.resume_offset as usize..cut].contains(&b'\n'),
                "resume offset must be the LAST newline at or before the cut"
            );
        }

        assert_eq!(
            scan.records[..],
            whole.records[..scan.records.len()],
            "a truncated parse must be a prefix of the whole file's parse"
        );
    }
}
