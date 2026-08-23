---
phase: 6
plan: 1
requirements: [FEED-01]
files:
  - crates/verbatim-core/src/recall/search.rs
  - crates/verbatim-core/src/store/schema.rs
  - crates/verbatim-core/src/inject/decision.rs
  - crates/verbatim-core/src/inject/mod.rs
  - crates/verbatim-core/src/inject/prompt.rs
  - crates/verbatim-core/src/feedback/mod.rs
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/tests/feedback.rs
  - crates/verbatim-core/tests/store_open.rs
  - crates/verbatim/src/cmd/search.rs
  - crates/verbatim/tests/decisions.rs
  - docs/json-shapes.md
---

# Phase 6: Feedback Loop - Plan 1 (decision capture)

## Goal

Every UserPromptSubmit leaves a durable decision record - non-fires included -
carrying entities, candidates with what they matched on, suppressions with
reasons, thresholds and char counts, drained into a `decisions` table that
survives reindex and reaches upgraded stores without one.

## Must be true when done

- A prompt that injects, a prompt that fires nothing on threshold, and a prompt
  whose candidates are empty before the store opens each yield one row in a
  `decisions` table after the next ingest pass.
- Each decision row carries the candidate spellings extracted, the scored
  candidates with their matched `(kind, value_norm)` pairs, the injected turn
  ids with per-turn and total char counts, the suppressed turn ids with
  reasons, and the threshold values in force.
- The phase-4 hook p99 budget test still passes: the per-prompt write is a file
  write on the abandoned thread, never SQLite.
- `verbatim reindex` on a store holding decisions returns the same `decisions`
  rows afterwards, and a store initialized by the phase-5 binary gains
  `decisions` and `labels` on the next `Store::open` with no reindex.
- `verbatim search --json` hits report the matched `(kind, value_norm)` pairs,
  and the documented shape says so.

## Context

- D-01: the record is a file under the data directory written on the hook's
  abandoned thread (the `inject/state.rs` tmp+rename pattern); ingest drains it
  into SQLite. The hook never writes the store.
- D-02: the new tables reach existing stores through `bring_forward`'s
  missing-table arm on `Store::open` - no `DERIVED_SCHEMA` bump.
- D-03: `decisions` stays out of `schema::DERIVED_TABLES`. Planner's call on
  `labels`: it also stays out (see Notes).
- D-04: a decision anchors by `session_id` plus wall-clock timestamp, never
  `turn_seq`. D-10: each decision carries a monotone watermark (max
  `sessions.session_no` at decision time). D-11: every prompt writes a record.
  D-12: char counts via the existing `chars`/`clip` proxy, no tokenizer.
- Out of scope: any auto-tuner, labeling (PLAN-2), replay and stats (PLAN-3),
  config-file thresholds (D-08 keeps them compile-time).
- Execution order: this plan runs before PLAN-2 and PLAN-3 (they share
  `pass.rs`, `prompt.rs`, `search.rs`, `feedback/mod.rs`, `docs/json-shapes.md`).

## Tasks

### Task 1: Expose matched (kind, value_norm) pairs on `Hit`

- **Files:** crates/verbatim-core/src/recall/search.rs (symbols `Matched`,
  `Hit`, `weight_by_entities`), crates/verbatim/src/cmd/search.rs,
  docs/json-shapes.md
- **Action:** `Matched.values` already collects the distinct
  `(kind, value_norm)` pairs the query matched on each candidate turn, and
  today only `values.len()` survives into `Hit::entity_count`. Add a public
  field on `Hit` that carries those pairs themselves, filled in
  `weight_by_entities` where `entity_count` is filled, empty for a hit that
  matched no entity. Do not change the score summation or the ordering in any
  way - `Matched::saw`'s doc comment warns that a re-ranking hidden inside a
  reporting change is the regression every existing test still passes.
  `entity_count` stays and must equal the new field's length. Extend the
  per-hit object `cmd/search.rs` emits under `--json` with the pairs (an array
  of objects naming kind and value), and update the `search` section of
  `docs/json-shapes.md:71-78` to document it. Leave the MCP tool output
  unchanged - D-05 names only the `search --json` contract.
- **Verify:** `cargo test -p verbatim-core recall` and
  `cargo test -p verbatim recall_cli` pass; `verbatim search --json` against a
  test store shows a hit whose matched-pairs array names the entity the query
  matched, and a free-text-only hit shows an empty array.

