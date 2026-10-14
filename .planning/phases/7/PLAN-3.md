---
phase: 7
plan: 3
requirements: [STOR-08, STOR-09]
files:
  - crates/terminus-core/src/verify.rs
  - crates/terminus-core/src/store/archive.rs
  - crates/terminus/src/cmd/verify.rs
  - crates/terminus-core/tests/verify.rs
  - crates/terminus/tests/cli.rs
  - crates/terminus-core/src/testkit.rs
  - crates/terminus/tests/crash.rs
  - crates/terminus-core/src/recall/excerpt.rs
  - crates/terminus-core/src/recall/get.rs
  - crates/terminus-core/src/recall/search.rs
  - crates/terminus-core/src/inject/prompt.rs
  - crates/terminus-core/tests/recall.rs
  - crates/terminus-core/tests/inject_prompt.rs
  - crates/terminus-core/src/recall/context.rs
  - crates/terminus-core/src/inject/brief.rs
  - crates/terminus-core/tests/inject_brief.rs
---

# Phase 7: Blocks as rows - Plan 3

## Goal

The phase goal: each session's archive is stored as numbered blocks, one row per block, so an append touches only the tail block, a read fetches only its blocks, and corruption is pinned to one block.

Plan 2 made block rows the storage. This plan proves the last two clauses. `terminus verify` names the session and block of every failure. Every turn reader fetches only the rows its turn covers, and a counter says so. The crash harness states its guarantees over block rows.

## Must be true when done

- **`verify` pins corruption to a block (AC3).** `terminus verify` reports a failure naming the session key, in its text line and in a `block` field under `--json`, for each of three kinds of damage: a block whose bytes were changed, a block row deleted from the middle of a session, and a session checksum that no longer matches the block hashes. The first two name the block number. Every other session in the same store passes.
- **A turn read fetches only its blocks (AC2).** Reading one turn through `recall/excerpt.rs`, `recall/get.rs`, `recall/context.rs` and `inject/brief.rs` fetches only the block rows the turn's `stream_offset` and `byte_len` cover, as counted by a block-row counter, and decompresses only those. `Reads.blobs` no longer exists.
- **The crash harness speaks in block rows.** It kills ingest at 20 or more randomized points, and each store it reopens has no watermark past its committed blocks. Every block that was complete after a first pass is byte-identical after the append pass.

## Context

- Plan 3 of 4. It runs after plan 2, so the storage is block rows and `ARCHIVE_FORMAT` is 2, and it shares files with plans 1, 2 and 4.
- This plan is bound by D-12 (crash checks restated), D-13 (range readers fetch covering rows), D-14 (the block-row counter replaces `Reads.blobs` and sits beside `blocks_decompressed`), D-15 (readers check no hash) and D-16 (verify).
- Readers stay as plan 1 left them: `store/archive.rs` is the only code that fetches block rows.
- Out of scope: checking hashes on read (CONTEXT Out). The brief's rendered text changes in no byte, because INJ-02 requires byte-identical output. No new `verify` check beyond D-16's three.

## Tasks

### Task 1: Make `verify` check every block and name the block that fails

- **Files:** crates/terminus-core/src/verify.rs (anchors: `Failure`, `Report::render`, `walk`, `check`), crates/terminus-core/src/store/archive.rs, crates/terminus/src/cmd/verify.rs (anchor: `run`), crates/terminus-core/tests/verify.rs, crates/terminus/tests/cli.rs (the `verify` tests), crates/terminus-core/src/testkit.rs
- **Action:**
  - For each session that is not evicted, check three things (D-16), one session's rows at a time, read through the archive module's every-row-as-stored read:
    - **Numbering.** `session_meta`'s block size and `uncompressed_len` give the block count. The rows must be numbered from 0 to that count minus 1 with no gap. Each missing number and each unexpected number is a failure naming that block. A block size of 0, or one above `MAX_BLOCK_SIZE`, is a failure for the session, never an abort or an allocation.
    - **Each block.** It must decompress to its expected length and hash to its stored hash. Otherwise it is a failure naming that block.
    - **The session checksum.** It must equal BLAKE3 over the stored hashes in `block_no` order. Otherwise it is a failure for the session.
  - Report every failing check, not only the first one.
  - `Failure` gains an optional block number:
    - `Report::render` prints `<session_key>: block N: <detail>` when the number is set, and `<session_key>: <detail>` when it is not.
    - `cmd::verify::run`'s `--json` failures gain a `block` field, a number or null.
    - Session-level failures carry no block number. Those are the checksum, a missing `session_meta` row, a bad block size and divergence. See Notes.
  - Keep everything else:
    - evicted sessions are counted and never checked;
    - the divergence check stays independent of these;
    - a session row with no `session_meta` row is still a failure;
    - `PRAGMA integrity_check` stays `doctor`'s (v0.1.0 D-16);
    - the session key is the identifier.
  - Restate the existing tests in `tests/verify.rs` and `cli.rs` whose text matched the blob-era detail strings.
  - Add the three AC3 tests in `tests/verify.rs`, on a session of at least three blocks:
    - a changed byte in a middle block's bytes;
    - a deleted middle row;
    - an altered `session_meta.checksum`.
  - Each test asserts that the report names that session (and the block number in the first two) and no other session key. One CLI test asserts the `block` field under `--json`.
- **Verify:**
  - `cargo test -p terminus-core --all-features --test verify` and `cargo test -p terminus --all-features --test cli` pass, including the three AC3 tests.
  - The text line of the changed-byte case reads `<session_key>: block <N>: ` followed by its detail.
  - Its `--json` failure has `"block": <N>`, and the checksum case has `"block": null`.

