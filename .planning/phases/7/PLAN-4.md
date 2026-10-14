---
phase: 7
plan: 4
requirements: [STOR-08, STOR-09, STOR-10]
files:
  - crates/terminus/tests/cli.rs
  - crates/terminus/src/cmd/export.rs
  - crates/terminus/src/cmd/usage.rs
  - crates/terminus/tests/lifecycle.rs
  - DESIGN-BRIEF.md
  - docs/json-shapes.md
  - CHANGELOG.md
---

# Phase 7: Blocks as rows - Plan 4

## Goal

The phase goal: each session's archive is stored as numbered blocks, one row per block, with the session checksum a hash over the block hashes, and there is no conversion tool. Any existing store is refused, and the owner's store is rebuilt by a clean install.

This plan proves the refusal on every command and proves the consumers, `export` and `usage`, on block rows. It brings the three documents in line, and it ends with the owner's clean install, which is the phase's last success criterion.

## Must be true when done

- **Every command refuses a format-1 store (AC4).** Each command that opens the store exits non-zero against a genuine format-1 store, which keeps its `blob` column, has no block table and has `meta.archive_format = 1`. Each one prints a message naming the clean-install steps. Afterwards the store's database file, and its WAL, are byte-for-byte unchanged, and no block table has appeared in it.
- **`export` and `usage` work on block rows (AC5).** On a format-2 store, `terminus export` writes each session captured under `full` byte-identical to the transcript it came from, and its manifest says `archive_format` 2. `terminus usage` reports archive bytes equal to the sum of the block rows' lengths. `terminus reindex` still gives the same query results as before a rebuild. `cargo test --workspace --all-features` passes.
- **The documents describe block rows (AC6).** `DESIGN-BRIEF.md`, `docs/json-shapes.md` and the `CHANGELOG.md` Unreleased section describe block rows, the per-session block size, `verify`'s `block` field and the refusal of an older archive format. Neither `DESIGN-BRIEF.md` nor `docs/json-shapes.md` still says "block-framed blob" or "rebuilds derived tables only".
- **The owner's store is rebuilt (AC7).** On the owner's machine, with the old data folder and a copy of the current binary set aside, a clean install archives every session whose transcript is still on disk, and `terminus verify` then reports no failure.

## Context