### Task 2: Add `decisions` and `labels` to the schema, surviving reindex and reaching upgraded stores

- **Files:** crates/verbatim-core/src/store/schema.rs (symbols `CREATE_SQL`,
  `TABLES`, `DERIVED_TABLES`), crates/verbatim-core/tests/store_open.rs,
  crates/verbatim-core/tests/feedback.rs
- **Action:** Add two tables to `CREATE_SQL` and `TABLES`, and to NEITHER
  `DERIVED_TABLES` nor `BRING_FORWARD_COLUMNS`. `decisions`: integer primary
  key, `session_id` TEXT, `ts` TEXT (wall-clock UTC, ISO-8601), `cwd` TEXT,
  `prompt` TEXT, `watermark_session_no` INTEGER (nullable - null when the
  prompt path never opened the store), `chars_injected` INTEGER, and TEXT
  columns holding JSON for the list-shaped payload: extracted candidate
  spellings, scored candidates (turn id, relevance, entity score, entity
  count, matched pairs), injected turn ids with per-turn chars, suppressed
  turn ids with reasons, and the thresholds in force. `labels`: references a
  decision id, a nullable turn id, a label TEXT, a detail column, and a
  labeled-at timestamp; index on the decision id. Keep both tables in the
  archival comment group of `CREATE_SQL`, with a comment stating why reindex
  must never drop them: a decision records prompt-time state no blob replay
  can reconstruct (D-03, a recorded divergence from `DESIGN-BRIEF.md:92`).
  Write two tests: (a) fabricate a phase-5 store by opening a fresh store and
  executing `DROP TABLE decisions; DROP TABLE labels;`, reopen through
  `Store::open`, assert both tables exist, `rebuild_required()` is `None`, and
  a pre-existing `turns` row count is unchanged (no reindex was triggered);
  (b) insert a `decisions` row, run `reindex::reindex`, assert the row is
  byte-identical after.
- **Verify:** `cargo test -p verbatim-core store_open feedback` passes,
  including the two new tests; `cargo test -p verbatim-core reindex` still
  passes.

### Task 3: The decision record module - shape, file writer, file reader

- **Files:** crates/verbatim-core/src/inject/decision.rs (new),
  crates/verbatim-core/src/inject/mod.rs
- **Action:** New module declaring the decision record: session id, wall-clock
  timestamp, cwd, prompt text, optional watermark (max `sessions.session_no`
  at decision time), extracted candidate spellings, scored candidates each
  carrying turn id, relevance, entity score, entity count and the matched
  `(kind, value_norm)` pairs from task 1, injected turn ids with per-turn char
  counts and the total, suppressed turn ids with their `state::Reason`,
  whether the compacted pool was in force and the dropped turn ids when it
  was, and the threshold values in force (the six constants of
  `inject/prompt.rs` plus the effective char budget). Serialize as JSON with a
  private format-version field exactly as `inject/state.rs` does, so an
  unrecognized document reads as nothing. Write one file per prompt event into
  a `decisions` subdirectory of the data directory using the same
  tmp+rename, `create_new`, allow-listed-name discipline `state.rs` uses -
  reuse or mirror its `file_name` allow-list for the session id, and make the
  full file name collision-proof across prompts of one session (pid,
  timestamp, attempt counter). Every write failure is silent (`bool`/`Option`
  like `State::save`) - a record that cannot land must never cost the user's
  prompt. Also provide the read side the drain will use: list the directory,
  parse each file fail-open (a file that is not this build's format yields
  nothing but still reports its path so the caller can delete it).
- **Verify:** `cargo test -p verbatim-core inject::decision` (unit tests in
  the module) shows: a record round-trips through write and read; a hostile
  session id (`../evil`, separators, NUL) produces no filesystem call; two
  records for one session land as two files; bytes that are not this format
  read as nothing.

### Task 4: Every UserPromptSubmit writes a decision record

- **Files:** crates/verbatim-core/src/inject/prompt.rs (symbols
  `user_prompt_submit`, `select`, `selected`, `surviving`, `render`),
  crates/verbatim-core/src/inject/decision.rs,
  crates/verbatim-core/tests/inject_prompt.rs
