//! Scanning a transcript into records, turns and a resume offset.
//!
//! The three decisions under test are D-03 (what is a turn), D-02 (turn order
//! comes from bytes, never from the clock) and D-14 (the resume offset stops at
//! the last newline). D-12's tolerance is here too: a `.jsonl` in the tree that
//! is not a transcript must scan without erroring.

#![cfg(feature = "testkit")]

use terminus_core::parse::{self, TURN_TYPES};
use terminus_core::testkit::{self, Rng};

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
        let kind = record
            .record_type
            .as_deref()
            .expect("every line has a type");
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

    assert_eq!(
        turns[0].1.parent_uuid, None,
        "the first turn threads nothing"
    );
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

/// D-21 + D-08. The boundary line carries its subtype and the raw bytes of its
/// `compactMetadata`, and it is an ordinary turn - `system` is already a turn
/// type and the record carries both identity fields, so no classification rule
/// changed to admit it.
///
/// "Verbatim" is checked two ways at once, because either alone is weak: the
/// stored bytes must be a *contiguous slice of the line itself* (so nothing was
/// re-serialized) and must deserialize to the same value the whole line's
/// `compactMetadata` deserializes to (so the right slice was taken). Neither
/// check knows where in the line the field sits, so the fixture is free to keep
/// the real record's key order.
#[test]
fn the_boundary_line_carries_its_subtype_and_its_metadata_bytes() {
    let line = testkit::boundary_line();
    let scan = parse::scan(&[line.as_slice(), b"\n"].concat());
    assert_eq!(scan.records.len(), 1);
    let record = &scan.records[0];

    assert_eq!(record.record_type.as_deref(), Some("system"));
    assert_eq!(record.subtype.as_deref(), Some(parse::COMPACT_BOUNDARY));
    assert!(record.is_compact_boundary());
    assert!(record.is_turn(), "D-03 already classifies the boundary");

    let bytes = record
        .compact_metadata
        .as_deref()
        .expect("the boundary carries compaction metadata");
    assert!(
        line.windows(bytes.len()).any(|w| w == bytes),
        "the stored metadata is not a slice of the line, so it was re-serialized"
    );

    let whole: serde_json::Value = serde_json::from_slice(&line).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        stored, whole["compactMetadata"],
        "a different slice was taken"
    );

    // The measured relation D-08 rests on: the preserved uuids are a PROPER
    // subset of all of them, so the lists describe what survived and cannot
    // enumerate what was dropped.
    let preserved: std::collections::BTreeSet<&str> = stored["preservedMessages"]["uuids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let all: std::collections::BTreeSet<&str> = stored["preservedMessages"]["allUuids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(preserved.is_subset(&all) && preserved.len() < all.len());
    for field in ["preTokens", "postTokens", "cumulativeDroppedTokens"] {
        assert!(stored[field].is_number(), "{field} missing from {stored}");
    }
}

/// The other half of the same change: a record with neither field is unchanged.
/// Every phase 1 assertion compares whole `Record` values, so a subtype or a
/// metadata blob appearing where the line has none would break them all.
///
/// `subtype` is read off whatever line carries one, and a phase 1 fixture
/// already does: `session-basic.jsonl` holds a `system` record with
/// `subtype: "local_command_output"`, exactly as real transcripts do. So the
/// property is that no fixture but the compacted one produces a *compaction*
/// signal, and that a line carrying neither field produces neither.
#[test]
fn only_the_compacted_fixture_produces_a_compaction_signal() {
    let mut lines_with_no_subtype = 0usize;
    for fixture in testkit::TRANSCRIPT_FIXTURES {
        if *fixture == testkit::COMPACTED_FIXTURE {
            continue;
        }
        let scan = parse::scan(&testkit::fixture_bytes(fixture));
        for record in &scan.records {
            assert_eq!(record.compact_metadata, None, "{fixture}: {record:?}");
            assert!(!record.is_compact_boundary(), "{fixture}: {record:?}");
            if record.subtype.is_none() {
                lines_with_no_subtype += 1;
            }
        }
    }
    assert!(
        lines_with_no_subtype > 0,
        "no fixture line exercises the neither-field case"
    );

    // The one real subtype the phase 1 corpus carries, still read as itself.
    let basic = parse::scan(&testkit::fixture_bytes("session-basic.jsonl"));
    let subtypes: Vec<&str> = basic
        .records
        .iter()
        .filter_map(|r| r.subtype.as_deref())
        .collect();
    assert_eq!(subtypes, ["local_command_output"]);
}

