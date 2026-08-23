---
phase: 1
plan: 1
requirements:
  - INJ-07
files:
  - crates/verbatim-core/src/store/schema.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/src/parse/record.rs
  - crates/verbatim-core/src/derive.rs
  - crates/verbatim-core/tests/schema.rs
  - crates/verbatim-core/tests/parse.rs
  - crates/verbatim-core/tests/derive.rs
  - crates/verbatim-core/tests/reindex.rs
---

# Phase 1: Prompt-True Resume Brief - Plan 1 of 3 (the stored discriminator)

**ORDER: PLAN-1, then PLAN-2, then PLAN-3.** The three plans share no file, but
they are sequential and not parallel: PLAN-2's query reads the column this plan
declares and the values this plan writes, and PLAN-3 proves properties of the
brief PLAN-2 changes. Running them out of order gives a query against a column
that is not there.

## Goal

Every `user` turn in the store says, from a stored column, whether the person
typed it or the harness wrote it - written at ingest, reproduced by a rebuild
from blobs alone, and present on a store that an earlier build wrote.

## Must be true when done

- On a freshly ingested store,
  `SELECT count(*) FROM turns WHERE record_type = 'user' AND is_typed IS NULL`
  returns 0, and
  `SELECT count(*) FROM turns WHERE record_type <> 'user' AND is_typed IS NOT NULL`
  returns 0. Two states for `user` rows and no third.
- A `user` row whose record carries a `tool_result` block, an `isMeta: true`
  flag, or a harness envelope reads 0; every other `user` row reads 1.
- `verbatim reindex` over the same store reproduces every value from the blobs
  alone, and a session archived under `lean` or `minimal` capture classifies
  the same as the same records archived under `full`.
- A store written by the previous build - no `is_typed` column, an older
  `derived_schema` - opens, acquires the column, is populated by the rebuild
  without any transcript being read, then completes a further ingest even when
  it holds an evicted session, and `verify` reports no failure over it.
- The resume brief's output is unchanged: nothing reads the column yet.

## Context

- D-01/D-05/D-06/D-07 bind the rule: it reads the record's own
  `message.content` blocks and never the top-level `toolUseResult`, so there is
  no cross-record join and elision cannot change the answer.
- D-02 binds the meaning: "typed" is "authored by the person", so `isMeta: true`
  records and harness envelopes join tool results on the not-typed side. Still
  exactly two states.
- D-03/D-04/D-08 bind the storage: a nullable column on the derived `turns`
  table, a `DERIVED_SCHEMA` bump to make `reindex` the blob-only backfill, and a
  `BRING_FORWARD_COLUMNS` entry so the preserving rebuild path cannot miss it.
- D-16 bounds the reach: `derive::derive_turn` and `parse::Record::parse` are
  the only production sites, and all three ingest entry points converge on the
  per-turn loop in `ingest/mod.rs`.
- Out of scope here: `inject/brief.rs`, `recall/*`, `verify.rs` and
  `inject/state.rs`, none of which change (D-09, D-10, D-12).

## Tasks

### Task 1: Declare `turns.is_typed` and force the rebuild that fills it

- **Files:** crates/verbatim-core/src/store/schema.rs,
  crates/verbatim-core/src/store/open.rs,
  crates/verbatim-core/tests/schema.rs
