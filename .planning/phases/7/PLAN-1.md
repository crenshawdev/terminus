---
phase: 7
plan: 1
requirements: [STOR-08]
files:
  - crates/terminus-core/src/store/archive.rs
  - crates/terminus-core/src/store/mod.rs
  - crates/terminus-core/src/recall/excerpt.rs
  - crates/terminus-core/src/recall/get.rs
  - crates/terminus-core/src/recall/context.rs
  - crates/terminus-core/src/inject/brief.rs
  - crates/terminus-core/src/testkit.rs
  - crates/terminus-core/src/reindex.rs
  - crates/terminus-core/src/verify.rs
  - crates/terminus-core/src/observe/mechanical.rs
  - crates/terminus/src/cmd/export.rs
  - crates/terminus/src/cmd/usage.rs
  - crates/terminus-core/tests/capture.rs
  - crates/terminus-core/tests/derive.rs
  - crates/terminus-core/tests/ingest.rs
  - crates/terminus-core/tests/lineage.rs
  - crates/terminus-core/tests/pass.rs
  - crates/terminus-core/tests/recall.rs
  - crates/terminus-core/tests/recover.rs
  - crates/terminus-core/tests/reindex.rs
  - crates/terminus-core/tests/retention.rs
  - crates/terminus-core/tests/schema.rs
  - crates/terminus-core/tests/verify.rs
  - crates/terminus/tests/backfill.rs
  - crates/terminus/tests/cli.rs
  - crates/terminus/tests/corpus.rs
  - crates/terminus/tests/crash.rs
  - crates/terminus/tests/lifecycle.rs
  - crates/terminus/tests/recall_cli.rs
---

# Phase 7: Blocks as rows - Plan 1

## Goal

The phase goal: each session's archive is stored as numbered blocks, one row per block (`session_no`, `block_no`, bytes, hash), in place of one whole-session blob. The session checksum is a hash over the block hashes, so an append touches only the tail block, a read fetches only its blocks, and corruption is pinned to one block.

This plan changes nothing a user can see. Every product read of a session's archived bytes moves behind one module, and every test that inspects the archive moves behind `testkit`. Plan 2 can then switch the storage to block rows by changing that module, the writer, retention and testkit, without touching every reader and every test in the same commit.

## Must be true when done

- Every product read of a session's archived bytes goes through `crates/terminus-core/src/store/archive.rs`. Running `grep -rnE 'blob FROM|SET blob|\bs\.blob|length\(s\.blob\)' crates/terminus-core/src crates/terminus/src` names only four files: that module, `ingest/mod.rs` (the writer), `retention/apply.rs` (evict) and `testkit.rs`.
- No test names `sessions.blob` or calls the blob format directly. Every test reads, sizes, damages and empties the archive through `testkit`. The only exceptions are `tests/blob.rs` and `tests/compaction.rs`, which test the format itself.
- Nothing observable has changed. `cargo test --workspace --all-features` passes, and every assertion that existed before this plan still checks what it checked: the same excerpts, records, windows, briefs, rebuilds, `verify` reports, exports, `usage` numbers, observations and `Reads.blobs` counts.

## Context

- Plan 1 of 4. The plans are sequential and share files, and each one runs after the one before it. The storage is still `sessions.blob` throughout this plan, and `ARCHIVE_FORMAT` stays 1.
- This plan is bound by four decisions:
  - D-13: range readers and whole-session readers are distinct.
  - D-15: readers check no hash.
  - v0.1.0 phase 1 D-04: turn rows keep their uncompressed coordinates.
  - D-20: one fetch per session per request, as `recall/excerpt.rs` documents it.
- Follow the existing shape: `reindex::reindex` already reads "keys first, one session at a time". The new module follows that, and it takes the `&Connection` that `Store::conn` hands out, as `config::visible::sessions` does.
- Out of scope: the ingest writer, and retention's evict and delete, which plan 2 changes along with the schema; the block codec (plan 2); the block-row counter (plan 3, D-14). No comment sweep beyond the code a task changes, because "comments back to invariants" is a deferred seed.

## Tasks

### Task 1: Route the four range readers through one archive module

