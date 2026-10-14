---
phase: 7
plan: 2
requirements: [STOR-08, STOR-10]
files:
  - crates/terminus-core/src/blob/block.rs
  - crates/terminus-core/src/blob/mod.rs
  - crates/terminus-core/src/blob/reader.rs
  - crates/terminus-core/src/blob/writer.rs
  - crates/terminus-core/tests/blob.rs
  - crates/terminus-core/src/store/open.rs
  - crates/terminus-core/src/error.rs
  - crates/terminus-core/src/reindex.rs
  - crates/terminus-core/src/inject/mod.rs
  - crates/terminus/src/cmd/install/mod.rs
  - crates/terminus-core/tests/store_open.rs
  - crates/terminus-core/tests/reindex.rs
  - crates/terminus/tests/install.rs
  - crates/terminus-core/src/store/schema.rs
  - crates/terminus-core/src/store/archive.rs
  - crates/terminus-core/src/ingest/mod.rs
  - crates/terminus-core/src/retention/apply.rs
  - crates/terminus-core/src/verify.rs
  - crates/terminus-core/src/testkit.rs
  - crates/terminus-core/tests/compaction.rs
  - crates/terminus-core/tests/schema.rs
  - crates/terminus-core/src/lib.rs
  - crates/terminus-core/src/recall/excerpt.rs
  - crates/terminus-core/src/recall/get.rs
  - crates/terminus-core/tests/recall.rs
---

# Phase 7: Blocks as rows - Plan 2

## Goal

The phase goal: each session's archive is stored as numbered blocks, one row per block, in place of one whole-session blob, with the session checksum a hash over the block hashes, so an append touches only the tail block, a read fetches only its blocks and corruption is pinned to one block.

This plan makes that the storage. The store moves to archive format 2, ingest appends by rewriting at most the tail block, and a format-1 store is refused with the clean-install steps instead of being half-upgraded.

## Must be true when done

- **A fresh store keeps its archive in block rows.** Its `sessions` table has only `session_key` and `session_no`. Each archived session's bytes are rows of a new block table keyed `(session_no, block_no)`, and each row holds the block's compressed bytes and a 32-byte BLAKE3 hash of its uncompressed bytes. `session_meta.checksum` is BLAKE3 over those hashes in `block_no` order. `session_meta` records the session's block size, which is 65,536 bytes. `meta.archive_format` reads 2.
- **An append touches only the tail (AC1).** An ingest that appends to a session of at least three blocks writes its new block rows and rewrites at most its last existing block row. Every other block row of that session is byte-identical before and after. For each file the pass walks, it reads the bytes of at most that session's last block row.
- **An older store is refused untouched.** A store whose archive format is older than this build's is refused by `Store::open` and by `Store::open_read_only` before any writable connection opens. The message names the clean-install steps, the store file is unchanged, and no table is created in it. `terminus install` on such a store prints that refusal, not "the next hook archives the same tree", and exits non-zero.
- **The whole-session blob format is gone.** No `BlobHeader`, no `BlobReader`, no `VBLB` header and no blob-level write or append remains. `cargo test --workspace --all-features` passes on block rows, including the crash harness, reindex's identical query results and every `verify` test.

## Context

- Plan 2 of 4. It runs after plan 1, whose archive module (`store/archive.rs`) and testkit helpers are where this switch lands, and before plans 3 and 4. All four plans share files and run in order.
- This plan is bound by these decisions:
  - D-01 and D-07: the refusal and its message.
  - D-03: the block table.
  - D-04: hashes over uncompressed bytes, and a checksum over the hashes.
  - D-05: the block size is recorded per session and stays at 64 KB; zstd stays at level 3.
  - D-09: eviction and deletion.
  - D-10 and D-11: the append and the existing state it reads.
  - D-12: one transaction per file, with `prepare` and `apply` split as now.
  - D-13: the exception for flagged sessions.
  - D-15: readers check no hash.
  - The recalled v0.1.0 phase 8 D-03: evicted sessions need an explicit arm in `verify`, `reindex` and the append path. All three have one today; keep them.
