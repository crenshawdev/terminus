//! The fixture corpus is byte-load-bearing, so it gets its own tests.
//!
//! Every later test in this phase reads these files and asserts on their exact
//! bytes. These assertions pin the properties CONTEXT measured against the real
//! corpus, so a hand edit that quietly drops one fails here rather than three
//! tasks downstream.

#![cfg(feature = "testkit")]

use std::collections::BTreeSet;
use terminus_core::testkit;

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

/// D-21 + D-08: the compacted fixture ends with a real-shaped boundary record.
///
/// Shape, position and the subset relation are all load-bearing. Two later
/// tests append this file's **last line** to another transcript, so a record
/// added after it would silently make them append the wrong bytes.
#[test]
fn the_compacted_fixture_ends_with_a_real_shaped_boundary() {
    let recs = records(testkit::COMPACTED_FIXTURE);
    assert!(recs.len() >= 2, "a boundary needs turns to come after");

    let boundary = recs.last().unwrap();
    assert_eq!(boundary["type"], "system");
    assert_eq!(boundary["subtype"], "compact_boundary");
    for field in ["uuid", "timestamp", "logicalParentUuid", "compactMetadata"] {
        assert!(boundary.get(field).is_some(), "boundary lacks {field}");
    }
    // No earlier record is one, so "the last line" is unambiguous.
    for r in &recs[..recs.len() - 1] {
        assert_ne!(r["subtype"], "compact_boundary");
    }

    let meta = &boundary["compactMetadata"];
    for field in ["preTokens", "postTokens", "cumulativeDroppedTokens"] {
        assert!(meta[field].is_number(), "compactMetadata lacks {field}");
    }

    // The measured fact D-08 rests on: 6 preserved uuids of 8, against 38,064
    // dropped tokens - the lists describe what survived, so no dropped-turn set
    // can be computed from them.
    let uuids: BTreeSet<&str> = meta["preservedMessages"]["uuids"]
        .as_array()
        .expect("preservedMessages.uuids is a list")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let all: BTreeSet<&str> = meta["preservedMessages"]["allUuids"]
        .as_array()
        .expect("preservedMessages.allUuids is a list")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(uuids.is_subset(&all), "uuids is not a subset of allUuids");
    assert!(uuids.len() < all.len(), "the subset must be proper");

    // `compactMetadata` is not the last key, so nothing may extract it by
    // position, and the fixture keeps the real record's key order.
    let text = String::from_utf8(testkit::fixture_bytes(testkit::COMPACTED_FIXTURE)).unwrap();
    let last = text.lines().last().unwrap();
    let at = last.find("\"compactMetadata\"").expect("the key is there");
    assert!(
        last[at..].contains("\"uuid\""),
        "compactMetadata must not be the final key"
    );
}

/// [`testkit::boundary_line`] returns exactly that last record, with no
/// newline: it is what the compaction tests append to another transcript.
#[test]
fn the_boundary_line_helper_returns_the_last_whole_record() {
    let line = testkit::boundary_line();
    assert!(!line.contains(&b'\n'), "the helper must return one line");
    let value: serde_json::Value = serde_json::from_slice(&line).expect("one whole record");
    assert_eq!(value["subtype"], "compact_boundary");

    let bytes = testkit::fixture_bytes(testkit::COMPACTED_FIXTURE);
    assert!(bytes.ends_with(&[line.as_slice(), b"\n"].concat()));
}

// --- Phase 3 -----------------------------------------------------------------