- **Files:** crates/terminus-core/src/store/archive.rs, crates/terminus-core/src/store/mod.rs, crates/terminus-core/src/recall/excerpt.rs (anchor: `attach`), crates/terminus-core/src/recall/get.rs (anchor: `records`), crates/terminus-core/src/recall/context.rs (anchor: `fill_text`), crates/terminus-core/src/inject/brief.rs (anchor: `last_exchange`), crates/terminus-core/src/testkit.rs (anchor: `read_turn`)
- **Action:**
  - Create `crates/terminus-core/src/store/archive.rs` as a public module of `store`, declared in `store/mod.rs`. From now on it holds the SQL that reads a session's archived bytes. Plan 2 changes what sits behind it, one blob today and block rows after, and no reader should have to change when that happens.
  - In this task the module reads ranges. A caller names one session by `session_key` and asks for some `(stream_offset, byte_len)` ranges of its uncompressed stream. The module returns:
    - each range's bytes, or a failure for that range alone;
    - the number of blocks it decompressed.
  - Behind it for now is one `SELECT blob FROM sessions WHERE session_key = ?1` and one `BlobReader` per call, so a session is still fetched once per request however many ranges are asked of it (D-20). "This session has no stored bytes" must stay distinguishable from "this range would not read", because the readers handle both today.
  - Move `excerpt::attach`, `get::records`, `context::fill_text`, `brief::last_exchange` and `testkit::read_turn` onto the module. Each keeps exactly its current behavior:
    - It still groups by session.
    - A session with no row, or with bytes that will not open, costs those turns their text and nothing else.
    - `Reads.blobs` still counts one per session the reader asked about.
    - `read_turn` keeps its signature and still reports blocks decompressed. It measures the product's own read path, so it should go through the module.
  - The module checks no hash (D-15). Coordinates stay in the uncompressed stream (v0.1.0 phase 1 D-04).
  - Do not change `Reads`, `Window`, `Fired` or the brief's output.
- **Verify:**
  - `cargo test --workspace --all-features` passes, and no test file was edited.
  - `grep -nE 'blob FROM|BlobReader' crates/terminus-core/src/recall/excerpt.rs crates/terminus-core/src/recall/get.rs crates/terminus-core/src/recall/context.rs crates/terminus-core/src/inject/brief.rs | grep -vE '^[^:]+:[0-9]+:\s*//'` prints nothing. Comment lines are filtered out, and any rustdoc link to `BlobReader` the move leaves dangling is corrected.
  - `testkit::read_turn` no longer names the `blob` column.

### Task 2: Route the whole-session readers and the archive sizes through the same module

- **Files:** crates/terminus-core/src/store/archive.rs, crates/terminus-core/src/reindex.rs (anchor: `reindex`), crates/terminus-core/src/verify.rs (anchors: `walk`, `check`), crates/terminus-core/src/observe/mechanical.rs (anchor: `from_blob`), crates/terminus/src/cmd/export.rs (anchor: `write_export`), crates/terminus/src/cmd/usage.rs (anchor: `measure`)
- **Action:**
  - The module gains two reads:
    - One session's whole uncompressed stream, which errors the way `blob::read_all` errors today when the bytes will not read.
    - Every session's stored archive byte count, keyed by `session_key`. This is `usage`'s input, and today it is `length(s.blob)`.
  - Move these callers onto the module:
    - `reindex::reindex`. It keeps reading keys first and one session at a time, and an unreadable session still lands on `Rebuilt::failed`.
    - `verify`'s `walk`. It reads keys and `session_meta` first and then each session's stream one at a time, rather than selecting every blob in one statement. That is the same memory bound `reindex` documents. `check` still compares BLAKE3 over the stream against `session_meta.checksum`, unchanged in this task (D-06 as it stands until plan 2).
    - `mechanical::from_blob`.
    - `export::write_export`. Its evicted arm still writes an empty file and reads nothing.
    - `usage`'s `measure`. Its totals reconcile exactly as before.
  - `usage` lives in the binary crate, which does not name `rusqlite` (see `cmd::status`). Call the module with the connection `reader.store().conn()` already hands out.
- **Verify:**
  - `cargo test --workspace --all-features` passes, and no test file was edited.
  - `grep -nE 'blob FROM|\bs\.blob|length\(s\.blob\)|read_all' crates/terminus-core/src/reindex.rs crates/terminus-core/src/verify.rs crates/terminus-core/src/observe/mechanical.rs crates/terminus/src/cmd/export.rs crates/terminus/src/cmd/usage.rs | grep -vE '^[^:]+:[0-9]+:\s*//'` prints nothing.

### Task 3: Make the core crate's tests reach the archive only through testkit

