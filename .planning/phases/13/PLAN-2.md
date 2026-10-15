---
phase: 13
plan: 2
requirements: ["#40"]
files:
  - crates/terminus-core/src/store/schema.rs
  - crates/terminus-core/src/feedback/mod.rs
  - crates/terminus-core/src/feedback/replay.rs
  - crates/terminus-core/src/inject/prompt.rs
  - crates/terminus-core/src/feedback/stats.rs
  - crates/terminus/src/cmd/stats.rs
  - crates/terminus-core/tests/feedback.rs
  - crates/terminus/tests/replay.rs
  - crates/terminus/tests/cli.rs
  - docs/json-shapes.md
  - docs/commands.md
  - CHANGELOG.md
---

# Phase 13: CI and Data Integrity - Plan 2

## Goal

The phase goal: the test suite runs on GitHub for every push and pull request, and the archive and decision-log fixes (#15, #40, #38, the drain race, and #17's second case) land under it, so no known path writes wrong bytes into the archive or loses a decision's data.

This plan fixes #40. The `decisions` table has no column for a record's `compacted` flag or its dropped turns, so every drained decision loses both for good. After this plan the table keeps them. `terminus replay` scores a decision made after a compaction the way the live prompt did, and `terminus stats` counts those decisions apart from ordinary ones.

## Must be true when done

- A store made before this change gains `decisions.compacted` and `decisions.dropped` on its first writable open, in the same column order a fresh store has. Rows drained before that open read null in both.
- A decision drained after a compaction is stored with `compacted` = 1 and its dropped turn ids as a JSON array. An ordinary one is stored with `compacted` = 0 and an empty array.
- `terminus stats`, with and without `--json`, counts compacted decisions apart from ordinary ones, and every key it printed before is unchanged. Decisions drained before this build are in neither count.
- `terminus replay --compacted-ranked <n>` changes the result for a compacted decision, which replays through the compacted window and its stored dropped set. Ordinary decisions and decisions with no recorded flag replay as they do today.
- On a store whose `decisions` table lacks the two columns, both commands still answer and exit 0.

## Context

- Locked: D-11, D-12, D-13, D-14 and D-15. No `DERIVED_SCHEMA` change and no rebuild (D-11). Recovering `compacted` and `dropped` for decisions already drained is out of scope.
- Pattern to follow: the nullable added columns `capture_mode` and `is_typed` in `store/schema.rs`, each with a documented meaning for null, declared last in its table and appended last in `BRING_FORWARD_COLUMNS`.
- Order: runs after plan 1. It shares `store/schema.rs` with plan 4, `feedback/mod.rs` and `tests/feedback.rs` with plan 3, and `CHANGELOG.md` with plans 3 and 4, so it runs before both.
- Prose in `docs/` and `CHANGELOG.md` follows `~/.claude/john-voice.md`, read in full before writing.

## Tasks

### Task 1: Store `compacted` and `dropped` with every drained decision

- **Files:** crates/terminus-core/src/store/schema.rs, crates/terminus-core/src/feedback/mod.rs, crates/terminus-core/tests/feedback.rs
- **Action:** In `CREATE_SQL`, add `compacted INTEGER` (nullable, no default) and `dropped TEXT` at the end of the `decisions` table, after `thresholds`. At the end of `BRING_FORWARD_COLUMNS`, add a new `decisions` entry listing the two columns in that order with the same type clauses (D-11, D-12). Comment both in the table the way `capture_mode` and `is_typed` are commented:
  - `compacted` is 1 when the prompt ran under the compacted pool (INJ-05) and 0 when it did not.
  - Null means the decision was drained before the column existed, so its value is unknown. `NOT NULL DEFAULT 0` was rejected because it would record every earlier decision as ordinary for good.
  - `dropped` holds the dropped turn ids as one JSON document, like the other list-shaped columns.
  - Both are declared last and appended last, by `BRING_FORWARD_COLUMNS`'s own rule.

  A writable open adds the columns through `bring_forward`. No `DERIVED_SCHEMA` change is involved. In `feedback/mod.rs`, `insert` grows from 11 to 13 columns. `compacted` comes from `Decision::compacted` as 1 or 0, and `dropped` comes from `Decision::dropped` through the existing `document` helper. Update `insert`'s doc comment. Nothing else in the drain changes in this task.
- **Verify:** `cargo test -p terminus-core --features testkit --test feedback` passes, including new tests that show the following:
  - Two decision files are written through `Decision::save`, one with `compacted` true and `dropped` [41, 42] and one with `compacted` false. After a pass, their rows read `compacted` 1 with `dropped` `[41,42]`, and `compacted` 0 with `dropped` `[]`.
  - On a store whose `decisions` table had both columns dropped and held one row, a pass adds the columns back, that row reads null in both, and `PRAGMA table_info(decisions)` lists the same columns in the same order as on a fresh store.

  `cargo test -p terminus-core --features testkit --test schema --test store_open` passes. Their aged-store tests iterate `BRING_FORWARD_COLUMNS`, so they now drop and restore both columns too. `git diff` leaves `DERIVED_SCHEMA` in `store/open.rs` untouched.

### Task 2: Replay a compacted decision through the compacted pool

- **Files:** crates/terminus-core/src/feedback/replay.rs, crates/terminus-core/src/inject/prompt.rs, crates/terminus-core/tests/feedback.rs, crates/terminus/tests/replay.rs, CHANGELOG.md
- **Action:** Replay reads `compacted` and `dropped` with each eligible decision (D-13). A decision with `compacted` = 1 searches with `Thresholds::compacted_ranked` as its limit instead of `ranked`. After `prompt::eligible`, its hits are filtered to its stored dropped set, then `prompt::capped` applies. That is the live arm's order, and `within`'s doc in prompt.rs says why the filter comes after the threshold. The filter is prompt.rs's existing `within`. Make it reachable from replay rather than writing a second copy, the same way replay already uses `prompt::eligible` and `prompt::capped`. `would_inject`'s doc gives the reason: a second implementation would make the diff a measurement of the replay engine. A decision with `compacted` 0 or null replays through the ordinary window exactly as today.

  The store may lack the two columns (D-15). Replay opens read-only and never brings a store forward. So the query that reads eligible decisions checks `Store::missing_columns`, and when either column is missing it does not name them and treats every decision as unknown.

  Rewrite the "What it cannot reconstruct" paragraph of the module doc to say what replay does now, and why. Compacted decisions replay through the compacted pool. Decisions drained before this build carry null and replay through the ordinary window. This is the replay that v0.1.0 phase 6, plan 3, task 3 asked for and could not deliver, because the table lacked the columns (`v0.1.0/phases/6/SUMMARY.md`). Add an `[Unreleased]` entry to CHANGELOG.md. It says `terminus replay` now scores decisions made after a compaction through the compacted window and their dropped set, so `--compacted-ranked` affects them, and that decisions logged before this release replay as ordinary ones. Read `~/.claude/john-voice.md` in full before writing the entry.
- **Verify:** `cargo test -p terminus-core --features testkit --test feedback` passes, including a new test with three otherwise equal decisions whose `compacted` values are 1, 0 and null:
  - The compacted decision's dropped set holds a turn the default compacted window reaches, so its replayed labels differ between the default `compacted_ranked` and `compacted_ranked` 0.
  - The compacted decision's would-inject set never includes a turn outside its dropped set.
  - The 0 and null decisions replay to the same labels under both values.

  `cargo test -p terminus --test replay` passes, including two new tests:
  - On a store holding a labelled compacted decision, `terminus replay --json` and `terminus replay --json --compacted-ranked 0` differ in `labels` or `changed`.
  - On a store whose `decisions` table had both columns dropped, `terminus replay --json` exits 0 with `ok` true.

### Task 3: Count compacted decisions apart from ordinary ones in `terminus stats`

- **Files:** crates/terminus-core/src/feedback/stats.rs, crates/terminus/src/cmd/stats.rs, crates/terminus/tests/cli.rs, crates/terminus/tests/replay.rs, crates/terminus-core/tests/feedback.rs, docs/json-shapes.md, docs/commands.md, CHANGELOG.md
- **Action:** `Stats` gains two counts: decisions recorded with `compacted` = 1, and decisions recorded with `compacted` = 0. A decision whose `compacted` is null was drained before this build and is in neither count. `decisions` still counts every row, so the gap between `decisions` and the two new counts is exactly the decisions whose kind is unknown (D-12). `terminus stats --json` gains one key for each count. No shipped key is renamed, moved or removed, and the two new keys are permanent once shipped (D-14, phase 12 D-01). Name them in the same snake_case as the existing keys. The human output gains one line for each, aligned like the existing lines. When the store lacks the columns, stats checks `Store::missing_columns` instead of naming them, and both counts are 0 (D-15). Stats opens read-only and never brings a store forward.

  In the same commit (D-14):
  - DATA_COMMANDS's `stats` entry in tests/cli.rs lists both keys.
  - The `### stats` example in docs/json-shapes.md carries both keys, and a bullet says what each counts and that decisions logged before this release are in neither.
  - The `terminus stats` paragraph in docs/commands.md mentions the split.

  Add an `[Unreleased]` entry to CHANGELOG.md for the new counts. Read `~/.claude/john-voice.md` in full before writing any of this prose.
- **Verify:**
  - `cargo test -p terminus --test cli` passes with the two keys in DATA_COMMANDS.
  - `cargo test -p terminus-core --features testkit --test feedback` passes. The report test (`the_report_adds_up_the_known_mix`, or a new sibling) asserts both counts over a mix of rows whose `compacted` is 1, 0 and null, with the null rows in neither count.
  - `cargo test -p terminus --test replay` passes, including a new test: on a store whose `decisions` table had both columns dropped, `terminus stats --json` exits 0 with `ok` true, both new counts 0, and `decisions` equal to the row count.
  - Every key `terminus stats --json` emitted before this change is still present under the same name, and `terminus stats` without `--json` prints the two new lines.

## Notes

- The two new `stats --json` key names are the executor's choice within the constraints in Task 3. D-14 makes them permanent once shipped, so the review should look at the names themselves, not only at the counts.
- Planning choice: stats gets two counts, not three. A third key for the unknown decisions would be more than D-14 states, and the gap from `decisions` already gives that number, which the json-shapes bullet explains.
