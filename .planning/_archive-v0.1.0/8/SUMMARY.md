---
phase: 8
status: complete
completed: 2026-08-22
---

# Phase 8: Retention And Lifecycle - Summary

Age-based retention (`keep`/`evict`/`delete`, off unless configured) applied as a bounded step at the end of every ingest pass, plus `compact`, `usage`, `export`, `data move`, scheduled `VACUUM INTO` snapshots, and three capture modes that elide record bodies before the blob.

## What shipped

- `[retention]` policy, global and per project, resolved by `Config::retention_for` / `retention_selects_nothing` - `crates/verbatim-core/src/config.rs`
- One evaluation function naming what retention would touch, bounded at `MAX_PER_PASS = 100` per action, oldest first - `crates/verbatim-core/src/retention/mod.rs`
- `evict_one` (blob to `x''`, `is_evicted = 1`, every row kept) and `delete_one` (child-first, one transaction per session) - `crates/verbatim-core/src/retention/apply.rs`
- The retention step between `observe_new` and `record_pass`, under the lock the pass already holds - `crates/verbatim-core/src/ingest/pass.rs`
- Evicted-session survival: `verify` counts and never fails one, `reindex` takes a per-session `clear_derived` path instead of `DROP TABLE` and reports `Rebuilt::preserved`, `prepare` returns no work before opening the file - `crates/verbatim-core/src/{verify,reindex,ingest/mod}.rs`
- `verbatim retention --dry-run --json` - `crates/verbatim/src/cmd/retention.rs`
- `verbatim compact` (`VACUUM` + `wal_checkpoint(TRUNCATE)` under the lock) - `crates/verbatim/src/cmd/compact.rs`
- `verbatim usage` (archive bytes by project and by month, plus a `dbstat` footprint reconciling to the file) - `crates/verbatim/src/cmd/usage.rs`
- `verbatim export` (one `.jsonl` per session plus a `manifest.json` carrying the unredacted-contents notice) - `crates/verbatim/src/cmd/export.rs`
- `store::snapshot::{take,prune}` - `VACUUM INTO` a `snapshots/` subdirectory, temp name then rename, keep the newest few - `crates/verbatim-core/src/store/snapshot.rs`
- `[snapshot]` (on / 24 h / 3 by default) driven by `pass::roll()` off a `meta` timestamp, outside the lock, every failure a note - `crates/verbatim-core/src/{config,ingest/pass}.rs`
- A `data-location` pointer read by one resolver below `VERBATIM_DATA_DIR` and above the platform arm, and `verbatim data move <path>` - `crates/verbatim-core/src/store/open.rs`, `crates/verbatim/src/cmd/data.rs`
- `[capture].mode` (`full`/`lean`/`minimal`), `session_meta.capture_mode`, and a `capture` module eliding `toolUseResult`/`attachment` bodies into a `{"verbatimElided": <bytes>}` mark before the blob - `crates/verbatim-core/src/capture.rs`
- The file/stored coordinate split in `prepare` (`resume_offset` on file bytes, `stream_base` on stored bytes) and the two recovery checks that assumed they were the same - `crates/verbatim-core/src/{ingest/mod,recover}.rs`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 336058f | `[retention]` table, per-project map, `RetentionAction`/`RetentionPolicy` |
| 1 | 2 | 8ff04d5 | `retention::evaluate` -> `Selection`, `MAX_PER_PASS = 100` |
| 1 | 3 | 80d7d5d | `retention::apply` and the non-failing `retain` entry point |
| 1 | 4 | 2528a85 | The pass step and its `runs.error` note lines |
| 2 | 1 | c4804f3 | `is_evicted` arms in `verify` and on the ingest append path |
| 2 | 2 | 70c4b4d | `reindex` preserves an evicted session instead of dropping it |
| 2 | 3 | d0080ca | `verbatim retention --dry-run` |
| 2 | 4 | ffa8123 | `retention` joins `DATA_COMMANDS` and `docs/json-shapes.md` |
| 3 | 1 | 970f2c4 | `verbatim compact` |
| 3 | 2 | 1a4c529 | `verbatim usage` |
| 3 | 3 | 305b303 | `verbatim export` with its manifest |
| 3 | 4 | 0420c48 | Three commands join the swept `--json` contract |
| 4 | 1 | 3f94aca | `store::snapshot` `take` and `prune` |
| 4 | 2 | 8bfaba9 | `[snapshot]` and the scheduled `pass::roll()` |
| 4 | 3 | 54b1217 | The `data-location` pointer in `store::open::data_dir` |
| 4 | 4 | 058eacd | `verbatim data move <path>` |
| 5 | 1 | 454840b | `[capture].mode` and `session_meta.capture_mode` |
| 5 | 2 | 03c71a0 | The `capture` module: `elide`, `elide_stream`, the mark |
| 5 | 3 | 9ba3fbe | Elide before the blob; the two coordinate systems split |
| 5 | 4 | f0397dd | Recovery and `prefix_matches` stop reading a file offset as stored bytes |

## Deviations