- **Files:** crates/terminus-core/src/testkit.rs, crates/terminus-core/tests/capture.rs, crates/terminus-core/tests/derive.rs, crates/terminus-core/tests/ingest.rs, crates/terminus-core/tests/lineage.rs, crates/terminus-core/tests/pass.rs, crates/terminus-core/tests/recall.rs, crates/terminus-core/tests/recover.rs, crates/terminus-core/tests/reindex.rs, crates/terminus-core/tests/retention.rs, crates/terminus-core/tests/schema.rs, crates/terminus-core/tests/verify.rs
- **Action:**
  - Add testkit helpers that do what these tests now do with raw SQL on `sessions.blob`:
    - read one session's whole archived stream, erroring when the stored bytes do not decode;
    - return one session's stored archive bytes in a form two reads, or two stores, can be compared by;
    - count one session's stored archive bytes;
    - damage one session's stored bytes so they no longer decode. This covers `verify.rs`'s one-byte flip and the zeroed payload ranges in `recall.rs` and `reindex.rs`.
    - replace a session's stream with different but validly encoded bytes (`verify.rs`'s `rewrite_the_stream`);
    - empty a session the way retention's `evict_one` does.
  - These helpers are the tests' second reading of the store. Each one uses its own SQL and the blob format directly, and never the product's archive module. A test that checks a product number against what is stored, such as `usage`, a rebuild or an export, would otherwise be checking the module against itself. Plan 2 rewrites these helpers' SQL when the storage changes, and the tests stay as they are.
  - Move every use of `sessions.blob` in SQL, `blob::read_all`, `blob::write`, `BlobReader` and `BlobHeader` in the listed test files onto the helpers. That includes their assertion messages, such as `capture.rs`'s "length(sessions.blob) must strictly decrease".
  - Every assertion keeps its meaning. In particular, `ingest.rs`'s incremental-against-one-shot comparison stays byte identity of the stored bytes and not equality of the streams. Its own comment gives the reason: a whole recompression would still decompress correctly.
  - `blob::BLOCK_SIZE` used as a size constant may stay.
- **Verify:**
  - `cargo test --workspace --all-features` passes.
  - `grep -nE 'blob FROM|SET blob|\bs\.blob|length\(blob\)|read_all|blob::write|BlobReader|BlobHeader' crates/terminus-core/tests/{capture,derive,ingest,lineage,pass,recall,recover,reindex,retention,schema,verify}.rs | grep -vE '^[^:]+:[0-9]+:\s*//'` prints nothing. Comment lines are filtered out. Today the same command prints 43 code lines.

### Task 4: Make the binary crate's tests reach the archive only through testkit

- **Files:** crates/terminus-core/src/testkit.rs, crates/terminus/tests/backfill.rs, crates/terminus/tests/cli.rs, crates/terminus/tests/corpus.rs, crates/terminus/tests/crash.rs, crates/terminus/tests/lifecycle.rs, crates/terminus/tests/recall_cli.rs
- **Action:**
  - Same rule as Task 3, applied to the binary crate's tests. Reuse Task 3's helpers, and add any missing one to `testkit` under the same independence rule.
  - The `Snapshot` structs in `crash.rs` and `backfill.rs` and `blobs` in `recall_cli.rs` compare every session's stored bytes through the comparable-bytes helper.
  - `corpus.rs` reads every session's stream through the stream helper.
  - `cli.rs`'s three damage sites use the damage helper.
  - `lifecycle.rs`'s `archived_bytes` stays the independent reconciliation target its comment says it is: one sum over what is stored, inside testkit, never `usage`'s own code path. Its export comparison reads the stream helper.
  - `crash.rs`'s `check_invariants` keeps all five properties. Property 1 checks each session the way `verify` checks it, either by calling `terminus_core::verify` over the store or through the helpers. Properties 3 and 4 take the committed length from the stream helper. Property 5 reads the byte before the watermark from that stream.
  - Both negative tests, `the_harness_catches_a_watermark_committed_outside_the_transaction` and `the_harness_catches_a_watermark_off_a_record_boundary`, must still be caught.
- **Verify:**
  - `cargo test --workspace --all-features` passes, including `--test crash` and both negative tests.
  - `grep -nE 'blob FROM|SET blob|\bs\.blob|length\(blob\)|read_all|BlobReader|BlobHeader' crates/terminus/tests/{backfill,cli,corpus,crash,lifecycle,recall_cli}.rs | grep -vE '^[^:]+:[0-9]+:\s*//'` prints nothing. Comment lines are filtered out.

## Notes

- **This plan was added in front of the CONTEXT plan shape.** The plan shape suggested three stages: the format and writer with ingest, then readers, `verify` and the other consumers, then docs. Plans 2, 3 and 4 follow that order. Plan 1 was added before them because of a fact in the code. Nine product files select `sessions.blob` directly: `excerpt`, `get`, `context`, `brief`, `reindex`, `verify`, `mechanical`, `export` and `usage`. On top of those, 17 test files read, damage or size it with raw SQL. The moment the column goes, all of them fail, so the switch could only stay green as one commit across about 30 files. Putting one module and one set of testkit helpers in between first keeps every commit green and keeps the switch reviewable.
- This also explains why the end-to-end path through block rows lands at plan 2, Task 3, rather than in the first two or three commits. Plan 1's module is the seam every reader passes through, so a reader that misses it fails a grep or a test here, before the storage changes.