- **Action:** Add one column, `is_typed INTEGER`, to the `turns` table in
  `CREATE_SQL`, placed after `byte_len` and before the
  `UNIQUE (session_key, turn_seq)` table constraint - last among the columns,
  which is the ordering rule `BRING_FORWARD_COLUMNS`' own doc states, since
  `ALTER TABLE ADD COLUMN` appends and a column declared mid-table would give a
  fresh store and an upgraded store different layouts. Carry the house-style
  comment on it: null means "nothing derived this", which is what a preserved
  evicted row and a non-`user` row both are. Nullable, never
  `NOT NULL DEFAULT`: reindex skips a preserved evicted session's rows, so they
  keep null whatever the declaration says, and a default would silently read
  every one of them as one class (D-04, and the `capture_mode` precedent in the
  same file). Append a `turns` entry to `BRING_FORWARD_COLUMNS` naming
  `is_typed` - without it the preserving rebuild path never creates the column,
  because it skips the DROP loop and runs only `CREATE TABLE IF NOT EXISTS`,
  and `derive_turn`'s explicit INSERT column list then fails with
  `no such column` on every future pass, permanently, since
  `reindex::open_up_to_date` runs at the top of each one (D-08). Bump
  `DERIVED_SCHEMA` in `store/open.rs` from 3 to 4, with a doc paragraph in the
  same shape as the existing "2 as of phase 2" and "3 as of phase 3" ones,
  saying that the bump is what makes `reindex::reindex` the blob-only backfill
  and that `ingest/backfill.rs` is a different mechanism with a colliding name
  (D-03). Extend
  `a_brought_forward_session_meta_has_the_same_column_order_as_a_fresh_one` in
  `tests/schema.rs` to compare `turns` as well as `session_meta` and `runs`;
  `aged_store` in that file already drops every `BRING_FORWARD_COLUMNS` entry,
  so it will now age `turns` too and needs no edit for that.
- **Verify:** `cargo test -p verbatim-core --features testkit --test schema` and
  `cargo test -p verbatim-core --features testkit --test store_open` both pass;
  on a fresh store `PRAGMA table_info(turns)` lists `is_typed` last, and the
  layout of `turns` in a store whose `is_typed` was dropped and then reopened is
  identical to the layout in a fresh one.

### Task 2: Classify a `user` record as typed or not, in the parser

- **Files:** crates/verbatim-core/src/parse/record.rs,
  crates/verbatim-core/tests/parse.rs
- **Action:** `Record::parse` already hands the parsed value to `tool_name`,
  whose doc names `message.content` as the extension point; the classification
  goes beside it and is carried on `Turn` as a nullable boolean. It is set only
  when the record's type is `user` and is absent for every other turn type, so
  the assistant, attachment and system rows stay null (D-04). Not typed when any
  block of `message.content` has type `tool_result` - measured over 400 sampled
  transcripts, `toolUseResult` present implies the block in 12,234 of 12,234
  cases and the block appears without the key in 282 more, so the block rule is
  a strict superset and the key rule adds nothing (D-01); a record carrying both
  a `text` block and a `tool_result` block is not typed, so the tool-result test
  wins (D-05). Not typed when the record's top-level `isMeta` is true. Not typed
  when the record's text, after leading whitespace, opens with one of a harness
  envelope tag list held in a single `const` in this module, so a new upstream
  shape is one line. Nothing may read the top-level `toolUseResult`, and nothing
  may call `Value::as_object` on it: over a 300-file sample it is a dict on
  6,265 records, a string on 522 and a list on 35, so an `as_object` test would
  misclassify 8.2% of them (D-06). Reading `message.content` and nothing else is
  also what makes the rule survive `[capture]` elision, since `ELIDED_FIELDS` in
  `capture.rs` is `toolUseResult` and `attachment` only and message content is
  never touched (D-07). `promptSource` must not be used: it is present on 57 of
  400 records and on no tool-result record at all (D-02). The envelope tag list,
  measured on this machine on 2026-08-23 over 400 top-level transcripts and
  12,379 `user` turn records, is `command-message` (143 records lead with it),
  `command-name` (119), `local-command-caveat` (119, every one of them also
  `isMeta`), `task-notification` (64), `local-command-stdout` (41) and
  `bash-stdout` (13). Match the LEADING tag against that list and never "the
  text contains a tag": roughly 200 person-typed prompts in the same sample
  carry `<objective>`, `<execution_context>` and `<process>` tags inside them
  and every one of them must stay typed. `bash-input` (13 records) is
  deliberately left on the typed side - it is the command the person typed after
  `!`, and the harness only wrapped it.
- **Verify:** `cargo test -p verbatim-core --features testkit --test parse`
  passes with new cases covering: a plain text prompt (typed); a `tool_result`
  block (not typed); a record carrying both a `text` and a `tool_result` block
  (not typed); `isMeta: true` (not typed); one record per envelope tag in the
  list (not typed); a prompt whose text merely contains `<objective>` (typed);
  a record whose `toolUseResult` is a bare string rather than an object (still
  classified off the block, not the key); an `assistant` record (no value at
  all); and the same tool-result line put through `capture::elide` under
  `CaptureMode::Minimal` classifying identically before and after.

