---
status: testing
phase: 7
fields_version: 1
started: 2026-10-14
updated: 2026-10-14
---

## Items

### 1. Cold start on an empty data folder
expected: With a fresh, empty data folder, terminus ingests a transcript, the new store reads archive_format 2 with one blocks row per 64 KB block, and a search returns that session's turns.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Spot-check with target/debug/terminus at HEAD on an empty scratchpad TERMINUS_DATA_DIR: ingest exit 0; meta.archive_format 2; 206282 bytes at block_size 65536 stored as 4 blocks rows with 32-byte hashes; sessions has only session_key and session_no; `terminus search` from the session's cwd returned its turn with an excerpt. Schema at crates/terminus-core/src/store/schema.rs:66-91,103; ARCHIVE_FORMAT = 2 at store/open.rs:22.

### 2. Append rewrites at most the tail block
expected: An ingest that appends to a session of three or more blocks writes the new block rows and rewrites at most the last existing one; every other block row of that session is byte-identical before and after. The pass reads at most the last block row per walked file, except a session flagged diverged, whose rows are all read for prefix_matches.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: crates/terminus-core/tests/compaction.rs:409 an_append_rewrites_at_most_the_tail_row_and_reads_no_other passed (rows before the old last byte-identical, old last changed only if mid-block, new rows numbered on, Pass.block_rows_read <= 1). Code: blob/block.rs:119-203 cuts only from the tail; store/archive.rs:297-316 fetches only hashes plus at most the tail row, :275-294 every row only for a diverged session; every walk reaches Existing::read (ingest/mod.rs:164, ingest/backfill.rs:402).

### 3. Readers fetch only the blocks a turn covers
expected: Reading one turn through the excerpt, get, context and brief readers fetches only the block rows the turn's offset and length cover, as counted by the block-row counter, and decompresses only those.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: Eight AC2 tests passed, each asserting block_rows == blocks_decompressed == testkit::covered_blocks on a >=3-block session: recall.rs:1266, :1285 (excerpt), :1366, :1383 (context), :2161, :2177 (get); inject_brief.rs:258, :289 (brief, through the same render the hook uses). All four readers call store/archive.rs:39-139 `ranges`, which selects only rows inside the covering spans.

### 4. verify names the failing block
expected: terminus verify reports a failure naming the session key and block number, in its text line and in a block field under --json, for a block whose bytes were changed and for a block row deleted from the middle of a session; a session checksum that no longer matches the block hashes is reported with the session key and block null; every other session in the store passes.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: Passed: tests/verify.rs:440 (changed byte, block 1), :462 (deleted middle row, block 1), :495 (altered checksum, block None, no 'block' in text), each naming no other of 4 sessions; cli.rs:726 verify_names_the_damaged_block_in_text_and_json (text `<key>: block 1: `, --json block 1, checksum block null). Spot-check of the deleted-row case at the CLI: text `<key>: block 1: row is missing`, --json block 1 plus a block-null checksum failure, only the damaged session named, exit 1.

### 5. Every command refuses a format-1 store
expected: Every command that opens the store, run against a format-1 store, exits non-zero with a message naming the clean-install steps, and the store file's bytes are unchanged; hook still exits 0 and mcp answers with a reason.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: cli.rs:291 every_command_refuses_a_format_one_store_and_leaves_it_untouched passed: 19 invocations non-zero, clean-install steps named, terminus.db and WAL byte-identical, no blocks table, archive_format still 1. Gate at store/open.rs:754-760, run before the writable connection (:157 vs :161). Spot-check on a scratchpad format-1 store: hook SessionStart and UserPromptSubmit exit 0; mcp recall_search answered isError false with the refusal and its three steps as `reason`; terminus.db sha256 unchanged.

### 6. reindex, export, usage and the full suite on format 2
expected: On a format-2 store, reindex gives the same query results as before the rebuild, export writes each session's stream byte-identical to its transcript (capture mode full) with archive_format 2 in the manifest, usage reports archive bytes equal to the sum of the block rows' lengths, and cargo test --workspace --all-features passes, including the crash harness killing ingest at 20 or more randomized points with no watermark past the committed blocks.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: Full suite at HEAD: 837 passed, 0 failed across 64 binaries. Passed by name: cli.rs:447 reindex_rebuilds_the_derived_tables_to_byte_identical_query_output; lifecycle.rs:579 export byte-identical to transcripts with manifest archive_format 2; lifecycle.rs:374 usage archive_bytes reconciles to SUM(length(bytes)) over blocks; crash harness `single file: 23 kills, 19 of them interrupted the pass` with check_invariants rejecting any watermark past the committed blocks (crash.rs:390-519). Spot-check: usage archive_bytes 1843 = sum over blocks 1843; export cmp-identical, manifest archive_format 2.

### 7. Docs describe block rows and the refused older format
expected: DESIGN-BRIEF.md, docs/json-shapes.md and the CHANGELOG.md Unreleased section describe block rows, the per-session block size, verify's block field and the refusal of an older archive format, and neither DESIGN-BRIEF.md nor docs/json-shapes.md says "block-framed blob" or "rebuilds derived tables only".
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: The exact phrases "block-framed blob" and "rebuilds derived tables only" appear in neither DESIGN-BRIEF.md nor docs/json-shapes.md. Block rows, per-session block size, verify's block and the refusal are at DESIGN-BRIEF.md:57-58, 73, 95, 98-99, 457-458, 499; docs/json-shapes.md:37, 45, 168, 175, 414-415; CHANGELOG.md Unreleased (block rows, block_size 64 KB, verify block field, archive format 2 refused with the clean install). Stale wording outside AC6's bar, already in SUMMARY Open items: DESIGN-BRIEF.md:432 "Store format bumps rebuild derived tables only" is now false for the archive format, and several "blob" mentions remain in both files and in the CLI usage text.

### 8. Clean install on the owner's machine
expected: With the old data folder and the current binary moved aside, a clean install ingests every session whose transcript is still on disk, and terminus verify then reports no failure. (human-verify: needs the owner's real store, which the move changes)
criterion: AC7
status: pass
first_pass: pass
reported: terminus verify: 1956 session(s) checked, 0 failed
reason: verify opens the store, so a format-1 store would have been refused; 1956 is the 1953 from the install plus sessions started since

## Summary

total: 8
passed: 8
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 0