/// Concatenate a record's `message.content` text blocks, which is all a phase 3
/// assertion ever needs off a turn.
fn message_text(record: &serde_json::Value) -> String {
    let Some(blocks) = record["message"]["content"].as_array() else {
        return record["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    };
    blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The phase 3 fixtures carry a root the test owns rather than this checkout's
/// path, and they carry two projects between them.
///
/// The phase 1 and 2 fixtures hardcode `/data/code/verbatim`. A project-scoped
/// assertion over those is true only on a checkout at that literal path, and on
/// that checkout it is true for the wrong reason: the directory exists, so
/// `git rev-parse` answers and the key is whatever the developer's tree is.
#[test]
fn the_rooted_fixtures_name_a_test_owned_root_and_two_projects() {
    let mut projects = BTreeSet::new();
    for (name, project) in testkit::ROOTED_FIXTURES {
        assert!(
            testkit::TRANSCRIPT_FIXTURES.contains(name),
            "{name} is rooted but not registered as a transcript"
        );
        projects.insert(*project);

        let expected = format!("{}/{project}", testkit::FIXTURE_ROOT_TOKEN);
        let recs = records(name);
        assert!(!recs.is_empty());
        for r in &recs {
            assert_eq!(
                r["cwd"].as_str(),
                Some(expected.as_str()),
                "{name} carries a cwd that is not the rooted one"
            );
        }
        let text = String::from_utf8(testkit::fixture_bytes(name)).unwrap();
        assert!(
            !text.contains("/data/code/verbatim"),
            "{name} hardcodes this checkout's path"
        );
    }
    assert!(
        projects.len() >= 2,
        "one project between the rooted fixtures leaves scoping nothing to be false about"
    );
}

/// AC1's two probes and AC3's structured-versus-prose pair, all in one fixture.
///
/// The pair is the part that is easy to get wrong by accident: the `Read`
/// `tool_use` and the prose mention must name the **same** path and sit in
/// **different** turns, or the assertion that one emits a `path` entity and the
/// other emits none is comparing a turn with itself.
#[test]
fn session_recall_carries_the_expansion_probes_and_the_prose_pair() {
    let recs = records("session-recall.jsonl");

    let camel: Vec<usize> = recs
        .iter()
        .enumerate()
        .filter(|(_, r)| message_text(r).contains("SearchManager"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(camel.len(), 1, "exactly one turn carries `SearchManager`");

    let pathy: Vec<usize> = recs
        .iter()
        .enumerate()
        .filter(|(_, r)| message_text(r).contains("src/worker/S.ts"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(pathy.len(), 1, "exactly one turn carries `src/worker/S.ts`");

    // The structured half: a `Read` whose `file_path` names the file.
    let mut structured = Vec::new();
    for (index, r) in recs.iter().enumerate() {
        let Some(blocks) = r["message"]["content"].as_array() else {
            continue;
        };
        for block in blocks {
            if block["type"] == "tool_use" && block["name"] == "Read" {
                structured.push((
                    index,
                    block["input"]["file_path"]
                        .as_str()
                        .expect("a Read names a file_path")
                        .to_owned(),
                ));
            }
        }
    }
    assert_eq!(structured.len(), 1, "exactly one Read tool_use");
    let (structured_at, file) = &structured[0];

    // The prose half: the same path, in a turn with no tool_use at all.
    let prose: Vec<usize> = recs
        .iter()
        .enumerate()
        .filter(|(_, r)| message_text(r).contains(file.as_str()))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(prose.len(), 1, "exactly one turn names {file} in prose");
    assert_ne!(
        prose[0], *structured_at,
        "the structured and prose mentions must be different turns"
    );
    assert!(
        recs[prose[0]]["message"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["type"] == "text"),
        "the prose turn must carry no structured block"
    );

    // The symbol probe, from the one field RCL-02 reads symbols out of.
    let patterns: Vec<&str> = recs
        .iter()
        .filter_map(|r| r["message"]["content"].as_array())
        .flatten()
        .filter(|b| b["type"] == "tool_use" && b["name"] == "Grep")
        .filter_map(|b| b["input"]["pattern"].as_str())
        .collect();
    assert_eq!(patterns, ["retryBudget"]);
}

/// Every `tool_result` and `toolUseResult` in the error fixtures, as
/// `(is_error, stderr, interrupted)`.
fn error_results(name: &str) -> Vec<(bool, String, bool)> {
    let mut out = Vec::new();
    for r in records(name) {
        let Some(blocks) = r["message"]["content"].as_array() else {
            continue;
        };
        let Some(block) = blocks.iter().find(|b| b["type"] == "tool_result") else {
            continue;
        };
        let result = &r["toolUseResult"];
        assert!(
            result.is_object(),
            "{name}: a tool_result with no toolUseResult object"
        );
        for key in ["stdout", "stderr", "interrupted"] {
            assert!(
                result.get(key).is_some(),
                "{name}: toolUseResult lacks {key}, which 2,920 real Bash results carry"
            );
        }
        out.push((
            block["is_error"].as_bool().expect("is_error is a bool"),
            result["stderr"]
                .as_str()
                .expect("stderr is a string")
                .into(),
            result["interrupted"].as_bool().expect("a bool"),
        ));
    }
    out
}

fn looks_like_uuid(token: &str) -> bool {
    let widths = [8usize, 4, 4, 4, 12];
    let groups: Vec<&str> = token.split('-').collect();
    groups.len() == widths.len()
        && groups
            .iter()
            .zip(widths)
            .all(|(g, w)| g.len() == w && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn looks_like_timestamp(token: &str) -> bool {
    token.len() == 24
        && token.as_bytes()[10] == b'T'
        && token.ends_with('Z')
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b':' | b'.' | b'T' | b'Z'))
}

fn looks_like_line_col(token: &str) -> bool {
    let mut parts = token.rsplit(':');
    let (col, line) = (parts.next(), parts.next());
    matches!((line, col), (Some(l), Some(c))
        if !l.is_empty() && !c.is_empty()
            && l.bytes().all(|b| b.is_ascii_digit())
            && c.bytes().all(|b| b.is_ascii_digit()))
}

/// AC2's whole premise, pinned in the bytes rather than in the extractor.
///
/// One pair across the two sessions differs *only* in the four things D-04
/// normalizes away, and the other differs *only* in a bare integer D-04
/// deliberately keeps. Neither claim is checkable once the extractor has run:
/// by then both pairs are just values, and a rule that collapsed everything and
/// a rule that collapsed nothing both look plausible.
#[test]
fn the_error_fixtures_hold_one_collapsing_pair_and_one_that_must_not() {
    let a = error_results("session-errors-a.jsonl");
    let b = error_results("session-errors-b.jsonl");
    assert!(!a.is_empty() && !b.is_empty());

    // Pair one: flagged, and differing only in the four variable kinds.
    let flagged = |rows: &[(bool, String, bool)]| -> String {
        let hits: Vec<String> = rows
            .iter()
            .filter(|(is_error, _, _)| *is_error)
            .map(|(_, stderr, _)| stderr.clone())
            .collect();
        assert_eq!(hits.len(), 1, "exactly one is_error result per file");
        hits.into_iter().next().unwrap()
    };
    let (one_a, one_b) = (flagged(&a), flagged(&b));
    assert_ne!(one_a, one_b, "the pair must not be literally identical");

    let (tokens_a, tokens_b): (Vec<&str>, Vec<&str>) = (
        one_a.split_whitespace().collect(),
        one_b.split_whitespace().collect(),
    );
    assert_eq!(
        tokens_a.len(),
        tokens_b.len(),
        "the pair must align token for token"
    );
    let differing: Vec<(&str, &str)> = tokens_a
        .iter()
        .zip(&tokens_b)
        .filter(|(x, y)| x != y)
        .map(|(x, y)| (*x, *y))
        .collect();
    assert_eq!(differing.len(), 4, "four differences, one of each kind");
    let mut kinds: BTreeSet<&str> = BTreeSet::new();
    for (x, y) in &differing {
        let kind = if looks_like_uuid(x) && looks_like_uuid(y) {
            "uuid"
        } else if looks_like_timestamp(x) && looks_like_timestamp(y) {
            "timestamp"
        } else if x.starts_with("0x") && y.starts_with("0x") {
            "address"
        } else if looks_like_line_col(x) && looks_like_line_col(y) {
            "line:col"
        } else {
            panic!("the pair differs in something D-04 does not normalize: {x} vs {y}");
        };
        kinds.insert(kind);
    }
    assert_eq!(
        kinds.len(),
        4,
        "one difference of each kind, not four of one"
    );
    assert!(
        tokens_a.len() > differing.len(),
        "the pair is entirely variable, so collapsing it proves nothing"
    );

    // Pair two: unflagged, non-empty stderr - D-03's other arm - and differing
    // in exactly one digit, which is the bare integer D-04 must not strip.
    let bare = |rows: &[(bool, String, bool)]| -> String {
        let hits: Vec<String> = rows
            .iter()
            .filter(|(is_error, stderr, _)| !*is_error && !stderr.is_empty())
            .map(|(_, stderr, _)| stderr.clone())
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "exactly one unflagged non-empty stderr per file"
        );
        hits.into_iter().next().unwrap()
    };
    let (two_a, two_b) = (bare(&a), bare(&b));
    assert_eq!(two_a.len(), two_b.len());
    let at: Vec<usize> = two_a
        .bytes()
        .zip(two_b.bytes())
        .enumerate()
        .filter(|(_, (x, y))| x != y)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(at.len(), 1, "the second pair must differ in one byte");
    let index = at[0];
    assert!(
        two_a.as_bytes()[index].is_ascii_digit() && two_b.as_bytes()[index].is_ascii_digit(),
        "the one difference must be a digit"
    );
    // And that digit must not read as a line number, or D-04's `:line:col` rule
    // would collapse this pair for a reason that has nothing to do with it.
    assert!(
        !two_a[..index].ends_with(':'),
        "the bare integer must not sit after a colon"
    );

    // The control: an interruption with an empty stderr and no error flag is
    // not an error, and one file carries one so the extractor can prove it.
    assert!(
        a.iter()
            .any(|(is_error, stderr, interrupted)| !*is_error && stderr.is_empty() && *interrupted),
        "no interrupted-but-not-failed result to control against"
    );
}

/// D-07's equal-score case: the sidechain turn and a top-level turn say exactly
/// the same thing, so BM25 cannot break the tie and the tiebreak under test is
/// the only thing left that can.
#[test]
fn the_echo_sidecar_repeats_a_top_level_turn_word_for_word() {
    let parent = records("session-recall.jsonl");
    let parent_id = parent[0]["sessionId"].as_str().unwrap().to_owned();

    let echo = records("subagents/agent-echo.jsonl");
    assert!(!echo.is_empty());
    for r in &echo {
        assert_eq!(r["sessionId"].as_str(), Some(parent_id.as_str()));
        assert_eq!(r["isSidechain"], serde_json::Value::Bool(true));
    }

    let shared: Vec<String> = echo
        .iter()
        .map(message_text)
        .filter(|text| parent.iter().any(|p| message_text(p) == *text))
        .collect();
    assert_eq!(
        shared.len(),
        1,
        "exactly one echoed turn, or the equal-score comparison is ambiguous"
    );
    assert!(!shared[0].is_empty());

    // No meta file beside it, which is the normal case for 2 of 818 real
    // sidecars and keeps `session_meta.agent_meta` null on this one.
    assert!(
        !testkit::fixture_dir()
            .join("subagents/agent-echo.meta.json")
            .exists(),
        "the echo sidecar is the no-meta case"
    );
}

// --- Phase 5 -----------------------------------------------------------------

/// D-05: `session-edits.jsonl` stores an ABSOLUTE path, the way the real corpus
/// does, and its tool turn carries a second entity beside it.
///
/// Both halves are what AC3 rests on. The absolute spelling is the one a stored
/// `path` entity almost always has - 1,029 absolute against 2 relative over 120
/// sampled real transcripts - and `Query::matches_entity` needs every token of
/// that value present in the query, so a prompt naming the file relatively can
/// only match it after resolution. The second entity is INJ-03's co-occurrence
/// condition having something to fire on: one turn, two independent facts.
///
/// The root token is substituted here rather than left literal, because
/// `{{ROOT}}/...` is not an absolute path on any platform and asserting over it
/// would be asserting over the fixture's placeholder.
#[test]
fn the_edits_fixture_stores_an_absolute_path_and_a_second_entity() {
    use std::path::Path;

    let root = std::env::temp_dir();
    let text = String::from_utf8(testkit::fixture_bytes("session-edits.jsonl")).unwrap();
    let rooted = text.replace(
        testkit::FIXTURE_ROOT_TOKEN,
        &root.to_string_lossy().replace('\\', "\\\\"),
    );

    let mut structural = 0;
    let mut prose = 0;
    for line in rooted.lines() {
        let record: serde_json::Value = serde_json::from_str(line).expect("one JSON object");
        let entities = terminus_core::index::entities(&record);
        let paths: Vec<&str> = entities
            .iter()
            .filter(|e| e.kind == terminus_core::index::entity::PATH)
            .map(|e| e.value.as_str())
            .collect();
        if paths.is_empty() {
            // The prose turn names the same file and emits nothing: entities
            // come from structured tool records, never from a sentence.
            if message_text(&record).contains("lantern.rs") {
                prose += 1;
                assert!(entities.is_empty(), "prose emitted {entities:?}");
            }
            continue;
        }

        structural += 1;
        assert_eq!(paths.len(), 1, "{paths:?}");
        assert!(
            Path::new(paths[0]).is_absolute(),
            "{} is not absolute, so no relative prompt could ever resolve to it",
            paths[0]
        );
        assert!(
            paths[0].starts_with(root.to_string_lossy().as_ref()),
            "{} is not beneath the root the fixture's own cwd names",
            paths[0]
        );

        let distinct: BTreeSet<(&str, &str)> = entities
            .iter()
            .map(|e| (e.kind, e.value.as_str()))
            .collect();
        assert!(
            distinct.len() >= 2,
            "one entity on the tool turn leaves the co-occurrence half of \
             INJ-03 nothing to fire on: {entities:?}"
        );
    }

    assert_eq!(structural, 1, "exactly one turn stores the path");
    assert_eq!(prose, 1, "exactly one turn names it in prose only");
}
