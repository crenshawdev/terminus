PLAN CHECKPOINT: structural
Plan: .planning/phases/7/PLAN-2.md
Tasks: 2 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Add the block codec | e765509 | `blob/block.rs`: `cut`, `write`, `append`, `checksum`, `check_block_size`, `BlockReader`; 13 codec tests in `tests/blob.rs`, `--test blob` 36/36 |
| 2 - Refuse an older archive format | 7ff134e | `Error::ArchiveFormatTooOld` from `read_versions` for both opens; install prints it and exits non-zero; `--test store_open` 17/17, `--test reindex` 12/12, `--test install` 27/27; both mutation checks fail the new tests as required |
| 3 - Store sessions as block rows | uncommitted | Implemented in the working tree, NOT committed. Files: store/schema.rs, store/open.rs, store/archive.rs, ingest/mod.rs, retention/apply.rs, verify.rs, testkit.rs, tests/compaction.rs, tests/schema.rs, tests/index.rs, tests/snapshot.rs. `cargo test --workspace --all-features --no-fail-fast`: 840 passed, 2 failed, both in tests/ingest.rs (below). Crash, reindex, verify, retention, schema and the new AC1 test all pass. The Verify grep prints nothing. AC1 mutation check done: making `prepare` rewrite every row fails the AC1 test ("the pass fetched 4 existing rows, more than the tail"). clippy (default and --all-features) and rustfmt are clean on the lease files. |

Checkpoint at Task 3 (implemented, uncommitted, suite red in one out-of-lease file).
Found: two tests in crates/terminus-core/tests/ingest.rs, outside the lease, assert the format-1 storage and fail on block rows exactly as the plan intends them to:
- ingest.rs:449, `the_stored_checksum_is_blake3_over_the_uncompressed_stream`: asserts `checksum == blake3(stream)`. D-04 makes the checksum BLAKE3 over the block hashes, so it now differs.
- ingest.rs:474, `a_pass_moves_the_session_its_turns_and_its_watermark_together`: the list of tables a pass moves now includes `blocks` (observed: sessions, blocks, session_meta, turns, turns_fts, entities, watermarks, runs).
Neither the plan's grep nor the previous survey caught these, because neither names the `blob` column.
Proposal: add crates/terminus-core/tests/ingest.rs to PLAN-2's `files:` and Task 3's Files line. In it, (1) compute the expected checksum independently as BLAKE3 over BLAKE3 of each 65,536-byte chunk of the source, and correct that test's two-line doc comment (432-433) to say so; (2) add "blocks" after "sessions" in the moved-tables list at line 474. Change nothing else in the file. Then commit Task 3 and continue with Task 4.
Alternative: none that meets Task 3's Verify. Keeping the whole-stream checksum contradicts D-04, and a pass that writes no `blocks` row contradicts D-03.

Deviations:
- [deviation] The plan asserted Task 3's `files:` (with the approved index.rs and snapshot.rs) cover the switch to block rows. In fact crates/terminus-core/tests/ingest.rs has two assertions over the format-1 storage that fail once it lands. I stopped before committing Task 3 and did not edit that file.
Open items:
- `verify`'s failure text still says "blob does not decompress". Plan 3 replaces verify's messages with per-block ones, so I left the string alone.
- `cargo fmt --all --check` fails at HEAD on six test files outside the lease (tests/inject_state.rs, tests/recall_golden.rs, crates/terminus/tests/hook.rs, mcp.rs, observations.rs, retention.rs). A `cargo fmt --all` I ran reformatted them; I restored all six with `git checkout` so none is modified.
- reindex.rs comments still describe "the blobs" as the input. reindex.rs is in the lease for Task 2 but not named in Task 3, so I left them for plan 4's docs pass.
- The new `session_meta.block_size` column is nullable, with no default, so the direct `session_meta` inserts in tests/retention.rs:79 and tests/pass.rs:777 still work. Readers and append refuse a null block size per session ("no block size recorded").
