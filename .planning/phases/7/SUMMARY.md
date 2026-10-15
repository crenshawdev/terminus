---
phase: 7
status: complete
completed: 2026-10-14
---

# Phase 7: Blocks as rows - Summary

Each session's archive is stored as numbered 64 KB block rows with a BLAKE3 hash each, an append rewrites at most the tail block, readers fetch only the blocks a turn covers, `verify` names the block that fails, a format-1 store is refused with the clean-install steps, and the owner's store was rebuilt by a clean install.

## What shipped

- One archive module every reader goes through, and testkit as the only way tests reach the archive - `crates/terminus-core/src/store/archive.rs`, `crates/terminus-core/src/testkit.rs`
- The block codec: cut, hash and append session streams as block rows - `crates/terminus-core/src/blob/`
- Archive format 2: the `blocks` table, `sessions` without its `blob` column, the block size per session in `session_meta`, and the whole-session blob format removed - `crates/terminus-core/src/store/schema.rs`, `crates/terminus-core/src/ingest/mod.rs`
- The refusal of a format-1 store before any writable connection opens, naming the clean-install steps - `crates/terminus-core/src/store/open.rs`
- `verify` checks the block count and numbering, each block's bytes against its hash and the session checksum, and names the block in text and in `--json` - `crates/terminus-core/src/verify.rs`
- The crash harness restated over block rows, with complete blocks held unchanged across an append - `crates/terminus/tests/crash.rs`
- `Reads { block_rows, blocks_decompressed }` on search excerpts, `recall_get`, the context window and the brief - `crates/terminus-core/src/recall/`, `crates/terminus-core/src/inject/brief.rs`
- A test that every command refuses a format-1 store and leaves its bytes alone, and an export held to its transcript and to archive format 2 - `crates/terminus/tests/cli.rs`, `crates/terminus/tests/lifecycle.rs`
- DESIGN-BRIEF.md, docs/json-shapes.md and CHANGELOG.md describe block rows and the refused older store
- The owner's store rebuilt: 1953 sessions, `verify` 0 failed, `archive_format` 2, the old store moved aside to `~/terminus-format1/data`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 5d0e8de | read turn ranges through one archive module |
| 1 | 2 | 767fd42 | read whole sessions and archive sizes through the archive module |
| 1 | 3 | 74da35a | reach the archive through testkit in the core crate's tests |
| 1 | 4 | f2223ca | reach the archive through testkit in the binary crate's tests |
| 2 | 1 | e765509 | cut, hash and append session streams as block rows |
| 2 | 2 | 7ff134e | refuse a store in an older archive format and name the clean-install steps |
| 2 | 3 | 2a6fd01 | store each session as block rows and rewrite at most the tail on append |
| 2 | 4 | 8f50e45 | remove the whole-session blob format now that sessions live in block rows |
| 3 | 1 | d1e12d2 | check every block in verify and name the block that fails |
| 3 | 2 | 3fda441 | state the crash harness over block rows and hold complete blocks across an append |
| 3 | 3 | 660284a | count the block rows a search excerpt or get fetches, and decompress each once |
| 3 | 4 | b1139b2 | report the block rows the context window and the brief read |
| 4 | 1 | add30ca | prove every command refuses a format-1 store and leaves its bytes alone |
| 4 | 2 | 7a9d9ce | hold an export to its transcript and the manifest to archive format 2 |
| 4 | 3 | ba10c91 | describe block rows and the refused older store in the brief, the shapes and the changelog |
| 4 | 4 | - | the owner's clean install, run on 2026-10-14; no repository file changes |

## Deviations