- **Action:** Restructure the prompt arm so every exit writes one decision
  record - including `selected`'s early returns for an empty/missing prompt or
  cwd and for an empty candidate set before the store opens (D-11: non-fires
  are where miss data lives), the store-open failure arm, the search-error
  arm, and the ordinary fire/non-fire outcomes. The total injected chars are
  known only after `render` clips, so the record must be completed where
  `user_prompt_submit` has the rendered text (or its per-hit clipped shares) -
  restructure `render`/`user_prompt_submit` so the count reaches the record;
  a record for a non-fire carries zero chars. When the store opened, read max
  `sessions.session_no` on the already-open connection and stamp it as the
  watermark (D-10); when it never opened, leave the watermark null for the
  drain to stamp. Suppressions recorded for THIS prompt go into the record
  with their reasons - collect them where `surviving` calls
  `record_suppressed`, do not re-read the state file, whose list is capped
  and cumulative. The write happens at the end of the arm, after the state
  save, on the same abandoned thread; a failed write changes nothing about
  what is emitted. Do not move any store write onto this path and do not
  alter the threshold, pool, or suppression logic.
- **Verify:** `cargo test -p verbatim-core inject_prompt` passes with new
  assertions: a firing prompt's record names the injected turn ids, their
  matched pairs, nonzero chars and the six threshold values; a
  threshold-non-fire record has empty injected and the scored candidates; a
  prompt with no path- or identifier-shaped word yields a record with empty
  candidates and a null watermark; a suppressed-everything prompt's record
  carries the suppressions with reasons.

### Task 5: Ingest drains decision files into the `decisions` table

- **Files:** crates/verbatim-core/src/feedback/mod.rs (new),
  crates/verbatim-core/src/lib.rs, crates/verbatim-core/src/ingest/pass.rs
  (symbols `run_with`, `Summary`), crates/verbatim-core/tests/feedback.rs
- **Action:** Create the `feedback` module (registered in `lib.rs`) with the
  drain: read every decision file via task 3's reader, insert one `decisions`
  row per record inside a transaction, stamp any null watermark with the
  current max `sessions.session_no`, and delete each drained file only after
  its transaction commits - a kill mid-drain must lose no record and
  duplicate none on rerun, so delete-after-commit per batch is the rule.
  A file that does not parse is deleted and its path noted. Wire the drain
  into `pass::run_with` after `recover::recover` and BEFORE the walk - the
  before-walk position is what makes the drain-time watermark stamp the max
  session number as it stood before this pass admits new sessions, the
  tightest bound available for a record whose prompt never opened the store
  (D-10). Add the drained count and any malformed-file notes to `Summary` so
  `record_pass` carries them into `runs.error`'s notes the way recovery lines
  already travel. The single-file `ingest::run` entry point does not drain -
  D-01 names the pass, and the hook only ever spawns the pass.
- **Verify:** `cargo test -p verbatim-core feedback` shows: files written by
  task 3 become rows with all fields intact after `pass::run_with`; the
  decision files are gone; a second pass inserts nothing new; a record with a
  null watermark gets one stamped; a garbage file is removed and the pass
  still commits.

### Task 6: End-to-end - three prompt shapes through the real hook, and the budget holds

- **Files:** crates/verbatim/tests/decisions.rs (new)
- **Action:** An integration test driving the built `verbatim` binary the way
  `crates/verbatim/tests/hook.rs` does (its `feed`/fixture pattern): against a
  store with indexed history, feed `UserPromptSubmit` three payloads - one
  whose prompt names an indexed entity (injects), one naming an entity-shaped
  token the archive has not seen (threshold non-fire), one of plain prose
  (empty candidates, store never opened) - then run `verbatim ingest` and
  assert three `decisions` rows with the shapes AC1 names. Assert hook stdout
  stayed the one-object-or-nothing protocol on all three.
- **Verify:** `cargo test -p verbatim decisions` passes, and
  `cargo test -p verbatim hook` still passes including
  `every_event_exits_zero_with_an_empty_stdout_inside_the_budget` (the 10 ms
  p99 assertion).

## Notes

- `labels` joins `decisions` outside `DERIVED_TABLES` (the call CONTEXT left
  to the planner): dropping it on reindex would leave `verbatim stats`
  reporting zero precision until the next ingest pass relabels, for no gain -
  recomputation under changed rules is what `verbatim replay` (PLAN-3) is
  for. Recorded divergence from `DESIGN-BRIEF.md:92` extends D-03's.
- Watermark stamping for records that never opened the store is done at drain
  time, before the walk. Sessions ingested between the prompt and the next
  pass can inflate that bound by a pass's worth of sessions; the alternative
  (opening the store on the empty-candidate path) is exactly what the early
  return exists to avoid.