- [deviation] Plan 4, task 2 - the executor halted `blocked` on a lease refusal: `pass::run_with` writing a `meta` row that `backfill::run_with` deliberately does not made `crates/verbatim-core/tests/backfill.rs` need one word (`meta`) added to its `POST_WALK_TABLES` exclusion list, and that file was not in PLAN-4's `files:`. The orchestrator amended the lease and re-dispatched from task 2; the change is one word plus a doc line saying why that table is excluded for a different reason than the other three. Landed in `8bfaba9`.
- [deviation] Plan 5 - PLAN-5's `files:` named `crates/verbatim-core/src/ingest/mod.rs` but not `ingest/pass.rs` or `ingest/backfill.rs`. `prepare` and `ingest_locked` are the two functions the configured mode has to reach, and their only callers outside `mod.rs` are `pass::walk` and `backfill`'s worker, so the lease as written would have left `[capture].mode` honoured by `verbatim ingest <path>` and silently ignored by the hook-driven pass - the product's actual ingest path - and by `verbatim backfill`. The executor added both paths to the plan frontmatter with the reason in the plan's Notes; one value is threaded through each file and nothing else changes. Landed across `454840b`..`f0397dd`.
- [deviation] Plan 5's first dispatch was stopped by the user partway through task 4, leaving uncommitted work that did not compile (E0618 in `tests/capture.rs`: a local `object` binding shadowing a function of the same name). The continuation reviewed that work rather than rebuilding it - both source changes kept as written, the tests kept with one edit folding a stray second `impl Bench` block into the existing one. Landed in `f0397dd`.

## Open items

- `verbatim reindex` does not SAY how many sessions it preserved. The count is on `Rebuilt::preserved` and documented, but `crates/verbatim/src/cmd/reindex.rs` was in no plan's lease, so the stderr line, the `--json` `data` object, `DATA_COMMANDS` and `docs/json-shapes.md` are untouched.
- `crates/verbatim-core/src/parse/mod.rs:3-5` still says the ingest hands the blob bytes identical to the source without qualification. D-05 narrows that to `full` mode. One sentence, one file, no behaviour.
- `verify` skips an evicted session entirely rather than asserting the one thing eviction guarantees about it - that its blob is empty - so it has a class of row it never checks. (`risk_surface` plan-2, downgraded from medium: `evict_one` is the only writer of the flag and it empties the blob in the same transaction, so the divergent state takes tampering or corruption to reach.)
- `retention::delete_one` does not re-check `transcript_is_gone` before removing a session; the precondition is evaluated once, in `evaluate`. Archived bytes are recoverable - the watermark row goes in the same transaction so `discover` re-ingests from offset 0 - but the `observations` rows are not. (`risk_surface` plan-1, downgraded from high on that basis.)
- The `verbatimElided` figure is the canonical reserialized size of the dropped subtree, not the source bytes it occupied, and the doc comment does not say so. Nothing in production reads it today. (`risk_surface` plan-5, downgraded from medium.)
- `cmd::status` and `cmd::uninstall` still hold private copies of the three-file footprint rule that `cmd::mod::footprint`/`human` now also expresses.
- `cargo fmt --check` reports pre-existing diffs in `crates/verbatim-core/tests/{retention,verify}.rs` and `crates/verbatim/tests/{hook,retention}.rs`, from earlier plans. Every file phase 8 touched is `rustfmt` clean.
- Several phase-8 test files are `#![cfg(feature = "testkit")]`, so a `cargo test --test <name>` without `--features testkit` compiles an empty binary and reports green. Some plans' `Verify:` commands are written in that vacuous form; every count in the reports came from a `--features testkit` run.
- `verbatim data move` skips the `LOCK` file in the copy and creates an empty one at the destination. The file's contents carry no meaning, only the OS lock attached to it, and copying a locked file is a read Windows refuses.

## Goal check

The twenty commits plausibly deliver the goal, and each half of it has a named artifact rather than an assertion. Aged: `retention::evaluate` and `apply` exist and run from `pass::run_with` (`crates/verbatim-core/src/ingest/pass.rs`), and criterion 1 - retention unset selects nothing - is enforced structurally by `Config::retention_selects_nothing` short-circuiting before any query, tested in `crates/verbatim-core/tests/retention.rs`. Shrunk: `verbatim compact` was falsified by removing its `wal_checkpoint(TRUNCATE)`, which produced `reclaimed_bytes` of -428,512 (reports/plan-3.md, task 1). Measured: `usage`'s `dbstat` footprint was reconciled against a live 1,114,308,608-byte store, short by exactly the one 4,096-byte lock page SQLite places past 1 GiB, which has a row of its own. Snapshotted, exported and relocated: `store::snapshot::take`, `cmd::export` with its manifest, and `cmd::data` with the `data-location` pointer, the last falsified by disabling the pointer arm in `data_dir` (2 of 5 `tests/datamove.rs` cases fail). How much of each record to keep: the three capture modes measure 28,785 / 2,640 / 2,436 uncompressed bytes on the plan fixture, and `the_blob_reproduces_the_transcript_byte_for_byte` still passes in `tests/ingest.rs`, which is criterion 7's byte-for-byte half. `cargo test --workspace --all-features` is 61 suites, 732 tests, 0 failures at `f0397dd`. Two criteria are only partly evidenced from this phase's own work and should be checked at UAT rather than taken on the commits: criterion 2's `recall_get` half rests on `Record::body_evicted`, which `crates/verbatim-core/src/recall/get.rs:192` reads off `session_meta.is_evicted` and `tests/recall.rs:1752` asserts, but no test in this phase drives the MCP tool end to end after a real eviction; and criterion 6 - a snapshot taken DURING an active ingest opening as a valid store and passing `verify` - is covered by `tests/snapshot.rs` taking a snapshot against a live connection, not by a concurrent writer. Nothing in the phase looks missing; those two are thin, not absent.