- [deviation] Plan 1: `tests/index.rs` and `tests/snapshot.rs` planted sessions with `INSERT INTO sessions (session_key, session_no, blob)` outside plan 1's files. They read nothing from the archive, so they were left for plan 2, which removed the column; no such insert remains at HEAD.
- [deviation] Plan 2: Task 3's files did not cover `crates/terminus-core/tests/ingest.rs`, which had two assertions over the format-1 storage. The owner approved adding it to the lease with exactly those two edits (2a6fd01).
- [deviation] Plan 2: Task 4's grep could not come back empty from its leased files, because `crates/terminus-core/Cargo.toml:3` said "block-framed blobs". The owner approved changing only that line (8f50e45).
- [deviation] Plan 4 Task 4 step 1: a Claude Code session stayed open during the move. No effect: the format-1 binary no longer existed on the machine by then, so no hook could start it, and the new store reads `archive_format` 2.
- [deviation] Plan 4 Task 4 step 2: the 2026-10-05 8,662,008-byte format-1 binary is not saved. `~/terminus-format1/terminus` is the format-2 build, byte-identical to `target/release/terminus`. A first attempt around 16:10 copied the old binary there, an install at 16:11 replaced `~/.local/bin/terminus` with the link, and a second attempt's `cp` at 16:15 overwrote the saved copy. The owner accepted this.
- [deviation] Plan 4 Task 4 step 3: the data folder could not be moved into the same folder as the binary copy, because both are named `terminus`. It was moved to `~/terminus-format1/data` after `fuser` showed no process holding `terminus.db`.

## Open items

- No format-1 binary exists on the owner's machine. To read format 1, build the commit before 2a6fd01 and run it against a copy of `~/terminus-format1/data`, never the original.
- "Blob" wording outlived the blob format. DESIGN-BRIEF.md lines 60, 65, 70, the `### Blob format` heading at 72, 93 and 341, and line 432 says "Store format bumps rebuild derived tables only", which is now false for the archive format. `docs/json-shapes.md` lines 96, 120 and 291-292 say "session blob". `crates/terminus/src/main.rs`'s usage text says `verify` checks "every blob against its checksum". In the core crate: `lib.rs:3` ("The session blob is truth"), the `Display` text of `Error::BlobFormat` and `Error::BlobChecksumMismatch` (`error.rs:124,149`), `reindex.rs:1,3,88,125`, `observe/mod.rs:10`, `recall/search.rs:25,95,254`, `tests/retention.rs:585`, `tests/reindex.rs:402` and `tests/backfill.rs:130`.
- `cargo fmt --all --check` fails on six test files no plan in this phase touched: `crates/terminus-core/tests/inject_state.rs`, `crates/terminus-core/tests/recall_golden.rs`, and `crates/terminus/tests/hook.rs`, `mcp.rs`, `observations.rs`, `retention.rs`.
- `tests/corpus.rs` returns early without the private corpus, so its move to `testkit::stream` compiles but was not exercised.
- Two `verify` behaviours plan 3 chose: a run of consecutive missing block numbers is one failure naming its first and last block, and the summary counts failing sessions rather than failure lines and says "failed verification".
- `session_meta.block_size` is nullable with no default; readers and append refuse a null block size per session ("no block size recorded").

## Goal check

The commits deliver the goal. 2a6fd01 stores each session as block rows and rewrites at most the tail on append, and 8f50e45 removes the whole-session blob format. D-12's check holds over block rows: 3fda441 keeps the 19 complete blocks of the first-pass store unchanged across every append scenario, after a kill and after converging. Reads fetch only their blocks: 660284a and b1139b2 count block rows fetched, and plan 3's AC2 tests assert 1/1 for one block and k/k for a span of k blocks. Corruption is pinned to one block: d1e12d2 names the failing block in text and `--json`. The no-conversion rule holds: 7ff134e refuses a format-1 store, and add30ca proves 19 invocations against one exit non-zero with the database and WAL byte-identical. The full suite passed at ba10c91, 837 tests across 64 binaries, 0 failed (plan 4 report). The owner's clean install proved it on real data: 1953 transcripts archived with 0 failed, `terminus verify` reported "1953 session(s) checked, 0 failed", and `meta.archive_format` reads 2. Two things are missing. "Blob" wording remains in docs, comments and two error messages, as listed under Open items. And the format-1 binary D-02 meant to keep was not saved; git is the fallback.
