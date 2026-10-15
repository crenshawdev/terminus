---
phase: 12
plan: 3
requirements: ["#33", "#14", "#41", "#42"]
files:
  - crates/terminus-core/src/store/open.rs
  - crates/terminus-core/src/ingest/mod.rs
  - crates/terminus/tests/status.rs
  - crates/terminus/src/cmd/reindex.rs
  - crates/terminus/tests/cli.rs
  - docs/json-shapes.md
  - docs/commands.md
  - crates/terminus-core/src/verify.rs
  - crates/terminus-core/tests/verify.rs
---

# Phase 12: Quick Issue Fixes - Plan 3

## Goal

The phase goal: twelve open GitHub issues that are small enough to fix inside one function or one doc section are fixed and closed before the release is announced, which cuts the open list from 39 to 27. #17 gets its type check here, and its second case moves to phase 13.

This plan fixes the four store and command issues. `initialize` runs in an IMMEDIATE transaction (#33). A single-file ingest records its committed and failed counts (#14). `reindex` prints its preserved count (#41). `verify` fails an evicted session that still has block rows (#42).

## Must be true when done

- A fresh store's `initialize` can run after another open has already committed the tables and while that open's connection holds the write lock. When it does, it waits for the lock and succeeds instead of failing with "database is locked". The test that shows this fails when `initialize` is switched back to a deferred transaction.
- After `terminus ingest <path>` on one transcript, `terminus status` prints `files        1 walked, 1 committed, 0 failed`.
- `terminus reindex --json` carries an integer `preserved` in its success document and in its lock-held document, where it is 0, so both have the same `data` keys. `docs/json-shapes.md` and `DATA_COMMANDS` list it, and the `terminus reindex` entry in `docs/commands.md` says the count is printed. Without `--json`, the count is printed on stderr only when it is above 0, and a store with no evicted sessions prints exactly what it prints today.
- `terminus verify` reports an evicted session that still has block rows as one failure with `block` null, saying how many rows remain, and exits 1. An evicted session with no rows is still never named.
- `cargo test -p terminus-core --all-features` and `cargo test -p terminus --all-features` pass.

## Context

- D-01 and D-08 to D-11 in `.planning/phases/12/CONTEXT.md` bind this plan exactly. A failed single-file ingest still writes no `runs` row, and rows already written with 0 committed are not backfilled. `verify --json` gains no field. `DERIVED_SCHEMA` is not bumped.
- `preserved` is a key scripts will read, so once shipped it is never renamed (D-01).
- Every task leaves `cargo clippy --workspace --all-features --all-targets -- -D warnings` and `cargo fmt --all --check` clean. Both are clean on the tree today.

## Tasks

### Task 1: Run initialize in an IMMEDIATE transaction (#33)

- **Files:** crates/terminus-core/src/store/open.rs (`Store::initialize` and a new tests module)
- **Action:** Open `initialize`'s transaction with `Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)`, the form `bring_forward` already uses. Both names are already imported in this file. Re-read nothing under the lock. Every statement in `CREATE_SQL` is `IF NOT EXISTS` and the meta write is `INSERT OR REPLACE`, so running both on a store another open just initialized is already correct (D-08). Extend `initialize`'s doc comment with why the transaction is IMMEDIATE. When another open has already created the tables, the `CREATE TABLE IF NOT EXISTS` statements only read, so a deferred transaction holds a read lock by the time the meta `INSERT` needs to write. SQLite does not call the busy handler for an upgrade from a read to a write, so the `INSERT` failed at once with "database is locked" instead of waiting `BUSY_TIMEOUT_MS`. Add a `#[cfg(test)]` module to `open.rs` with a test that reproduces the losing open deterministically. The test calls `initialize` directly, because `Store::open` has no seam between `inspect` and `initialize`. Run the steps in this order:
  1. The winning open is `Store::open` of a fresh temporary data directory.
  2. A second `Store::open` of the same directory gives the losing open's `Store`. It takes the existing-store path and writes nothing.
  3. A separate `rusqlite::Connection` on the same file, standing in for the winner's backfill, begins an IMMEDIATE transaction, writes a row, and signals that it holds the lock. It commits about one second later from its own thread, well inside the 5-second busy timeout.
  4. Only after that signal, call `initialize` on the losing `Store`. It must return `Ok`, and `meta` must then hold `ARCHIVE_FORMAT` and `DERIVED_SCHEMA`.

  The lock is taken before `initialize` starts and released on a timer, so a deferred transaction fails before the timer fires.
- **Verify:** `cargo test -p terminus-core --all-features --lib store::open` passes. Temporarily change `initialize` back to `self.conn.unchecked_transaction()`, the deferred form it has today, and the same test fails with "database is locked". Then restore the fix, and never commit the reverted form. `cargo test -p terminus-core --all-features --test store_open` passes, including `concurrent_opens_of_an_older_store_all_bring_it_forward`. Clippy and fmt are clean.

### Task 2: Record the committed and failed counts of a single-file ingest (#14)

- **Files:** crates/terminus-core/src/ingest/mod.rs (`record_run` and the `RunRow` doc comment), crates/terminus/tests/status.rs
- **Action:** Make `record_run`'s `INSERT INTO runs` write `files_committed = 1` and `files_failed = 0` beside the `files_seen = 1` it already writes. It runs only for `RunRow::PerFile`, inside the transaction of a file that is committing, so 1 committed and 0 failed is exactly what that row describes. Do not add a `runs` row for a failed single-file ingest, and do not touch rows already written (D-09). Update the `RunRow` doc comment, which says `record_run` inserts a literal `1` for `files_seen`, so it names all three literals. In `tests/status.rs`, add a test that places one transcript with the bench's `place` and runs `terminus ingest <that path>`. Then `terminus status` must print `files        1 walked, 1 committed, 0 failed`, and the latest `runs` row must read `files_seen` 1, `files_committed` 1, `files_failed` 0.
- **Verify:** `cargo test -p terminus --all-features --test status` passes, including the new test. The new test fails against today's `record_run`. `cargo test -p terminus-core --all-features --test ingest --test pass --test backfill` passes. Clippy and fmt are clean.

### Task 3: Report the preserved count from reindex (#41)

- **Files:** crates/terminus/src/cmd/reindex.rs (`run`), crates/terminus/tests/cli.rs (`DATA_COMMANDS` and `reindex_json_reports_a_held_lock_as_a_document`), docs/json-shapes.md (the `reindex` section), docs/commands.md (the `terminus reindex` entry)
- **Action:** In `cmd::reindex::run`, add an integer `preserved` field to the success document, set to `rebuilt.preserved`, and to the lock-held failure document, set to 0. Both documents then carry `sessions`, `turns`, `skipped` and `preserved` (D-01). Without `--json`, print the preserved count on stderr only when `rebuilt.preserved` is above 0. Leave the existing `rebuilt N turn(s) across M session(s)` line exactly as it is, so a store with no evicted sessions prints what it prints today (D-10). `--json` mode still writes nothing to stderr, which `the_three_shipped_commands_take_json_without_changing_their_plain_output` asserts. `sessions` keeps meaning the sessions that were rebuilt. `reindex::reindex` already leaves preserved sessions out of that count, so the core library needs no change. In `tests/cli.rs`, add `preserved` to the `reindex` entry of `DATA_COMMANDS`. Make `reindex_json_reports_a_held_lock_as_a_document` assert that the refusal's `data` keys are exactly the four documented ones and that `preserved` is 0. Add a test that ingests two fixtures and evicts one through `testkit::evict`. On that store, `reindex --json` must give `preserved` 1 and `sessions` 1, and plain `reindex` must print a stderr line carrying the count 1. On a store with nothing evicted, plain `reindex`'s stderr must be exactly the one `rebuilt <turns> turn(s) across <sessions> session(s)` line. In `docs/json-shapes.md`, add `preserved` to the `reindex` example. Add a bullet saying what it counts: evicted sessions whose derived rows were carried across rather than rebuilt, which `sessions` does not include. The bullet also says it is 0 in a lock-held refusal, and that without `--json` the count goes to stderr when it is above 0. In `docs/commands.md`, add one sentence to the `terminus reindex` entry saying that it prints how many evicted sessions it carried across, on stderr when there are any. The doc prose is John's. Read `~/.claude/john-voice.md` in full before writing it, and match the bullets and sentences around it. Recalled prior art: phase 8 put the count on `Rebuilt::preserved` but no plan leased `cmd/reindex.rs`, so it never reached the output (`v0.1.0/phases/8/SUMMARY.md`, phase 8).
- **Verify:** `cargo test -p terminus --all-features --test cli` passes, including `every_data_command_emits_the_documented_shape`, the strengthened lock-held test and the new test. `grep -n '"preserved"' docs/json-shapes.md` finds the key in the `reindex` example, and the `terminus reindex` entry in `docs/commands.md` mentions the preserved count. Running plain `terminus reindex` on a store with no evicted session prints the same stderr bytes as before the change. Clippy and fmt are clean.

### Task 4: Fail an evicted session that still has block rows (#42)

- **Files:** crates/terminus-core/src/verify.rs (`walk` and the module doc comment), crates/terminus-core/tests/verify.rs
- **Action:** In `walk`, for a session `session_meta.is_evicted` marks, count its rows in `blocks`, the table `archive::remove` deletes from, with one query, and never read or decode them. When the count is above 0, push one `Failure` for that session with `block: None` and a `detail` saying how many block rows remain and that an evicted session should have none. Do not run `check` on an evicted session, so it gets one failure, never one per block. An evicted session with no rows stays silent. It is still counted in `checked`, and the divergence failure stays independent of this arm, as it is today. Use the existing `Failure` shape. `verify --json` and `docs/json-shapes.md` gain nothing (D-11). Correct the module doc comment's phase 8 paragraph, which says an evicted session is never a failure. Eviction promises the session has no block rows left, and that is now checked. Correct the inline comment on the evicted arm in `walk` the same way. Eviction is still a flag read off `session_meta.is_evicted` and never inferred from missing rows (phase 3 D-08, `v0.1.0/phases/3/CONTEXT.md`). In `tests/verify.rs`, add a test that marks one session evicted with an `UPDATE` of `session_meta.is_evicted` while leaving its block rows in place, which `testkit::evict` would clear. `verify::verify` must then report exactly one failure. That failure names that session, has `block` `None`, and has a `detail` carrying the session's actual row count. No other session may be named.
- **Verify:** `cargo test -p terminus-core --all-features --test verify --test retention` passes, including the new test and the unchanged `an_evicted_session_is_counted_and_never_named_while_a_corrupt_one_still_is`, `a_store_of_evicted_sessions_verifies_clean` and `an_evicted_session_can_still_report_a_divergence`. The new test fails against today's `walk`. `cargo test -p terminus --all-features --test cli --test retention` passes. `verify_names_the_corrupt_session_and_no_other` already proves that a non-empty report makes `terminus verify` exit 1. Clippy and fmt are clean.

## Notes

- AC3 asks for a test that races two opens of a fresh data directory. Task 1 makes that race deterministic by driving the losing open's `initialize` directly, after the winner has committed and while a writer holds the lock. A barrier race of two `Store::open` calls would reach that window only by chance, and the reverted code would then pass on most runs. During planning, the same sequence was run with Python's sqlite3 module (SQLite 3.54.0). The deferred form failed in under a millisecond with "database is locked". The IMMEDIATE form waited for the commit and succeeded.
- The CONTEXT assumption that switching a brand-new file to WAL does not race the way table creation does is untouched by this plan. If it is wrong, `status` can still fail in a fresh store's first moments.
- `docs/json-shapes.md` and `docs/commands.md` are docs John reviews himself. Open each in RustRover once Task 3 has written it.