- Plan 4 of 4. It runs after plan 3 and shares files with plans 1, 2 and 3.
- This plan is bound by D-01, D-02 (move aside, never purge), D-06 (`decisions` and `labels` start empty and nothing copies them), D-07, D-08 (the brief's lines 57, 73, 94 and 98), D-16 and D-17.
- Snapshots (`VACUUM INTO`) and `data move` need no change (D-17), and their existing tests are the check.
- `REQUIREMENTS.md` already carries STOR-08, STOR-09 and STOR-10 and the extended list of replaced rows; commit `712843b` landed them. This plan does not edit it.
- Out of scope: the release (phase 8), the README (phase 9), a conversion tool, and copying `decisions` or `labels` into the new store.

## Tasks

### Task 1: Prove that every command refuses a genuine format-1 store and leaves it untouched

- **Files:** crates/terminus/tests/cli.rs (anchor: `a_store_one_archive_format_ahead_is_refused_with_both_versions_named`)
- **Action:**
  - Add a test beside the newer-format one. It builds a format-1 store by hand, without `CREATE_SQL`: a `sessions` table that still has its `blob` column, with one session row; no block table; and `meta` with `archive_format` set to `1` and `derived_schema` current. It keeps a non-empty WAL the same way the newer-format test does.
  - Run each command that opens the store against it, one per run, on a fresh copy if a command's own lock or setup needs one. Give each command whatever valid arguments it needs to get as far as opening the store:
    - `ingest`, and `ingest <path>`;
    - `backfill`;
    - `search <word>`, `show <id>`, `sessions`;
    - `verify`, `reindex`, `status`, `compact`, `usage`, `export <dir>`;
    - `stats`, `observations`, `observations regenerate`, `replay`;
    - `retention --dry-run`;
    - `doctor`;
    - `install`, with `HOME` and the stable path pointed into the test's temporary directory the way `tests/install.rs` does.
  - Assert for each one:
    - its exit code is not 0;
    - its stdout and stderr together name moving the data folder and the binary aside and rerunning `install.sh`. `doctor` prints its report on stdout and exits 1.
    - `terminus.db` and its WAL are byte-identical to what they were before;
    - `export` created no destination.
  - At the end, assert that the store has no block table and that `archive_format` still reads `1`. That proves `bring_forward` never ran (D-01).
  - Leave four commands out, and say why in the test's doc comment:
    - `hook` returns 0 whatever happens, and its spawned ingest is the case `ingest` covers (INJ-06, ING-10).
    - `mcp` answers with a reason rather than exiting (RCL-10).
    - `uninstall` and `data move` never open the store.
- **Verify:**
  - `cargo test -p terminus --all-features --test cli` passes, and the new test runs all 19 invocations listed above.
  - Deleting the older-format arm from `read_versions` makes the new test fail.

### Task 2: Prove `export` and `usage` on block rows, and say so in their docs

- **Files:** crates/terminus/src/cmd/usage.rs (anchor: module doc), crates/terminus/src/cmd/export.rs (anchors: `write_export`'s evicted comment, the manifest `archive_format` comment), crates/terminus/tests/lifecycle.rs (anchors: `an_export_is_the_archived_streams_byte_for_byte_with_a_manifest_over_them`, `archived_bytes`)
- **Action:**
  - In `usage.rs`'s module doc, the ARCHIVE table is now each session's block byte lengths summed, and it reconciles to the sum over the block table (D-17), not to `length(sessions.blob)`.
  - In `export.rs`, the evicted-session comment says an evicted session has no block rows and is written as an empty file. The manifest's `archive_format` is still the store's own answer, which is 2 on every store this build opens.
  - In `lifecycle.rs`:
    - The export test keeps its comparison against the archive, and also asserts that each exported session captured under `full` is byte-identical to the transcript fixture it was ingested from (AC5). For a fixture that ends on a partial record, compare up to the end of its last complete record, the session's watermark.
    - The manifest's `archive_format` equals `terminus_core::ARCHIVE_FORMAT`, which is 2.
    - The `usage` reconciliation still checks `archive_bytes` against testkit's independent sum over the block table, never against `usage`'s own code path.
- **Verify:**
  - `cargo test -p terminus --all-features --test lifecycle --test datamove` passes.
  - `cargo test -p terminus-core --all-features --test snapshot --test reindex` passes.
  - `reindex_rebuilds_the_derived_tables_to_byte_identical_query_output` in `tests/cli.rs` passes on a format-2 store.
  - `grep -n 'sessions.blob' crates/terminus/src/cmd/usage.rs crates/terminus/src/cmd/export.rs` prints nothing.

### Task 3: Bring the design brief, the JSON shapes and the CHANGELOG in line with block rows

- **Files:** DESIGN-BRIEF.md (anchors: the storage table's `sessions` row, `### Blob format`, `### Truth and derived data`'s "The archive table never migrates", `### Durability`'s first two bullets, the ingest steps, the `reindex`, `verify` and `observations regenerate` command rows, the store format version bullet), docs/json-shapes.md (anchors: `## Exit codes`, `### verify`, the `usage` ARCHIVE bullet), CHANGELOG.md (anchor: `## [Unreleased]`)
- **Action:**
  - First read `~/.claude/john-voice.md` in full and follow it (CLAUDE.md). Its "README, project docs" rules govern the CHANGELOG. Its "Design docs" rules govern the brief: describe the system as it stands now, never as a change log.
  - Quote only what the code from plans 1 to 3 actually does and prints.
  - **`DESIGN-BRIEF.md`.** Change exactly the nine places D-08 names. No other line changes.
    - **The storage table's `sessions` row (line 57).** It becomes `session_key` to `session_no`, plus a row for the block table: `(session_no, block_no)` to the compressed bytes and the BLAKE3 hash of the uncompressed bytes.
    - **The block format paragraph (line 73).** Each session's stream is cut into blocks of the size its `session_meta` row records, 64 KB today. Each block is compressed with zstd on its own and stored as one row with its hash. A turn read fetches and decompresses only the blocks its range covers, and an append rewrites at most the last block.
    - **"The archive table never migrates" (line 94).** A store in an archive format older or newer than the binary's is refused, with the clean-install steps, and nothing converts it. Only an older derived schema rebuilds the derived tables.
    - **The first durability bullet (line 98).** A BLAKE3 hash per block and a session checksum over the block hashes, in `session_meta`, independent of SQLite's own integrity checks, so corruption is pinned to a named session and block.
    - **The second durability bullet (line 99).** Replace "incremental blob verification during ingest" with what an append checks (D-10): the stored block hashes against the session checksum, and the tail block against its own hash.
    - **The ingest steps (lines 130 and 131).** Step 4 cuts the stream into blocks and compresses each one; step 5 commits the new block rows, the replaced tail row, turns, FTS, entities and the watermark in one transaction.
    - **The `reindex` and `verify` rows (lines 456 and 457).** `reindex` rebuilds the derived tables from the block rows. `verify` checks every block against its hash and each session checksum, and names the session and block of a failure.
    - **The `observations regenerate` row (line 464).** It recomputes stored observations from the archived block rows.
    - **The store format bullet (line 498).** The binary refuses a store whose archive format differs from its own, newer or older, and an older derived schema rebuilds the derived tables. A product bump never implies a store rebuild.
  - **`docs/json-shapes.md`.**
    - **`### verify`.** Each failure carries `block`: the failing block's number, or null for a session-level failure. The text line reads `<session_key>: block N: ...`.
    - **The `usage` ARCHIVE bullet.** `archive_bytes` is the sum of each session's block byte lengths over the sessions a read path may see (D-17).
    - **The exit-code-1 row.** It names a store whose archive format this build refuses.
  - **`CHANGELOG.md` Unreleased.** Add entries saying that:
    - each session is stored as block rows, with a hash per block and a session checksum over the hashes;
    - the block size is recorded per session, at 64 KB;
    - an append rewrites at most the last block, and a turn read fetches only its blocks;
    - `verify` names the session and block of a failure, with `block` under `--json`;
    - `usage`'s archive bytes are the block rows' bytes;
    - a store written by an earlier build is refused with the clean-install steps and is not converted.
- **Verify:**
  - `grep -nE 'block-framed blob|rebuilds derived tables only' DESIGN-BRIEF.md docs/json-shapes.md` prints nothing.
  - `grep -niE 'blob' DESIGN-BRIEF.md` finds none of the nine places D-08 names, and line 498 no longer says an older store format rebuilds.
  - `grep -n '"block"' docs/json-shapes.md` finds the `verify` shape.
  - Reading the files shows that `DESIGN-BRIEF.md` states the per-session block size and the refusal, and that the CHANGELOG Unreleased section names block rows, the per-session block size, `verify`'s `block` field and the refusal.
  - `git diff --stat` shows only these three files.

### Task 4: Rebuild the owner's store with a clean install

- **Files:** none. This is a step on the owner's machine, and no repository file changes.
- **Action:** After Tasks 1 to 3 have landed, the owner runs a clean install (D-02, D-07).
  1. **Close every Claude Code session.** While the data folder is gone, no hook should start the old binary, because it would create a fresh format-1 store in the empty folder.
  2. **Copy the current binary aside.** Copy `~/.local/bin/terminus`, the plain 8,662,008-byte file from 2026-10-05 and the only binary that reads format 1, to a folder of the owner's choosing. It carries the stable-binary marker (checked 2026-10-14), so `terminus install` accepts it at the stable path and replaces it with the link.
  3. **Move the data folder aside.** Move `~/.local/share/terminus/`, 9.4 GB with its snapshots, to the same folder. Purge nothing. 4,636 of the 6,570 archived sessions exist nowhere else (D-02).
  4. **Install from a local build.** No release exists until phase 8, so `install.sh` has nothing to download. In `/code/terminus`, run `cargo build --release && target/release/terminus install`.
  5. **Wait for the backfill.** `terminus status` shows its progress.
  6. **Check the result.** Compare the archived session count with the transcript count the install's backfill estimate printed, then run `terminus verify`.
  - Under D-06, `terminus stats` then has no decisions, because `decisions` and `labels` start empty in the new store.
- **Verify:** human-verify. This needs the owner's real store, which the move changes. After step 6 the owner should see four things:
  - the session count equals the install receipt's transcript count, 1,934 as counted on 2026-10-14, or that day's count;
  - `terminus verify` exits 0 and prints "N session(s) checked, 0 failed";
  - `sqlite3 ~/.local/share/terminus/terminus.db "SELECT value FROM meta WHERE key='archive_format'"` prints `2`;
  - the moved-aside folder still holds the old `terminus.db` and the binary copy.

## Notes

- **AC4 says "every command".** Task 1 covers the 19 invocations that open the store and can exit non-zero, `install` among them since the owner's call on 2026-10-14 (D-07). `hook` and `mcp` are left out by their own contracts, which are listed in the task, and `uninstall` and `data move` never open the store.
- **The usage text.** `crates/terminus/src/main.rs`'s usage text still says `verify` checks "every blob against its checksum". No decision covers it, so no task changes it.