- Out of scope:
  - a conversion from format 1;
  - changing 64 KB or zstd level 3;
  - checking hashes on read;
  - `verify`'s per-block report and its `block` field (plan 3);
  - the block-row read counter (plan 3);
  - the end-to-end AC4 test over every command (plan 4);
  - docs (plan 4).

## Tasks

### Task 1: Add the block codec

- **Files:** crates/terminus-core/src/blob/block.rs, crates/terminus-core/src/blob/mod.rs, crates/terminus-core/tests/blob.rs
- **Action:**
  - Add the block-level codec as pure functions, with no SQL and no connection, in `crates/terminus-core/src/blob/block.rs`, declared and re-exported from `blob/mod.rs`. The whole-blob code stays as it is, because ingest still writes blobs until Task 3.
  - **Writing.** Cut a byte stream at a given block size into blocks, numbered from a given block number. Compress each one independently at `ZSTD_LEVEL`. Hash each one with BLAKE3 over its uncompressed bytes (D-04). Never hash the compressed bytes: a later zstd that re-encodes a block would then fail every hash, which is v0.1.0 phase 1 D-06's reasoning carried down to the block.
  - **The session checksum.** BLAKE3 over the block hashes, concatenated in `block_no` order (D-04).
  - **Reading.**
    - Given a block size and an uncompressed length, say which block numbers a stream range covers.
    - Read the range out of exactly those blocks' compressed bytes.
    - Decompress each block to its expected length, which is the block size or what is left of the stream. A length mismatch fails the way `decompress_block` fails today.
    - Count the blocks decompressed.
    - Reading past the end is `Error::RangeOutOfBounds` and never a short read, and the up-front reservation stays capped at `MAX_RESERVE`. That is `BlobReader::read_range`'s contract, kept for blocks that arrive as separate rows.
  - **Appending (D-10).** The inputs are the session's block size and current uncompressed length, every stored block hash in `block_no` order, the recorded session checksum, the tail block's compressed bytes and stored hash when the stream ends mid-block, and the new bytes.
    - First check that the hashes reproduce the checksum and that the tail decompresses and hashes to its stored hash. On either failure, return `Error::BlobChecksumMismatch` and nothing to write. Those two checks are all an append makes, and no complete block's bytes are an input.
    - The result carries:
      - the rows to write: the rewritten tail at its own block number, then the new blocks;
      - the new checksum, computed from the untouched hashes plus the new ones;
      - the new uncompressed length;
      - the number of blocks compressed, and how many of those redid stored bytes. The second number is at most 1. These are the counters `Appended` carries today, there so that an append which recompressed more than the tail shows up in a number.
    - An append onto an exact block boundary recompresses nothing.
  - **Block size bounds.** The block size now comes from a column (D-05), so refuse 0, and anything above `MAX_BLOCK_SIZE`, with `Error::BlobFormat`, for the reason `MAX_BLOCK_SIZE`'s comment gives. A new session uses `BLOCK_SIZE`. An append cuts at the session's own size.
  - **Tests.** `tests/blob.rs` gains codec tests covering what the blob tests cover today:
    - round trips at the named boundary lengths and at pseudorandom lengths;
    - appending a kilobyte at a time gives the same rows and checksum as one write;
    - an append onto a boundary recompresses nothing;
    - an append crossing a boundary redoes only the tail;
    - a tampered hash list and a tampered tail are both refused;
    - a range decompresses exactly the blocks it touches;
    - reading past the end is an error;
    - the checksum equals BLAKE3 over the hashes, computed independently in the test;
    - an absurd block size is refused.
- **Verify:** `cargo test -p terminus-core --all-features --test blob` passes, including the new tests above. `cargo test --workspace --all-features` passes.

### Task 2: Refuse a store whose archive format is older than this build's

