PLAN CHECKPOINT: structural
Plan: .planning/phases/7/PLAN-2.md
Tasks: 2 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Add the block codec | e765509 | `blob/block.rs`: `cut`, `write`, `append`, `checksum`, `check_block_size`, `BlockReader`; 13 codec tests in `tests/blob.rs`, `--test blob` 36/36 |
| 2 - Refuse an older archive format | 7ff134e | `Error::ArchiveFormatTooOld` from `read_versions` for both opens; install prints it and exits non-zero; `--test store_open` 17/17, `--test reindex` 12/12, `--test install` 27/27; both mutation checks fail the new tests as required |

Checkpoint at Task 3 (not started, nothing uncommitted).
Found: dropping `sessions.blob`, which "Must be true when done" requires (`sessions` has only `session_key` and `session_no`), breaks three INSERTs in two test files outside the plan's `files:` lease. Each names the `blob` column directly:
- crates/terminus-core/tests/index.rs:203 `INSERT INTO sessions (session_key, session_no, blob) VALUES ('k', 0, x'00')`
- crates/terminus-core/tests/index.rs:756 (same statement)
- crates/terminus-core/tests/snapshot.rs:125 `INSERT INTO sessions (session_key, session_no, blob) VALUES ('mid-pass', 9999, x'00')`
Task 3's Verify grep (`blob FROM|SET blob|\bs\.blob|length\(s\.blob\)|blob +BLOB +NOT`) does not match this form, which is likely why planning missed them.
Proposal: add crates/terminus-core/tests/index.rs and crates/terminus-core/tests/snapshot.rs to PLAN-2's `files:` and drop `blob` (and its value) from those three column lists. Nothing else in either file changes. Neither test reads archive bytes: index.rs only needs a parent `sessions` row for `derive_turn`, and snapshot.rs only needs an uncommitted `sessions` row.
Alternative: none that meets the criterion. Keeping a `blob` column contradicts D-03 and the plan's first must-be-true.

Survey for the continuation (read, not run). Every other out-of-lease test that touches the archive goes through testkit (`stored`, `stored_len`, `stored_total`, `stream`, `damage`, `rewrite_stream`, `evict`, `plant_session`) or through `verify`, so it keeps working if testkit keeps each helper's meaning over block rows. That covers crates/terminus/tests/crash.rs, recall_cli.rs, backfill.rs, cli.rs, lifecycle.rs, corpus.rs and crates/terminus-core/tests/verify.rs, recover.rs, capture.rs, ingest.rs, pass.rs, retention.rs, derive.rs, recall.rs, lineage.rs. tests/retention.rs:79 and tests/pass.rs:777 insert `session_meta` rows directly without a block-size column, so the new column must be nullable or carry a default, or those out-of-lease inserts fail.

Deviations:
- [deviation] The plan asserted Task 3's `files:` cover the switch to block rows. In fact tests/index.rs (2 statements) and tests/snapshot.rs (1 statement) also name `sessions.blob` and fail once the column is dropped. I stopped before Task 3 and did not edit either file.
Open items: none
