//! The fixture corpus is byte-load-bearing, so it gets its own tests.
//!
//! Every later test in this phase reads these files and asserts on their exact
//! bytes. These assertions pin the properties CONTEXT measured against the real
//! corpus, so a hand edit that quietly drops one fails here rather than three
//! tasks downstream.

#![cfg(feature = "testkit")]

use std::collections::BTreeSet;
use verbatim_core::testkit;

/// Every fixture, transcript or not.
fn all_fixtures() -> Vec<&'static str> {
    let mut v = testkit::TRANSCRIPT_FIXTURES.to_vec();
    v.push(testkit::NON_TRANSCRIPT_FIXTURE);
    v
}

fn records(name: &str) -> Vec<serde_json::Value> {
    let bytes = testkit::fixture_bytes(name);
    let text = String::from_utf8(bytes).expect("fixtures are UTF-8");
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{name}: bad JSON line: {e}")))
        .collect()
}

/// D-14: the watermark is the byte just past the last `\n`, so a complete
/// transcript ends with one. `session-truncated.jsonl` is the exception that
/// makes the rule testable. No fixture carries a `\r`.
#[test]
fn line_endings_match_the_measured_corpus() {
    for name in all_fixtures() {
        let bytes = testkit::fixture_bytes(name);
        assert!(!bytes.is_empty(), "{name} is empty");
        assert!(
            !bytes.contains(&0x0d),
            "{name} contains a CR; zero were found in the real corpus"
        );
        if name == testkit::TRUNCATED_FIXTURE {
            assert_ne!(
                bytes[bytes.len() - 1],
                0x0a,
                "{name} must be cut mid-record, with no trailing newline"
            );
        } else {
            assert_eq!(
                bytes[bytes.len() - 1],
                0x0a,
                "{name} must end with a newline"
            );
        }
    }
}

/// D-03: fifteen record types, four of which are turns.
#[test]
fn session_basic_carries_all_fifteen_record_types() {
    const TURN_TYPES: [&str; 4] = ["user", "assistant", "attachment", "system"];
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

    let seen: BTreeSet<String> = records("session-basic.jsonl")
        .iter()
        .map(|r| {
            r["type"]
                .as_str()
                .expect("every record has a type")
                .to_owned()
        })
        .collect();

    let expected: BTreeSet<String> = TURN_TYPES
        .iter()
        .chain(STATE_TYPES.iter())
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(seen, expected);
    assert_eq!(seen.len(), 15);
}

/// D-03 again, from the other side: a state record may carry one of the two
/// identity fields, and a classifier keying on either alone would be wrong.
#[test]
fn state_records_never_carry_both_identity_fields() {
    const TURN_TYPES: [&str; 4] = ["user", "assistant", "attachment", "system"];
    let mut uuid_only = 0;
    let mut timestamp_only = 0;

    for r in records("session-basic.jsonl") {
        let kind = r["type"].as_str().unwrap();
        let has_uuid = r.get("uuid").is_some();
        let has_ts = r.get("timestamp").is_some();
        if TURN_TYPES.contains(&kind) {
            assert!(has_uuid && has_ts, "turn record {kind} lacks turn identity");
        } else {
            assert!(
                !(has_uuid && has_ts),
                "state record {kind} carries both uuid and timestamp"
            );
            if has_uuid {
                uuid_only += 1;
            }
            if has_ts {
                timestamp_only += 1;
            }
        }
    }

    assert!(
        uuid_only >= 1,
        "no state record exercises uuid-without-timestamp"
    );
    assert!(
        timestamp_only >= 1,
        "no state record exercises timestamp-without-uuid"
    );
}

/// D-02: 31 of 62 sampled real transcripts contain an out-of-order timestamp,
/// so `turn_seq` must come from byte order. The fixture reproduces one.
#[test]
fn session_basic_has_an_adjacent_turn_pair_whose_timestamps_decrease() {
    const TURN_TYPES: [&str; 4] = ["user", "assistant", "attachment", "system"];
    let turns: Vec<serde_json::Value> = records("session-basic.jsonl")
        .into_iter()
        .filter(|r| {
            r.get("uuid").is_some()
                && r.get("timestamp").is_some()
                && TURN_TYPES.contains(&r["type"].as_str().unwrap())
        })
        .collect();

    let decreasing = turns
        .windows(2)
        .filter(|w| w[1]["timestamp"].as_str().unwrap() < w[0]["timestamp"].as_str().unwrap())
        .count();
    assert!(
        decreasing >= 1,
        "no adjacent turn pair decreases in timestamp"
    );
}