### Task 3: Write the discriminator through the one seam

- **Files:** crates/verbatim-core/src/derive.rs,
  crates/verbatim-core/tests/derive.rs
- **Action:** `derive_turn`'s `INSERT INTO turns` names its columns explicitly
  and its `ON CONFLICT(id) DO UPDATE SET` names them again; add `is_typed` to
  both halves, or a re-derive of a turn whose classification changed would keep
  the old value while every other column moved. The value comes off the `Turn`
  the parser produced and is never re-read out of `row.record` here - the rule
  the `subtype` field's doc in this file already states, because the parser is
  the one place a line's fields are extracted and a second parse site inside the
  seam is a second place for ingest and rebuild to disagree. Nothing else in the
  function changes: the FTS projection, the entity extraction and the boundary
  row are untouched. No caller changes either - `derive_turn` has exactly two
  non-test callers, the per-turn loop in `ingest/mod.rs` and the one in
  `reindex.rs`, and `ingest/backfill.rs` reaches the store through
  `crate::ingest::apply`, so all three ingest entry points already converge here
  (D-16). `crates/verbatim-core/tests/derive.rs` builds a `parse::Turn` literal
  and will not compile until it carries the new field; update that construction
  rather than working around it.
- **Verify:** `cargo test -p verbatim-core --features testkit --test derive`
  passes; over a store with every transcript fixture ingested,
  `SELECT count(*) FROM turns WHERE record_type = 'user' AND is_typed IS NULL`
  returns 0 and
  `SELECT count(*) FROM turns WHERE record_type <> 'user' AND is_typed IS NOT NULL`
  returns 0; the three `user` rows of `session-errors-a.jsonl` all read 0 and
  the single `user` row of `session-recall.jsonl` reads 1.

### Task 4: Bring a previous build's store forward and backfill it from blobs

- **Files:** crates/verbatim-core/tests/reindex.rs
- **Action:** One test for AC5, built on the bench this file already has. Evict
  a session the way the existing `evict` helper does, then age the store to what
  the previous build wrote: `ALTER TABLE turns DROP COLUMN is_typed` and
  `derived_schema` set to `DERIVED_SCHEMA - 1`. Remove or rename the work
  directory holding the transcripts before reopening, which is how
  `the_rebuild_reads_nothing_but_the_blobs` in this file already establishes
  that a rebuild reads no transcript, then call `reindex::open_up_to_date`.
  Assert: the column is back; every `user` row of every non-evicted session
  carries a value; the evicted session's derived rows are untouched and its
  `is_typed` values stay null (D-04); a further `ingest::run` over a fixture
  commits rather than failing with `no such column`, which is the permanent
  failure D-08 names; and `verify::verify` reports no failure afterwards, which
  is a regression guard rather than new work, since that command's query reads
  `sessions` and `session_meta` and never touches `turns` (D-09).
- **Verify:** `cargo test -p verbatim-core --features testkit --test reindex`
  passes including the new test, and deleting the `turns` entry from
  `BRING_FORWARD_COLUMNS` that task 1 added makes it fail with
  `no such column: is_typed`.

## Notes

- Column name chosen here, since CONTEXT leaves it open: `turns.is_typed`, an
  INTEGER reading 1 for a turn the person typed, 0 for one the harness wrote,
  null for a row nothing derived. The name says "authored by the person" rather
  than "tool result", which is what D-02 requires it to mean.
- Prior art carried forward: v0.1.0 phase 3's D-02
  (`.planning/_archive-v0.1.0/3/CONTEXT.md`, phase 3) established that entities
  are derived from the same record's own bytes with no cross-record join, and
  named `parse/record.rs`'s `tool_name` walk as the extension point. Task 2 is
  that extension point being used, and task 3 keeps the one-record-in /
  one-row-out shape of `derive_turn` that the same decision protects.
- The envelope tag list extends CONTEXT D-02's four named shapes with
  `command-message` and `bash-stdout`, both measured above: `command-message`
  leads the very same slash-command record D-02 names by its other tag
  (`command-name`), and `bash-stdout` is harness-written command output. That is
  membership in the one list D-02 already establishes, not a second check beside
  it. `bash-input` was measured and deliberately left out.