- **Files:** crates/terminus-core/src/store/open.rs (anchors: `read_versions`, `ARCHIVE_FORMAT`, `RebuildRequired`), crates/terminus-core/src/error.rs (anchor: `ArchiveFormatTooNew`), crates/terminus-core/src/reindex.rs (anchors: module doc, `open_up_to_date`), crates/terminus-core/src/inject/mod.rs (anchor: `open`), crates/terminus/src/cmd/install/mod.rs (anchor: `start_backfill`), crates/terminus-core/tests/store_open.rs (anchor: `an_older_archive_format_also_reports_a_rebuild`), crates/terminus-core/tests/reindex.rs (anchor: `the_version_integers_move_with_the_rebuild`), crates/terminus/tests/install.rs
- **Action:**
  - **The gate (D-01).** `read_versions` refuses a store whose `archive_format` is below `ARCHIVE_FORMAT` with a new `Error` variant beside `ArchiveFormatTooNew`. It refuses for both `Store::open` and `Store::open_read_only`, and before any writable connection opens, so `bring_forward` never runs on such a store.
    - `RebuildRequired` now arises only from a `derived_schema` mismatch. That is STOR-10's second half and replaces STOR-05's older-format rebuild arm.
    - The rejected alternative explains why: rebuilding would add an empty block table, run `reindex`, fail every session and stamp `archive_format = 2` on a store whose only bytes sit in the old `blob` column.
  - **The message (D-07).** Shape it like `ArchiveFormatTooNew`: the store path and both integers. Then add the clean-install steps: move the data folder and the current terminus binary aside, then rerun `install.sh`. Say there is no conversion. This is in-tool text, so read the in-tool text rules in `~/.claude/john-voice.md` first (CLAUDE.md).
  - **`ARCHIVE_FORMAT` stays 1 in this task.** Task 3's bump is what makes a real store hit this refusal. Tests express "older" as `ARCHIVE_FORMAT - 1`, so they keep their meaning after the bump.
  - **`terminus install` (D-07).** When the store is refused for its archive format, `start_backfill` prints the refusal, which names the steps, and does not print "nothing else is affected; the next hook archives the same tree". On such a store every hook-spawned pass fails.
    - `Failure` carries only text, so recognize the new variant where the core error is still typed, for example by asking `Store::open_read_only` directly in install.
    - **Install exits non-zero on that refusal (D-07, owner's call 2026-10-14).** `install.sh` runs `terminus install` last under `set -eu`, so install's status is the script's. `start_backfill` returns whether the store was refused for its archive format, and `run` returns `Failure::Silent` after the summary when it was, the way an incomplete install already does (its `Err(_) => Err(Failure::Silent)` arm). Every other backfill problem still prints its line and leaves the exit at 0.
    - Rewrite `start_backfill`'s "It cannot fail an install" paragraph to say so: a backfill problem never fails an install, except a store this build refuses, because then no hook will archive anything.
  - **Doc comments.** Correct the ones that promise an older archive format gets rebuilt: `ARCHIVE_FORMAT`'s doc, `reindex`'s module doc and `open_up_to_date`, and `inject::open`'s "the next ingest rebuilds the derived tables and the brief comes back".
  - **Tests.**
    - In `store_open.rs`, replace `an_older_archive_format_also_reports_a_rebuild` with tests showing that both opens refuse with the new variant, the message names the data folder, the binary and `install.sh`, the file bytes are unchanged, and the store's table list is unchanged.
    - In `reindex.rs`, `the_version_integers_move_with_the_rebuild` now lowers only `derived_schema`, because an older `archive_format` now refuses the open.
    - In `install.rs`, install against an older-format store prints the refusal and not the hook sentence, exits non-zero, and still leaves the binary placed and both settings files written.
- **Verify:**
  - `cargo test -p terminus-core --all-features --test store_open --test reindex` passes.
  - `cargo test -p terminus --all-features --test install` passes.
  - `cargo test --workspace --all-features` passes.
  - The new store_open test fails if the `store_archive < ARCHIVE_FORMAT` refusal is removed from `read_versions`.
  - The new install test fails if `run` returns `Ok(())` after a refused archive format.

### Task 3: Store each session as block rows, and append by rewriting at most the tail

- **Files:** crates/terminus-core/src/store/schema.rs (anchors: `CREATE_SQL` `sessions` and `session_meta`, `TABLES`), crates/terminus-core/src/store/open.rs (anchor: `ARCHIVE_FORMAT`), crates/terminus-core/src/store/archive.rs, crates/terminus-core/src/blob/block.rs, crates/terminus-core/src/blob/mod.rs, crates/terminus-core/src/ingest/mod.rs (anchors: `Existing::read`, `ExistingSession`, `prepare`, `Work`, `apply`, `prefix_matches`, `write_session_meta`, `Pass`), crates/terminus-core/src/retention/apply.rs (anchors: `evict_one`, `delete_one`), crates/terminus-core/src/verify.rs (anchor: `check`), crates/terminus-core/src/testkit.rs (anchors: `archive_digest` and plan 1's archive helpers), crates/terminus-core/tests/compaction.rs (anchors: `completed_blocks`, `raw_blob`, `archived`, `the_completed_block_helper_excludes_only_the_partial_tail`), crates/terminus-core/tests/schema.rs (anchor: `sessions_carries_a_stable_integer_surrogate`)
- **Action:**
  - **Schema (D-03, D-05).**
    - `sessions` keeps `session_key` and `session_no` and loses `blob`.
    - Add the block table: one row per block, with the composite key `(session_no, block_no)`, the compressed bytes, and a 32-byte hash. Make it an ordinary rowid table with `PRIMARY KEY (session_no, block_no)`. Not `WITHOUT ROWID`, and the page size stays at SQLite's default: that is what the measurement in the Notes chose. List it in `TABLES` right after `sessions`, and never in `DERIVED_TABLES`, so `reindex` never drops it.
    - `session_meta` gains the per-session block size column. Write it when a session is first archived and never change it on a later pass. Declare it beside `uncompressed_len`, before the bring-forward columns, so `tests/schema.rs`'s `a_brought_forward_session_meta_has_the_same_column_order_as_a_fresh_one` still holds. Leave it out of `BRING_FORWARD_COLUMNS`: no format-2 store predates it, and every format-1 store is refused.
    - Insert a session's row before its blocks, and delete its blocks before its row, whether or not a reference is declared (D-09). `reindex`'s drop-order comment records that the bundled SQLite enforces declared foreign keys.
  - **Format.** `ARCHIVE_FORMAT` becomes 2, and its doc says what format 2 is. A format-1 store now meets Task 2's refusal.
  - **Archive module.** Plan 1 made the readers depend on its contract; only what is behind it changes.
    - A range read fetches only the rows whose blocks the requested ranges cover, each row once (D-13). It finds them from the session's own block size and `uncompressed_len` in `session_meta`. A missing row, or a block that will not decode, costs only the ranges that need it, and nothing checks a hash (D-15).
    - A whole read fetches every row in `block_no` order and decodes them. A gap or a decode failure is an error for that session.
    - A session's stored byte count is the sum of its rows' byte lengths (D-17).
    - New in this task:
      - the append state: every block hash in order, plus the tail row's bytes when the stream ends mid-block, or every row's bytes when the session is flagged diverged;
      - row writes: new rows and the replaced tail;
      - removing one session's rows.
  - **Ingest (D-10, D-11, D-12).**
    - `Existing::read` no longer selects the whole session. It reads `session_no`, `checksum`, block size and `uncompressed_len`, every hash, and at most the tail row's bytes. A flagged session is the one exception: it reads all of that session's rows, so `prefix_matches` can compare against the archived stream (D-13). An evicted session reads no rows at all. `Existing::read` still runs on the writer thread (`backfill`'s `read_existing`), so `prepare` stays free of any connection. The refusal of an archived session with no `session_meta` row stays.
    - `prepare` appends through Task 1's codec at the session's recorded block size, and cuts a new session at `BLOCK_SIZE`. Compression and hashing stay outside the transaction, before `fault::AFTER_BLOB`.
    - `apply` writes in the file's one existing transaction (STOR-02): the `sessions` row if it is new, the new block rows and the replaced tail row, then `session_meta` (checksum over the block hashes, `uncompressed_len`, block size on first archive), then the turns, then the watermark. Keep every fault point's name and position. `IN_TX_AFTER_SESSION` still fires after the archive rows and `session_meta`.
    - The `capture_mode` rule is unchanged: completed blocks are still never revisited.
    - Add an instrument in the normal build that counts the existing block rows whose bytes the pass fetched for a file, in the way `Appended` counts recompressions, and make it readable from an integration test. A count carried on `Pass`, which only `prepare` constructs, is one way.
  - **Retention (D-09).** `evict_one` deletes the session's rows and sets `is_evicted`, and leaves `checksum` and `uncompressed_len` exactly as they are, for the reason its doc gives. `delete_one` deletes the block rows before the `sessions` row. `decisions` and `labels` stay untouched.
  - **`verify`.** Only the definition of the checksum changes here. `check` recomputes the session checksum from the decoded stream, cut at the session's block size: hash each block, then hash the hashes. Plan 3 replaces this with per-block checks.
  - **testkit.** Plan 1's helpers now read, compare, count, damage, rewrite and empty block rows with their own SQL. Damage changes bytes and keeps the stored hashes. `archive_digest` hashes `sessions`, every block row, and the `session_meta` columns it hashes today plus the block size.
  - **Tests.**
    - `compaction.rs` reads completed blocks from rows, and its helper self-test checks against `uncompressed_len` and the block size.
    - Add the AC1 test to `compaction.rs`:
      - archive the big-record fixture, a session of at least three blocks;
      - snapshot every row (`block_no`, bytes, hash);
      - append records to the transcript and ingest again;
      - assert that every row except the old last one is byte-identical, that the old last row changed only if the stream ended mid-block, that the rows past it are new, and that the instrument reads 1 or less.
    - `schema.rs`'s `sessions_carries_a_stable_integer_surrogate` asserts that `sessions` has exactly its two columns and that the block table exists.
  - Comments in the files this task touches that still describe the whole-session blob as the storage are corrected.
- **Verify:**
  - `cargo test --workspace --all-features` passes. That includes `--test crash`, `--test reindex`, `--test verify`, `--test retention`, `--test schema`, and the new AC1 test in `--test compaction`.
  - `grep -rnE 'blob FROM|SET blob|\bs\.blob|length\(s\.blob\)|blob +BLOB +NOT' crates/` prints nothing.
  - The AC1 test fails if `prepare` is made to rewrite every row of the session.

### Task 4: Remove the whole-session blob format

- **Files:** crates/terminus-core/src/blob/mod.rs, crates/terminus-core/src/blob/reader.rs, crates/terminus-core/src/blob/writer.rs, crates/terminus-core/src/blob/block.rs, crates/terminus-core/src/error.rs (anchors: `BlobFormat`, `BlobChecksumMismatch`), crates/terminus-core/src/lib.rs (anchor: crate doc), crates/terminus-core/tests/blob.rs, crates/terminus-core/src/recall/excerpt.rs (anchor: `Reads` doc), crates/terminus-core/src/recall/get.rs (anchor: module doc), crates/terminus-core/src/ingest/mod.rs (anchor: `Work::stream_base` doc), crates/terminus-core/tests/recall.rs (anchor: `ids_across_three_sessions_read_three_blobs` doc), crates/terminus-core/tests/compaction.rs (anchor: module doc)
- **Action:**
  - Delete the blob format, which nothing calls after Task 3:
    - `BlobHeader` with its parse and serialize, `BlockEntry`, `MAGIC`, `HEADER_LEN` and `TABLE_ENTRY_LEN`;
    - `BlobReader` over a blob;
    - the blob-level `write` and `append` with `Written`, `Appended`, `finish` and `finish_parts`;
    - `blob::read_all`.
  - `reader.rs` and `writer.rs` go if nothing is left in them.
  - Keep `BLOCK_SIZE`, `ZSTD_LEVEL`, `MAX_BLOCK_SIZE` and `MAX_RESERVE`, which the codec uses.
  - `blob/mod.rs`'s module doc describes block rows. The layout now lives in the schema, not in a header.
  - Where a codec function now does what `blob::write` or `blob::append` did, prefer giving it that path. Comments in files outside this plan, such as `capture.rs`, `retention/mod.rs` and `ingest/mod.rs`, cite `blob::append` for "completed blocks are copied across untouched", which stays true of the codec.
  - Error docs:
    - `BlobFormat` names a block row's bytes or a block size, not `sessions.blob`.
    - `BlobChecksumMismatch` names the hash-list and tail checks an append makes.
  - The crate doc in `lib.rs` stops calling the format "block-framed".
  - `tests/blob.rs` drops the tests that exist only for the header: damaged header, absurd header block size, payload over-claim and block extents. What they protected is covered by Task 1's codec tests.
  - Comments and rustdoc links that cite a deleted name are corrected wherever they sit. As of planning they are in `excerpt.rs` (the `Reads` doc), `get.rs` (the module doc's block-counter sentence), `ingest/mod.rs` (two "`blob::BlobReader`" coordinate sentences, unless Task 3 already fixed them), `tests/recall.rs` (one doc comment), `tests/compaction.rs` (the module doc's `finish_parts` sentence, unless Task 3 already fixed it) and `lib.rs`. Only the sentence naming the deleted item changes.
- **Verify:** `grep -rnE 'BlobHeader|BlobReader|VBLB|finish_parts|block-framed' crates/` prints nothing, and `cargo test --workspace --all-features` passes.

## Notes

- **Block table layout, measured 2026-10-14 as CONTEXT's first flagged assumption asks.**
  - **The data.** A copy of the owner's store: 6,571 sessions and 62,396 blocks, read from each blob's block table without touching the payload. That is 1,063,559,260 compressed bytes. The mean row is 17,045 bytes, p50 16,479, p99 41,754 and the maximum 50,741. The largest session is 404 blocks and 6,042,629 compressed bytes, which is what one append to it rewrites today.
  - **The method.** Rows were filled with random bytes at the real lengths, in WAL mode with `synchronous=normal`. One append rewrote the tail a little larger and added one row, and was measured as the WAL bytes it wrote.
  - **Rowid table, `PRIMARY KEY (session_no, block_no)`, 4096-byte pages:**
    - the file is 1.047 times the payload;
    - one append writes 40.3 KiB of WAL, the same for the largest session and for the median one;
    - 100 appends grew the file by 0.16 MB, because freed overflow pages are reused;
    - a point read takes 3.5 µs.
  - **`WITHOUT ROWID` at 4096-byte pages:** 1.098 times the payload, 52 to 56 KiB of WAL per append, and 16 µs per point read.
  - **Larger pages** were worse on space at every size. The rowid table was 1.090 times at 8 KB, 1.160 at 16 KB and 1.185 at 64 KB, and at 64 KB an append wrote 192 KiB of WAL. `WITHOUT ROWID` at 64 KB was 2.632 times.
  - **Result:** a rowid table, with SQLite's default page size left alone. The scripts lived in the session scratchpad and are not kept in the repo. The live store was copied once and never opened.
- **D-13 makes an exception to AC1.** AC1 says the pass reads at most a session's last block row for each file it walks. D-13 has `prefix_matches` read all of one session's rows for a session flagged diverged. The AC1 test therefore uses an unflagged session. A flagged session is skipped every pass until it is whole again, so the exception costs a full read only for a file that is already being refused.
- **`session_meta` column placement.** The new column goes before the bring-forward columns rather than at the end, because `aged_store` in `tests/schema.rs` drops and re-adds the bring-forward columns, and a column declared after them would give a fresh store and a brought-forward one different orders.