/// D-05: 28 of 6,731 sampled records exceed one 64 KB block.
#[test]
fn session_large_record_has_exactly_one_oversized_line() {
    let bytes = testkit::fixture_bytes("session-large-record.jsonl");
    let oversized: Vec<usize> = bytes
        .split(|b| *b == b'\n')
        .enumerate()
        .filter(|(_, line)| line.len() > 65536)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(oversized, vec![0], "the big record must be the first line");

    // Offset 0 plus this length is exactly four 64 KB blocks: three full plus a
    // partial. At a non-zero offset the same record would straddle a fifth.
    let first = bytes.split(|b| *b == b'\n').next().unwrap();
    assert_eq!(first.len().div_ceil(65536), 4);
}

/// D-12: `journal.jsonl` is not a transcript. Discovery reaches it, so nothing
/// downstream may assume transcript shape from the file extension.
#[test]
fn journal_records_carry_no_transcript_identity() {
    let recs = records(testkit::NON_TRANSCRIPT_FIXTURE);
    assert!(!recs.is_empty());
    for r in recs {
        for field in ["sessionId", "uuid", "timestamp", "cwd"] {
            assert!(
                r.get(field).is_none(),
                "journal record carries {field}: {r}"
            );
        }
        for field in ["agentId", "key", "result", "type"] {
            assert!(r.get(field).is_some(), "journal record lacks {field}: {r}");
        }
    }
}

/// D-01: this is why `sessions` is keyed on file identity. Every sidecar record
/// reports the *parent's* session id.
#[test]
fn sidecar_reports_the_parent_session_id() {
    let parent = records("session-basic.jsonl")[0]["sessionId"]
        .as_str()
        .expect("session-basic records carry a sessionId")
        .to_owned();

    let recs = records("subagents/agent-alpha.jsonl");
    assert!(!recs.is_empty());
    for r in recs {
        assert_eq!(r["sessionId"].as_str(), Some(parent.as_str()));
        assert_eq!(r["isSidechain"], serde_json::Value::Bool(true));
    }
}

/// D-11: `session_id` (foreign, file-level) and `sessionId` (own) are two
/// different signals and the fixture keeps them distinguishable.
#[test]
fn continuation_names_its_predecessor_through_a_foreign_session_id() {
    let parent = records("session-basic.jsonl")[0]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();

    let recs = records("session-continuation.jsonl");
    assert!(!recs.is_empty());
    for r in &recs {
        assert_eq!(r["session_id"].as_str(), Some(parent.as_str()));
        assert_ne!(r["sessionId"].as_str(), Some(parent.as_str()));
    }
}

/// D-14 / ING-01: the truncated fixture is a byte prefix of the complete one,
/// which is what makes "ingest the partial file, then the rest" testable.
#[test]
fn truncated_fixture_is_a_byte_prefix_cut_mid_record() {
    let full = testkit::fixture_bytes("session-basic.jsonl");
    let cut = testkit::fixture_bytes(testkit::TRUNCATED_FIXTURE);
    assert!(cut.len() < full.len());
    assert_eq!(&full[..cut.len()], &cut[..]);

    let resume = cut
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .expect("the cut keeps whole records before the partial one");
    assert!(
        resume < cut.len(),
        "the cut must land mid-record, not on a record boundary"
    );
}

/// One known-unique token, so a search test can assert on a single hit.
#[test]
fn unique_token_appears_exactly_once_across_the_corpus() {
    let total: usize = all_fixtures()
        .iter()
        .map(|name| {
            let bytes = testkit::fixture_bytes(name);
            String::from_utf8(bytes)
                .unwrap()
                .matches(testkit::UNIQUE_TOKEN)
                .count()
        })
        .sum();
    assert_eq!(total, 1, "`{}` must be unique", testkit::UNIQUE_TOKEN);
}

/// D-04: the meta fixture is one JSON object carrying the five fields every
/// real `agent-*.meta.json` carries, and it is not a transcript.
///
/// It sits outside [`testkit::TRANSCRIPT_FIXTURES`] on purpose: the JSONL and
/// trailing-newline assertions above do not describe it, and discovery's
/// filename filter must keep it out of the archive entirely.
#[test]
fn the_agent_meta_fixture_is_one_json_object_and_not_a_transcript() {
    assert!(
        !testkit::TRANSCRIPT_FIXTURES.contains(&testkit::AGENT_META_FIXTURE),
        "the meta file is not a transcript"
    );
    assert!(
        testkit::AGENT_META_FIXTURE.ends_with(".meta.json"),
        "the extension is what keeps discovery from picking it up"
    );

    let bytes = testkit::fixture_bytes(testkit::AGENT_META_FIXTURE);
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("one JSON object");
    for field in [
        "agentType",
        "description",
        "toolUseId",
        "spawnDepth",
        "model",
    ] {
        assert!(
            value.get(field).is_some(),
            "meta file lacks {field}: {value}"
        );
    }

    // Its stem matches the sidecar it belongs to, which is how ingest finds it.
    let sidecar = testkit::AGENT_META_FIXTURE.replace(".meta.json", ".jsonl");
    assert!(
        testkit::TRANSCRIPT_FIXTURES.contains(&sidecar.as_str()),
        "the meta file must sit beside a sidecar fixture"
    );
}
