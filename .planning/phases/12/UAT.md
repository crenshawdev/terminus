---
status: testing
phase: 12
fields_version: 1
started: 2026-10-15
updated: 2026-10-15
---

## Items

### 1. Cold start on a fresh data directory
expected: With an empty data directory, `terminus ingest` on a real transcript creates the store with no error, and `terminus search` on a word from that transcript returns turns from it.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Isolated scratch env, data dir absent: `terminus ingest` on a copy of a real transcript exited 0 and created the store; `terminus search Running --project /code/baley/crates --json` returned 6 hits from that session. Same result on tests/fixtures/session-basic.jsonl ("backwards", turn 5).

### 2. initialize asking for 2025-03-26 is answered 2025-06-18
expected: An MCP `initialize` with protocolVersion `2025-03-26` gets a result whose protocolVersion is `2025-06-18`, and `2025-03-26` is not in `PROTOCOL_VERSIONS`.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: mod.rs:74 drops 2025-03-26 from PROTOCOL_VERSIONS; the_revision_that_requires_batches_is_not_offered passed; binary spot-check answered protocolVersion 2025-06-18.

### 3. A request id that is not an integer that fits is refused
expected: A request whose id is `1.5`, `1.0`, `1e3`, or outside both the i64 and u64 range gets a `-32600` error with a null id. String ids and integers from i64::MIN to u64::MAX are answered with the id echoed back.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: rpc.rs:114 is_string/is_i64/is_u64 check; a_request_id_that_is_not_an_integer_that_fits_is_refused passed; binary spot-check refused 1.5, 1.0, 1e3, 2^64 and i64::MIN-1 with -32600 and id null, and echoed u64::MAX, i64::MIN and "abc".

### 4. recall_get removes repeated ids before the cap
expected: A `recall_get` call with 30 ids, 20 of them distinct, returns all 20 records in the order first asked, with a null `reason`.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: tools.rs:535 dedupes before the cap at :541; repeated_ids_are_removed_before_the_cap (tests/mcp.rs:1044) passed: 30 ids / 20 distinct returns the 20 in first-asked order with null reason.

### 5. No indexed turn body is longer than MAX_BODY_BYTES
expected: A record whose pieces reach `MAX_BODY_BYTES` only once separators are counted is indexed at no more than `MAX_BODY_BYTES` bytes, and the body never ends on a bare newline.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: text.rs:103-118 charges the separator and stops after a cut; a_record_that_fills_the_cap_only_with_separators_stays_under_it and the other three index::text tests passed. Expansion tokens keep their own MAX_EXPANSION_BYTES budget, outside #20 by design.

### 6. A compact_boundary subtype on a record that is not system gets no boundary row
expected: A `user`, `assistant` or `attachment` record carrying subtype `compact_boundary` produces no compaction boundary row; a `system` record with that subtype still does.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: derive.rs:62 and parse/record.rs:112 require type system; a_boundary_subtype_on_a_record_that_is_not_system_gets_no_row, only_a_system_record_is_a_compact_boundary and the unchanged system-boundary tests passed.

### 7. The miss-labelling query matches recall_search by the extractor's rule
expected: The query matches `recall_search` and `mcp__terminus__recall_search`, and matches none of `recall_get`, `xrecall_search` and `MCP__TERMINUS__RECALL_SEARCH`.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: label.rs:242-243 exact name or GLOB '*_recall_search', recall_get arm removed; the_miss_query_matches_recall_search_by_the_extractors_rule passed over all five names.

### 8. Two opens of a fresh data directory race safely
expected: The test that races two opens of a fresh data directory passes with the fix, and fails when `initialize` is switched back to a deferred transaction.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: open.rs:304 IMMEDIATE transaction, called from Store::open at :182; a_losing_initialize_waits_for_the_write_lock passed. A scratch SQLite 3.54.0 probe of the same sequence showed the deferred form failing with "database is locked" at 0.00s and the immediate form succeeding after 1.03s.

### 9. A single-file ingest records its counts
expected: After `terminus ingest <path>` on one transcript, `terminus status` shows 1 walked and 1 committed for that run.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: ingest/mod.rs:1267 writes 1, 1, 0; status_counts_the_file_a_single_file_ingest_committed passed; spot-check status printed `files        1 walked, 1 committed, 0 failed`.

### 10. reindex reports its preserved count
expected: `terminus reindex --json` carries an integer `preserved` in both its success and lock-held output, `docs/json-shapes.md` lists it, and `docs/commands.md` says it is printed. Without `--json`, the count prints when it is above 0, and the output is unchanged when it is 0.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: reindex.rs:46 and :73 add preserved; :92-97 print it only above 0; docs/json-shapes.md:217 and docs/commands.md:116 carry it; cli tests passed; spot-check showed one unchanged stderr line at 0 and the preserved line plus "preserved":1 after an eviction.

### 11. verify fails an evicted session that still has block rows
expected: `terminus verify` on a store where an evicted session still has block rows reports exactly one failure, naming that session.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: verify.rs:161-178; an_evicted_session_that_still_has_block_rows_is_one_failure passed; spot-check printed one failure naming the session, 1 failed, exit 1.

### 12. brief.rs uses the shared chars and clip
expected: `brief.rs` defines no `chars` or `clip` of its own.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: No fn chars or fn clip in brief.rs; line 60 imports super::{chars, clip, Payload}; the inject and brief tests passed.

### 13. The lock test no longer claims a process death
expected: No test is named `the_lock_is_released_by_process_death`, and the renamed test's comment names `killing_ingest_anywhere_leaves_a_store_that_converges`.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: Old name absent from crates, docs and CHANGELOG; lock_race.rs:118 renamed test, comment at :115 names killing_ingest_anywhere_leaves_a_store_that_converges; both passed.

### 14. json-shapes.md marks the nullable sessions and show fields
expected: `docs/json-shapes.md` says when each `sessions` and `show` field listed in D-14 is null.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: docs/json-shapes.md:114-147 and :160-174 cover all 15 D-14 fields with when-null bullets; spot-checked claims match parse/record.rs, verify.rs and retention/apply.rs.

### 15. The full test suite passes
expected: `cargo test --workspace --all-features` exits 0 with no failed test.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: cargo test --workspace --all-features --offline: exit 0, 66 binaries, 877 passed, 0 failed.

### 16. CHANGELOG has a Fixed entry for each change
expected: The `[Unreleased]` section of `CHANGELOG.md` has a Fixed entry for each change D-15 lists.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: CHANGELOG.md:12 ### Fixed under [Unreleased] with the seven D-15 entries; additions only, released sections untouched.

### 17. The issues are closed by the merged PR
expected: #10, #14, #20, #25, #28, #29, #30, #33, #35, #39, #41 and #42 are closed by the merged PR, and #17 is open with a reference to it. (human-verify: needs GitHub after the PR merges)
criterion: AC7
status: skipped
reported: skip it
reason: The PR does not exist until /cad-land; checked on GitHub after it merges.

## Summary

total: 17
passed: 16
failed: 0
pending: 0
skipped: 1
blocked: 0
reworked: 0
