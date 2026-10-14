PLAN CHECKPOINT: structural
Plan: .planning/phases/7/PLAN-2.md
Tasks: 3 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Add the block codec | e765509 | Committed by an earlier run (see plan-2.2.md) |
| 2 - Refuse an older archive format | 7ff134e | Committed by an earlier run (see plan-2.2.md) |
| 3 - Store sessions as block rows | 2a6fd01 | Earlier run's working-tree implementation committed as is, plus the two approved edits in tests/ingest.rs: the checksum test now computes BLAKE3 over the BLAKE3 of each 65,536-byte chunk, with its doc comment to match, and "blocks" follows "sessions" in the moved-tables list. `--test ingest` 13/13. clippy (default and --all-features), cargo check and rustfmt clean. The Verify grep prints nothing. The earlier run's full suite had only these two failures; the full suite has not been rerun yet. |
| 4 - Remove the whole-session blob format | uncommitted | Implemented in the working tree, NOT committed. blob/reader.rs and blob/writer.rs deleted (`BlobHeader`, `BlockEntry`, `MAGIC`, `HEADER_LEN`, `TABLE_ENTRY_LEN`, `BlobReader`, the blob-level `write`/`append`, `Written`, `Appended`, `finish`, `finish_parts`, `read_all` are gone). blob/mod.rs re-exports the codec's `append`, `write`, `Written`, so `blob::append` and `blob::write` now name the codec and the comments in capture.rs, retention/mod.rs and elsewhere that cite `blob::append` stay true; ingest/mod.rs calls them by that path. blob/mod.rs module doc and constant docs describe block rows. error.rs `BlobFormat` and `BlobChecksumMismatch` docs rewritten; lib.rs crate doc no longer says "block-framed"; tests/recall.rs doc comment no longer names `BlobReader`. tests/blob.rs: the whole-blob section is gone (the header-only tests the plan names, and the rest, whose every call was to a deleted API and which the codec tests already repeat); the five range-reading tests the codec section did not repeat (one block, three blocks, whole stream, every boundary, empty range) were carried over to block rows. `--test blob` 18/18; `--test ingest` 13/13, `--test compaction` 13/13, `--test recall` 50/50. clippy (default and --all-features), cargo check and rustfmt clean. The Verify grep prints one line: `crates/terminus-core/Cargo.toml:3:description = "Terminus archive core: store, block-framed blobs, ingest."` |

Checkpoint at Task 4 (implemented, uncommitted).
Found: Task 4's Verify says `grep -rnE 'BlobHeader|BlobReader|VBLB|finish_parts|block-framed' crates/` prints nothing. The only remaining hit is the package description in crates/terminus-core/Cargo.toml line 3, which is not in PLAN-2's `files:` (nor in plan 3's or plan 4's).
Proposal: add crates/terminus-core/Cargo.toml to PLAN-2's `files:` and Task 4's Files line, and change line 3 to `description = "Terminus archive core: store, block rows, ingest."`. Change nothing else in the file. Then commit Task 4 and run the full suite.
Alternative: commit Task 4 without the Cargo.toml line and leave the grep failing on that one line until a later plan picks it up. That redefines the Verify, so I did not take it.

Deviations:
- [deviation] (earlier run) The plan asserted Task 3's files covered the switch to block rows. crates/terminus-core/tests/ingest.rs had two assertions over the format-1 storage. Checkpointed; the owner approved adding it to the lease with exactly the two edits above.
- [deviation] Task 4's Verify asserts its grep can print nothing from the files in the lease. crates/terminus-core/Cargo.toml:3 says "block-framed blobs" and is outside the lease. Stopped before committing Task 4; the file is not edited.
Open items:
- `verify`'s failure text still says "blob does not decompress". Plan 3 replaces verify's messages with per-block ones.
- tests/verify.rs:346 still cites the deleted `blob::read_all` in a doc comment ("finds no header in zero bytes"). That file is outside this lease and in plan 3's, which restates those tests.
- tests/retention.rs:585 says `blob::append` "has no header to parse" out of an evicted session. `blob::append` still exists (it is the codec now), but the header reason is stale. Outside the lease.
- The `Display` text of `Error::BlobFormat` ("malformed session blob") and `Error::BlobChecksumMismatch` ("session blob failed its recorded checksum") still says "session blob". Task 4 asks only for the docs, and tests may match the text, so I left the messages alone.
- lib.rs's first sentence still says "The session blob is truth". Task 4 changes only the "block-framed" sentence; left for plan 4's docs pass.
- recall/excerpt.rs, recall/get.rs, ingest/mod.rs (the `BlobReader` sentences) and the tests/compaction.rs module doc needed no change: plan 1 and Task 3 had already corrected them.
- `cargo fmt --all --check` fails at HEAD on six test files outside the lease (tests/inject_state.rs, tests/recall_golden.rs, crates/terminus/tests/hook.rs, mcp.rs, observations.rs, retention.rs). Left untouched.
- reindex.rs comments still describe "the blobs" as the input. reindex.rs is not named in Task 3 or 4; left for plan 4's docs pass.
- The new `session_meta.block_size` column is nullable with no default, so the direct `session_meta` inserts in tests/retention.rs:79 and tests/pass.rs:777 still work. Readers and append refuse a null block size per session ("no block size recorded").