/// The metadata is found structurally, not by searching for the field name: a
/// line whose message text merely quotes `compactMetadata` has none.
#[test]
fn a_nested_mention_of_the_field_name_is_not_the_field() {
    let line = br#"{"type":"user","uuid":"u1","timestamp":"t","message":{"role":"user","content":[{"type":"text","text":"what is \"compactMetadata\":{\"preTokens\":1} for?"}]},"nested":{"compactMetadata":{"preTokens":2}}}"#;
    let scan = parse::scan(&[line.as_slice(), b"\n"].concat());
    assert_eq!(scan.records.len(), 1);
    assert_eq!(scan.records[0].compact_metadata, None);
    assert!(scan.records[0].is_turn());
}

// ---------------------------------------------------------------------------
// INJ-07 (v0.1.1 phase 1): did the PERSON type this `user` record?
//
// D-01/D-05/D-06 bind the rule to the record's own `message.content` blocks -
// never the top-level `toolUseResult`, whose shape is a dict on 6,265 records,
// a string on 522 and a list on 35 - and D-02 binds its meaning to "authored by
// the person", which folds `isMeta` caveats and the harness envelopes in beside
// tool results. D-07 is the consequence: reading message content and nothing
// else is what makes a `lean` or `minimal` ingest agree with a `full` one.

/// The envelope tags measured on 2026-08-23 over 400 top-level transcripts.
///
/// Spelled here as well as in the parser on purpose: a test that iterated the
/// parser's own list would pass just as happily after a tag was deleted from
/// it.
const ENVELOPE_TAGS: [&str; 6] = [
    "command-message",
    "command-name",
    "local-command-caveat",
    "task-notification",
    "local-command-stdout",
    "bash-stdout",
];

/// One record line, from the fields that matter to the classification.
fn line(fields: serde_json::Value) -> Vec<u8> {
    let mut object = fields.as_object().expect("an object").clone();
    object
        .entry("type")
        .or_insert_with(|| serde_json::json!("user"));
    object.insert("uuid".into(), serde_json::json!("u-1"));
    object.insert(
        "timestamp".into(),
        serde_json::json!("2026-08-23T00:00:00Z"),
    );
    serde_json::to_vec(&serde_json::Value::Object(object)).unwrap()
}

/// What the parser says about one record's authorship.
fn classify(line: &[u8]) -> Option<bool> {
    let bytes = [line, b"\n"].concat();
    let scan = parse::scan(&bytes);
    let (_, turn) = scan.turns().next().expect("the line has to be a turn");
    turn.is_typed
}

/// A user record with the given `message.content`.
fn user(content: serde_json::Value) -> Vec<u8> {
    line(serde_json::json!({ "message": { "role": "user", "content": content } }))
}

/// The two states, and the shape 41% of the corpus's bytes take.
#[test]
fn a_typed_prompt_reads_typed_and_a_tool_result_does_not() {
    assert_eq!(
        classify(&user(serde_json::json!([
            { "type": "text", "text": "where did SearchManager get its retry budget" }
        ]))),
        Some(true)
    );
    // The other content shape a real prompt takes: 1,097 of 7,933 sampled
    // `user` records carry a bare string rather than a block list.
    assert_eq!(
        classify(&user(serde_json::json!(
            "run the build and show me the stderr"
        ))),
        Some(true)
    );
    assert_eq!(
        classify(&user(serde_json::json!([
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
        ]))),
        Some(false)
    );
}

/// D-05: the tool-result test wins over the text one. Both real examples are
/// fork boilerplate in `agent-*.jsonl`.
#[test]
fn a_record_carrying_both_a_text_and_a_tool_result_block_is_not_typed() {
    assert_eq!(
        classify(&user(serde_json::json!([
            { "type": "text", "text": "continuing from a previous conversation" },
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
        ]))),
        Some(false)
    );
}

/// D-02: `isMeta` is the harness talking, whatever the content shape.
#[test]
fn an_is_meta_record_is_not_typed() {
    let mut fields = serde_json::json!({
        "message": { "role": "user", "content": "Caveat: the messages below were generated by ..." },
        "isMeta": true,
    });
    assert_eq!(classify(&line(fields.take())), Some(false));

    // And with a block list, since the two shapes take different branches.
    assert_eq!(
        classify(&line(serde_json::json!({
            "message": { "role": "user", "content": [{ "type": "text", "text": "Caveat: ..." }] },
            "isMeta": true,
        }))),
        Some(false)
    );
}