### Task 2: Restate the crash harness over block rows

- **Files:** crates/terminus/tests/crash.rs (anchors: `Snapshot`, `check_invariants`, `killing_a_tree_pass_converges_on_a_first_pass_and_on_an_append`), crates/terminus-core/src/testkit.rs
- **Action:** Restate the harness's guarantees over block rows, through testkit's own SQL (D-12).
  - `Snapshot` holds every block row of every session (`block_no`, bytes and hash), so "the same store an uninterrupted pass reaches" means the same rows.
  - `check_invariants` keeps its five properties, each stated over block rows:
    1. Every session that is not evicted passes `verify`'s checks.
    2. No derived row is orphaned.
    3. No turn row addresses a byte past the committed blocks' stream length.
    4. No watermark sits past that length.
    5. The byte before each watermark, read from the committed rows, is a newline.
  - Add the D-12 check that complete blocks are byte-identical across passes. In the first-pass-then-append scenario, record every block row that was complete after the first pass. After the append pass, assert each of them is unchanged, in the reference store and in every interrupted store.
  - Leave the kill counts and their spread alone. The single-file test is 3 rounds of the 5 points plus 8 timed kills, 23 in all, against AC5's 20 and the 20-point bar set by v0.1.0 phase 1 UAT ("Crash at 20+ randomized points converges"). Both negative tests must still complain.
- **Verify:**
  - `cargo test -p terminus --all-features --test crash` passes. Its output shows at least 20 kills in the single-file test and kills on both the first pass and the append pass.
  - `the_harness_catches_a_watermark_committed_outside_the_transaction` and `the_harness_catches_a_watermark_off_a_record_boundary` still pass, which means the harness still catches both watermark defects.

### Task 3: Count block rows fetched in `Reads`, for search excerpts and `recall_get`

- **Files:** crates/terminus-core/src/store/archive.rs, crates/terminus-core/src/recall/excerpt.rs (anchors: `Reads`, `attach`, module doc), crates/terminus-core/src/recall/get.rs (anchors: `Fetched`, `records`, module doc), crates/terminus-core/src/recall/search.rs (anchor: `Response` `reads` doc), crates/terminus-core/src/inject/prompt.rs (anchor: `Fired` `reads` doc), crates/terminus-core/tests/recall.rs, crates/terminus-core/tests/inject_prompt.rs
- **Action:**
  - The archive module's range read reports how many block rows it fetched from the database, beside the blocks it decompressed.
  - `Reads` replaces `blobs` with that block-row count and carries blocks decompressed beside it (D-14). Both stay in the normal build, for the reason `blocks_decompressed` does: the claim is about a number.
  - Within one request, each session's covering rows are fetched once, even when several hits share a block (D-20's per-session grouping, now per row).
  - Rewrite the docs that explain the old cost. `excerpt.rs` and `get.rs` both say rusqlite's incremental blob I/O is off, so one turn materializes the whole compressed session. That is no longer true. Also rewrite the `reads` docs on `Response` and `Fired`.
  - Restate every test that asserted `reads.blobs`, in `tests/recall.rs` and `tests/inject_prompt.rs`, as the block rows and blocks the hits actually cover. An out-of-scope or excluded id still fetches 0 rows.
  - Add AC2 tests for the excerpt and get readers, on a session of at least three blocks:
    - a turn inside one block fetches 1 row and decompresses 1 block;
    - a turn spanning k blocks fetches k rows and decompresses k blocks.
- **Verify:**
  - `cargo test -p terminus-core --all-features --test recall --test inject_prompt` passes, including the new AC2 tests.
  - `grep -rn 'reads.blobs\|pub blobs' crates/` prints nothing.

### Task 4: Report the same counters from the context and brief readers

- **Files:** crates/terminus-core/src/recall/context.rs (anchors: `Window`, `fill_text`), crates/terminus-core/src/inject/brief.rs (anchors: `last_exchange`, `render`, `Brief`, `session_start`), crates/terminus-core/tests/recall.rs, crates/terminus-core/tests/inject_brief.rs
- **Action:**
  - `context::window` reports the block rows fetched and the blocks decompressed for the window, using Task 3's `Reads`.
  - The brief's `last_exchange` reports the same counters for the two turns it quotes, and an integration test can read them.
  - The hook's stdout must not change in any byte (INJ-02). A testkit-gated entry point is acceptable, the same way `ingest::fault` keeps test-only paths out of the shipped binary.
  - Add AC2 tests for both readers on a session of at least three blocks. The rows fetched must equal the number of blocks the turn's range covers, or for the brief the union over its two quoted turns, with blocks decompressed equal to that count too.
- **Verify:**
  - `cargo test -p terminus-core --all-features --test recall --test inject_brief` passes, including the new AC2 tests.
  - `cargo test --workspace --all-features` passes, which includes `recall_golden.rs`'s goldens unchanged.

## Notes

- **AC3's third case names no block.** AC3 lists "a session checksum that no longer matches the block hashes" among the failures that name a block number, and D-16 says each failure carries one. The checksum covers every block hash at once. When it fails while every block matches its own hash, no single block is wrong, so Task 1 reports that case with `block` null and the text line without "block N:".
  - If a stored hash is altered, the same session also gets a block-numbered failure from that block's own check.
  - The owner confirmed this reading on 2026-10-14, and AC3 and D-16 in CONTEXT now say it.
