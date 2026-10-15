---
phase: 13
plan: 4
requirements: ["#15", "#17"]
files:
  - crates/terminus-core/src/ingest/mod.rs
  - crates/terminus-core/src/store/schema.rs
  - crates/terminus-core/src/store/open.rs
  - crates/terminus-core/src/derive.rs
  - crates/terminus-core/src/reindex.rs
  - crates/terminus-core/src/inject/compaction.rs
  - crates/terminus-core/src/observe/mechanical.rs
  - crates/terminus-core/src/retention/apply.rs
  - crates/terminus-core/tests/compaction.rs
  - crates/terminus-core/tests/derive.rs
  - crates/terminus-core/tests/inject_compaction.rs
  - crates/terminus-core/tests/reindex.rs
  - crates/terminus-core/tests/retention.rs
  - crates/terminus-core/tests/snapshot.rs
  - crates/terminus/tests/crash.rs
  - crates/terminus/tests/suppression.rs
  - CHANGELOG.md
---

# Phase 13: CI and Data Integrity - Plan 4

## Goal

The phase goal: the test suite runs on GitHub for every push and pull request, and the archive and decision-log fixes (#15, #40, #38, the drain race, and #17's second case) land under it, so no known path writes wrong bytes into the archive or loses a decision's data.

This plan fixes the two archive issues.

- **#15.** A diverged session's prefix check and its tail read each open the file. A file replaced between those two opens can splice one file's tail onto another file's prefix. That is the one known way to write wrong bytes into the archive.
- **#17's second case.** A real compaction boundary with no `uuid` or no `timestamp` gets no boundary row. The fix settles where such a boundary sits in the session: at its own byte offset.

## Must be true when done

- A diverged (flagged) session's ingest pass opens its transcript exactly once. One handle does the length check, the prefix comparison and the tail read. An unflagged session's pass is unchanged.
- Boundary rows live in a new derived table keyed by `(session_key, stream_offset)`, which replaces `compaction_boundaries`. The dropped set is the turns before the most recent boundary in stream order, which gives today's answer for every boundary that is a turn.
- A `system`/`compact_boundary` record with no `uuid` or no `timestamp` gets a boundary row at its byte offset, through ingest and through both reindex paths, without becoming a turn. The turns after it keep their ids, and the next prompt after that compaction treats the turns before it as dropped.
- A store at `DERIVED_SCHEMA` 4 is rebuilt to 5 on its first writable open, with or without evicted sessions. The old `compaction_boundaries` table is gone afterwards, and every boundary row an evicted session had is carried into the new table.
- A record that is not `system` but carries subtype `compact_boundary` still gets no row, as phase 12 made it.

## Context

- Locked: D-08, D-09, D-10, D-19, D-20, D-21 and D-22. Out of scope: protecting #15's read against an in-place rewrite of the same file (D-09).
- `derive.rs` is the one seam both ingest and reindex call to write derived rows, and it stays that way (its module doc).
- Foreign keys are enforced, because the bundled SQLite is built with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`. Every drop and delete goes children first, as `reindex.rs` documents.
- Order: runs last. It shares `store/schema.rs` with plan 2 and `CHANGELOG.md` with plans 2 and 3.
- `CHANGELOG.md` prose follows `~/.claude/john-voice.md`, read in full before writing.

## Tasks

### Task 1: Read a diverged session's prefix and tail through one file handle

- **Files:** crates/terminus-core/src/ingest/mod.rs, crates/terminus-core/tests/compaction.rs
- **Action:** D-08, D-09 and D-10. In `prepare`, a flagged session (`existing.diverged`) opens its transcript once, through `discover::open_transcript`. That one handle does three things in order:
  1. It checks the length. A file shorter than the watermark is still `Error::TranscriptDiverged`.
  2. It reads `[0, watermark)` for the comparison `prefix_matches` makes today.
  3. It reads on from the watermark to EOF for the tail.

  `prefix_matches`'s other answers stay exactly as they are, and each is settled without reading prefix bytes:
  - a session with nothing archived matches;
  - a capture mode other than `full` does not match;
  - an archive shorter than the watermark does not match.

  A mismatch is still `Error::TranscriptRewritten`, with the flag left set. The order of the errors is unchanged: a shorter file is `TranscriptDiverged` before any question about its prefix is asked. An unflagged session keeps today's path, which seeks to the watermark and reads only the tail (`read_tail`), never the prefix.

  The single handle protects against the file being replaced between reads, whether by rename or by unlink and recreate. It does not protect against an in-place rewrite of the same inode during the read, and no identity or length re-check is added (D-09). Say both in the doc comments of the functions this changes, and remove the comment text that describes two opens.
- **Verify:** `cargo test -p terminus-core --features testkit --test compaction` passes, including new tests:
  - A session is flagged, its transcript is restored or grown, and a pass runs. `discover::opened::under` shows exactly one new entry for that transcript's path. Count the entries before and after the pass rather than calling the global `reset`, because the log is shared by the whole test binary and its tests run in parallel.
  - The same holds for a flagged session whose transcript was rewritten under its watermark: still flagged, and still one open.

  Run the new tests before the change and they see two opens. `cargo test -p terminus-core --features testkit --test capture` passes, so `a_truncated_lean_session_is_flagged_skipped_and_never_cleared` and `a_restored_full_session_still_clears_its_flag` still hold, and every existing divergence test in compaction.rs passes.

### Task 2: Key boundary rows by stream position, in a new derived table

- **Files:** crates/terminus-core/src/store/schema.rs, crates/terminus-core/src/store/open.rs, crates/terminus-core/src/derive.rs, crates/terminus-core/src/reindex.rs, crates/terminus-core/src/inject/compaction.rs, crates/terminus-core/src/observe/mechanical.rs, crates/terminus-core/src/retention/apply.rs, crates/terminus-core/tests/compaction.rs, crates/terminus-core/tests/derive.rs, crates/terminus-core/tests/inject_compaction.rs, crates/terminus-core/tests/reindex.rs, crates/terminus-core/tests/retention.rs, crates/terminus-core/tests/snapshot.rs, crates/terminus/tests/crash.rs, crates/terminus/tests/suppression.rs
- **Action:** D-20, D-21 and D-22. Most of this task's file count is the old table's name being retargeted in tests. The concern is one change: boundary rows get a new key.

  **The table.** Replace `compaction_boundaries` with a new derived table under a new name. It is keyed by `(session_key, stream_offset)` and keeps the metadata bytes verbatim, as today. It has to be a new table rather than the old one re-keyed, for two reasons. The old `turn_id INTEGER PRIMARY KEY` would give a null `turn_id` the next turn's id. And `CREATE TABLE IF NOT EXISTS` would never reach a re-keyed shape on the reindex path that keeps evicted sessions. Make these changes:
  - In `CREATE_SQL`, the new table replaces the `compaction_boundaries` block. Rewrite its comment: rows are keyed by stream position so that a boundary with no turn behind it has a place (D-19, D-20).
  - In `TABLES` and `DERIVED_TABLES`, the new table replaces `compaction_boundaries`, placed so that reindex's drop-in-reverse order still satisfies every enforced foreign key.
  - Its `stream_offset` is the same uncompressed-stream coordinate `turns.stream_offset` uses (see the `turns` comment in schema.rs), so the two compare correctly under every capture mode.

  **The writer.** `derive::derive_turn` writes the row for a turn that is a boundary at `(session_key, row.stream_offset)`, delete-then-insert at that key as today. Update its module doc. In this task only boundaries that are turns get rows; Task 3 adds the rest. So every dropped set stays what it is today.

  **Readers and deleters** move to the new key:
  - `inject::compaction::boundary` picks the session's most recent boundary by `stream_offset`. `turns_before` returns the session's turns whose `stream_offset` is lower than the boundary's (D-21). For a boundary that is a turn, this is the same set the `turn_seq` rule gives, and the boundary turn itself is still excluded. Update the module and function docs that name `compaction_boundaries` or `turn_seq`.
  - `observe/mechanical.rs` counts a session's boundary rows by `session_key`.
  - `reindex::clear_derived` and `retention::apply::delete_one` delete a session's boundary rows by `session_key`, never through its turn ids, because Task 3's rows have no turn. Order the deletes so the foreign keys allow them, and update both doc comments.

  **The upgrade (D-22).** `DERIVED_SCHEMA` goes from 4 to 5. Add a doc paragraph in the style of the ones for 2, 3 and 4: the boundary rows moved to the new table, and the first ingest after the upgrade rebuilds every session's derived tables. `reindex::reindex` retires the old table on both paths, before `turns` is dropped or cleared. That order matters because the old table's foreign key to `turns(id)` is enforced and would fail the drop or the delete.
  - On the path that keeps evicted sessions, first copy each evicted session's old boundary rows into the new table, at the `(session_key, stream_offset)` of the boundary's turn. An emptied blob cannot rebuild them, and the module doc's rule is that a preserved session's derived rows are the only copy left (phase 8 D-03).
  - Then drop `compaction_boundaries` if it exists. On a store that never had the old table, both steps do nothing.

  Update reindex.rs's module doc and its drop-order comment. `CREATE_SQL`'s `IF NOT EXISTS` creates the new table on both paths (D-20). Do not touch `parse`: record classification, `turn_seq` and `turns.id` stay as they are (D-19).

  **Tests.** Retarget every test that reads or writes `compaction_boundaries` to the new table and key, keeping what each assertion means:
  - crash.rs's orphan check becomes boundary rows whose `session_key` has no `sessions` row;
  - suppression.rs's test deletes and reinserts the row under the new key;
  - derive.rs, compaction.rs, inject_compaction.rs, reindex.rs and retention.rs are retargeted the same way;
  - snapshot.rs's header comment names the new table.

  Add two tests to reindex.rs. Each starts from a store put into the version-4 shape: the old `compaction_boundaries` table, with its old DDL, holding rows for a session that has a boundary; the new table absent; and `derived_schema` set to 4.
  - `reindex::open_up_to_date` brings that store to 5. The old table is gone, and the new table holds the same boundary rows at the same offsets with the same metadata.
  - The same, with that session evicted first. Its boundary row is carried across and the session still counts in `Rebuilt::preserved`.
- **Verify:**
  - `cargo test --workspace --all-features` passes.
  - `cargo test -p terminus-core --features testkit --test reindex` passes with the two new upgrade tests.
  - `cargo test -p terminus-core --features testkit --test inject_compaction` passes with its existing dropped-set assertions unchanged.
  - `grep -rn compaction_boundaries crates/terminus-core/src crates/terminus/src` matches only reindex.rs's retirement step and the history paragraphs in open.rs's `DERIVED_SCHEMA` doc.
  - `grep -n "DERIVED_SCHEMA: i64 = 5" crates/terminus-core/src/store/open.rs` matches.

### Task 3: Give a boundary with no `uuid` or no `timestamp` a row at its own offset

- **Files:** crates/terminus-core/src/derive.rs, crates/terminus-core/src/ingest/mod.rs, crates/terminus-core/src/reindex.rs, crates/terminus-core/tests/compaction.rs, crates/terminus-core/tests/derive.rs, crates/terminus/tests/suppression.rs, CHANGELOG.md
- **Action:** D-19 and D-20. A `system`/`compact_boundary` record that lacks `uuid` or `timestamp` is not a turn and stays that way. `Record::parse`'s classification, `turn_seq` and `turns.id` do not change.

  Ingest and reindex each gain a path that writes a boundary row for such a record at `(session_key, record offset)`, with its `compact_metadata` bytes. Both call one function in derive.rs, which is the seam rule in derive.rs's module doc. In ingest the write happens inside the same transaction as the turn rows (STOR-02, in ingest's `apply`), and the record offset is the second scan's offset in stored-stream coordinates, the same coordinate `derive_turn` gets. In reindex it is the record's offset in the blob's scan.

  Rules for the new path:
  - **Which records count.** The existing type-and-subtype rule, `Record::is_compact_boundary`. A `user`, `assistant` or `attachment` record with subtype `compact_boundary` still gets no row (`phases/12/UAT.md`, phase 12).
  - **One row each.** Every boundary record yields exactly one row. Either a boundary that is a turn keeps getting its row from `derive_turn`, or every boundary goes through the new function. Never both.
  - **The dropped set.** It needs no change. Task 2 already orders boundaries by stream position, so the turns before a boundary with no turn are the ones with a lower `stream_offset`.
  - **Stuck flags.** A session whose `compaction_owed` flag stuck because its boundary had no row clears on its next prompt, once the rebuild gives the boundary its row (D-22). The flag's semantics are in `v0.1.0/phases/5/CONTEXT.md` D-08.

  Add an `[Unreleased]` entry to CHANGELOG.md. It says a compaction boundary with no `uuid` or `timestamp` now gets a row, so the prompt after it can offer back what fell out of context (#17), and that the first ingest after upgrading rebuilds the derived tables once. Read `~/.claude/john-voice.md` in full before writing the entry.
- **Verify:** `cargo test -p terminus-core --features testkit --test compaction` passes, with new tests built on two boundary lines made from `testkit::boundary_line()`: one with its `uuid` removed and one with its `timestamp` removed. Each line is placed after ordinary turns and followed by more turns, and in one case it is the transcript's last line, so its pass archives no turn at all. For each line:
  - the new table holds one row at the line's byte offset, with metadata equal, as parsed JSON, to the line's `compactMetadata`;
  - no turn row exists for the line;
  - the turns after it carry the same ids as in the same transcript with that line removed;
  - the same row is there after `reindex::reindex` on a store with no evicted session, and after one on a store where a different session is evicted, so the rebuild takes the preserving path.

  A `user` record with that subtype and no `uuid` gets no row. `cargo test -p terminus --test suppression` passes with a new test shaped like `after_a_compaction_the_next_prompt_draws_from_what_fell_out` whose boundary record has no `uuid`. After a `compact` SessionStart, the next prompt offers back the turn that fell out and not the preserved one, and `compaction_owed` is false afterwards. `cargo test --workspace --all-features` passes.

## Notes

- Planning choice: Task 2 carries each evicted session's old boundary rows into the new table during the version 4 to 5 rebuild. CONTEXT says boundary rows "move" to the new table (D-20) but does not spell out the evicted case. Dropping those rows would break the rule shipped in phase 8 D-03, that an evicted session's derived rows are the only copy. Raised in the return so the owner can confirm.
- The new table's name is the executor's choice.