/// D-02's envelope list, one record per tag, in both content shapes.
#[test]
fn every_harness_envelope_tag_is_not_typed() {
    for tag in ENVELOPE_TAGS {
        let text = format!("<{tag}>/cad-plan is running</{tag}>");
        assert_eq!(
            classify(&user(serde_json::json!(text))),
            Some(false),
            "<{tag}> as a string content"
        );
        assert_eq!(
            classify(&user(serde_json::json!([{ "type": "text", "text": text }]))),
            Some(false),
            "<{tag}> as a text block"
        );
        // Leading whitespace does not hide the tag.
        assert_eq!(
            classify(&user(serde_json::json!(format!("\n  {text}")))),
            Some(false),
            "<{tag}> behind whitespace"
        );
    }

    // `bash-input` is measured (13 records) and deliberately on the typed side:
    // it is the command the person typed after `!`, only wrapped.
    assert_eq!(
        classify(&user(serde_json::json!(
            "<bash-input>cargo test -p terminus-core</bash-input>"
        ))),
        Some(true)
    );
}

/// The LEADING tag, never "the text contains a tag": roughly 200 person-typed
/// prompts in the same sample carry `<objective>` and friends inside them.
#[test]
fn a_prompt_that_merely_contains_a_tag_is_still_typed() {
    for text in [
        "<objective>ship the resume brief</objective>",
        "read this and tell me why <command-name>/cad-plan</command-name> shows up in it",
        "<command-message-of-my-own>not upstream's tag</command-message-of-my-own>",
    ] {
        assert_eq!(
            classify(&user(serde_json::json!(text))),
            Some(true),
            "{text}"
        );
    }
}

/// D-06: the classification never reads `toolUseResult`, which is why its shape
/// cannot mislead it. The record here has the key as a bare STRING - 522 of a
/// 300-file sample do - and no `tool_result` block, so it is typed.
#[test]
fn the_top_level_tool_use_result_is_not_what_is_read() {
    assert_eq!(
        classify(&line(serde_json::json!({
            "message": { "role": "user", "content": [{ "type": "text", "text": "a prompt" }] },
            "toolUseResult": "a bare string, not an object",
        }))),
        Some(true)
    );
    // And the converse: the block decides even when the key is absent, which is
    // the 282 sidecar records D-01 measured.
    assert_eq!(
        classify(&user(serde_json::json!([
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
        ]))),
        Some(false)
    );
}

/// D-04: the question is asked of `user` records and of no other type, so
/// nothing else carries a value at all.
#[test]
fn only_a_user_record_carries_a_classification() {
    for kind in ["assistant", "attachment", "system"] {
        assert_eq!(
            classify(&line(serde_json::json!({
                "type": kind,
                "message": { "content": [{ "type": "text", "text": "not a prompt" }] },
            }))),
            None,
            "{kind} must carry no classification"
        );
    }

    // Every turn of a real fixture, so the claim covers what ingest actually
    // sees rather than three constructed lines.
    let bytes = testkit::fixture_bytes("session-basic.jsonl");
    let scan = parse::scan(&bytes);
    let mut users = 0;
    for (_, turn) in scan.turns() {
        if turn.record_type == "user" {
            users += 1;
            assert!(
                turn.is_typed.is_some(),
                "a user turn with no classification"
            );
        } else {
            assert_eq!(turn.is_typed, None, "{} carries one", turn.record_type);
        }
    }
    assert!(users > 0, "the fixture has to hold a user turn");
}

/// D-07: `[capture]` elision replaces `toolUseResult` and `attachment` and
/// never touches `message.content`, so a reduced ingest and a later blob-only
/// rebuild classify the same bytes the same way.
#[test]
fn elision_does_not_change_the_classification() {
    use terminus_core::capture::{self, ELISION_MARK};
    use terminus_core::config::CaptureMode;

    let source = line(serde_json::json!({
        "message": { "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "see toolUseResult" }
        ]},
        "toolUseResult": { "stdout": "x".repeat(64), "stderr": "" },
    }));
    assert_eq!(classify(&source), Some(false));

    for mode in [CaptureMode::Lean, CaptureMode::Minimal] {
        let elided = capture::elide(&source, mode);
        assert_eq!(classify(&elided), Some(false), "{mode:?}");
        if mode == CaptureMode::Minimal {
            // The premise: elision really happened, or this compares a line
            // with itself.
            assert!(
                String::from_utf8_lossy(&elided).contains(ELISION_MARK),
                "the minimal mode elided nothing"
            );
        }
    }

    // And the typed side survives it too: a prompt carrying an attachment.
    let typed = line(serde_json::json!({
        "message": { "role": "user", "content": [{ "type": "text", "text": "a prompt" }] },
        "attachment": { "content": "y".repeat(64) },
    }));
    assert_eq!(classify(&typed), Some(true));
    assert_eq!(
        classify(&capture::elide(&typed, CaptureMode::Minimal)),
        Some(true)
    );
}
