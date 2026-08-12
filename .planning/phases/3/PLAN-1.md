---
phase: 3
plan: 1
requirements:
  - RCL-01
  - RCL-02
  - RCL-03
  - RCL-04
files:
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/index/mod.rs
  - crates/verbatim-core/src/index/text.rs
  - crates/verbatim-core/src/index/expand.rs
  - crates/verbatim-core/src/index/entity.rs
  - crates/verbatim-core/src/derive.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/index.rs
  - crates/verbatim-core/tests/derive.rs
  - crates/verbatim-core/tests/fixtures.rs
  - crates/verbatim-core/tests/reindex.rs
  - crates/verbatim/tests/corpus.rs
  - tests/fixtures/README.md
  - tests/fixtures/session-recall.jsonl
  - tests/fixtures/session-errors-a.jsonl
  - tests/fixtures/session-errors-b.jsonl
  - tests/fixtures/subagents/agent-echo.jsonl
---

# Phase 3: Recall - Plan 1 of 4 (derivation)

**SEQUENTIAL: PLAN-1 must complete before PLAN-2, PLAN-2 before PLAN-3, and
PLAN-3 before PLAN-4.** PLAN-1 and PLAN-2 both write
`crates/verbatim-core/src/lib.rs` and `crates/verbatim-core/src/store/open.rs`;
PLAN-2, PLAN-3 and PLAN-4 build on the query layer PLAN-2 creates; PLAN-3 and
PLAN-4 both write `crates/verbatim/src/main.rs` and
`crates/verbatim/src/cmd/mod.rs`. Do not run them in parallel.

## Goal

The index under recall: every turn's searchable text is a projection of what
the turn actually said rather than the JSON line that carried it, expanded so
that `manager` finds `SearchManager`, and every structured tool record leaves
behind exact-match entities of the five kinds - path, command, error, symbol,
tool - with the same error recurring in a later session normalizing to the
value the earlier one already carries.

## Must be true when done

- A turn containing `SearchManager` is returned by a search for `SearchManager`
  and by a search for `manager`, and a turn containing `src/worker/S.ts` is
  returned by a search for `worker`.
- A search for `toolUseResult` returns nothing: no JSON key and no record-type
  literal is ever indexed.
- Two turns in different sessions whose stderr differs only in line numbers,
  addresses, timestamps and UUIDs carry one identical `error` entity value,
  while two whose stderr differs only in a bare integer carry two different
  values.
- A `Read` tool_use naming a file emits a `path` entity and a `paths` row; an
  assistant message naming that same path in prose emits neither.
- Over the real corpus every one of the five entity kinds is emitted, and no
  turn carries more entities than the cap.
- Dropping `turns_fts`, `entities` and `paths` and running `verbatim reindex`
  reproduces the same rows, and a store written by a phase 2 binary rebuilds on
  the next `verbatim ingest` because `DERIVED_SCHEMA` moved.

## Context

- D-01 locks the FTS body as a per-record-type text projection plus expansion
  tokens, never the raw line. `crates/verbatim-core/src/derive.rs` currently
  inserts `String::from_utf8_lossy(row.record)` into `turns_fts`, which is why
  `MATCH 'assistant'` returns records whose only `assistant` is `"type":
  "assistant"`.
- D-02 locks the entity source to the record's own bytes: `message.content[]`
  `tool_use` and `tool_result` blocks plus the top-level `toolUseResult`
  object. No cross-record join between a call and its result, and nothing is
  extracted from prose or from any other subtree.
- D-03 locks the error signal as `tool_result.is_error == true` plus a
  non-empty `toolUseResult.stderr`; the transcript carries no exit code.
- D-04 locks RCL-03 normalization to UUIDs, timestamps, `0x` addresses and
  `:line:col`, stopping deliberately short of bare integers (measured: the
  aggressive rule scores worse on the recurrence the rule exists for).
- D-13 says camelCase splitting is the only expansion that changes recall;
  snake, kebab and path components already tokenize under `unicode61`. D-14
  says the size cost lands inside budget, so no index-size mitigation is
  designed in. D-15 sets the entity cap in the tens and leaves its shape to
  this plan. D-16 says `turns.tool_name` already captures the only `tool_use`
  block a record carries.
- D-17 requires the `DERIVED_SCHEMA` bump this plan's body-shape change forces,
  and D-18 keeps the rebuild off every read path - `reindex::open_up_to_date`
  stays the only caller that acts on `Store::rebuild_required`.
- Out of scope here: query-time IDF weighting, project scoping, exclusion on
  read, excerpts and every CLI or MCP surface (PLAN-2 onward).

## Tasks

### Task 1: Add the phase-3 fixtures

- **Files:** tests/fixtures/session-recall.jsonl,
  tests/fixtures/session-errors-a.jsonl, tests/fixtures/session-errors-b.jsonl,
  tests/fixtures/subagents/agent-echo.jsonl, tests/fixtures/README.md,
  crates/verbatim-core/src/testkit.rs, crates/verbatim-core/tests/fixtures.rs
- **Action:** Add four synthetic transcript fixtures shaped to what this phase
  must prove, register each in `testkit::TRANSCRIPT_FIXTURES`, and document
  each in the README table and prose the way every existing fixture is
  documented. `session-recall.jsonl` carries a turn whose message text contains
  `SearchManager`, a turn whose message text contains `src/worker/S.ts`, a
  `Read` `tool_use` block whose `file_path` names a file, and a separate
  assistant text turn naming that same path in prose and nothing else - those
  last two are AC3's structured-versus-prose pair and must be different turns.
  `session-errors-a.jsonl` and `session-errors-b.jsonl` are two distinct
  sessions each carrying `tool_result` blocks with `is_error: true` and a
  top-level `toolUseResult` object with `stdout`, `stderr` and `interrupted`
  keys, matching the key set D-02 measured: one stderr pair across the two
  files differing ONLY in a line number, a `0x` address, a timestamp and a
  UUID, and a second pair differing ONLY in a bare integer that is not a line
  number. `subagents/agent-echo.jsonl` is a sidecar reporting
  `session-recall.jsonl`'s `sessionId` with `isSidechain: true`, carrying one
  turn whose message text is byte-identical to a turn in
  `session-recall.jsonl` - that identical pair is what lets PLAN-2 assert
  D-07's "sorts below at equal score" against two genuinely equal BM25 scores.
  Do not edit any existing fixture: `session-truncated.jsonl` is a byte prefix
  of `session-basic.jsonl` and `session-large-record.jsonl`'s first-line offset
  is load-bearing for phase 1's block-count assertions, both asserted in
  `crates/verbatim-core/tests/fixtures.rs`. Do not use the token `brillig`
  anywhere in the new files - `testkit::UNIQUE_TOKEN` promises exactly one
  match across the whole set. Every new file ends with a trailing newline and
  contains no `\r`, and each record that is a turn carries both `uuid` and
  `timestamp` so D-03 classifies it. Add the per-fixture assertions to
  `tests/fixtures.rs` that pin the properties above, in the style of the
  existing `session_basic_carries_all_fifteen_record_types`.
- **Verify:** `cargo test -p verbatim-core --features testkit --test fixtures`
  passes, and `cargo test --workspace --features testkit` still passes with the
  four new fixtures in `TRANSCRIPT_FIXTURES` (the loops and `len()`-based
  assertions in `tests/reindex.rs`, `tests/parse.rs`, `tests/ingest.rs`,
  `tests/compaction.rs` and `crates/verbatim/tests/cli.rs` pick them up
  automatically; any that do not are the ones to fix).

### Task 2: Project turn text instead of indexing the raw line

- **Files:** crates/verbatim-core/src/index/mod.rs,
  crates/verbatim-core/src/index/text.rs, crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/src/derive.rs, crates/verbatim-core/src/store/open.rs,
  crates/verbatim-core/src/testkit.rs, crates/verbatim-core/tests/index.rs
- **Action:** Create the `index` module and give it a pure function that turns
  one turn record's bytes into the text that belongs in `turns_fts.body`
  (D-01). The projection reads string leaves out of exactly four subtrees of
  the record and nothing else: `message.content` (a plain string, or an array
  whose `text` blocks contribute their `text`, whose `tool_use` blocks
  contribute the tool `name` and the string leaves of `input`, and whose
  `tool_result` blocks contribute their `content` whether string or array of
  text blocks), the top-level `content` when it is a string (the `system`
  record in `session-basic.jsonl` carries `"content": "ran cargo build
  --workspace"` there), the top-level `toolUseResult` (string, or the string
  leaves of the object - `stdout`, `stderr` and the rest), and the top-level
  `attachment` object's string leaves. JSON keys and the record-type literal
  are never emitted, which is what makes a search for `toolUseResult` return
  nothing. Bound the recursion depth and the total projected bytes per turn so
  a pathological record cannot make one FTS row unbounded. Wire it into
  `derive::derive_turn`, which currently inserts
  `String::from_utf8_lossy(row.record)`: parse `row.record` into a
  `serde_json::Value` ONCE inside `derive_turn` and hand a reference to the
  projection (task 4 takes the same reference), because `parse::Record`
  deliberately does not keep the parsed value - a `Scan` holds every record of
  a session and a `Value` per record would hold the whole session's JSON tree
  in memory. A record that will not parse projects to empty text, never to the
  raw line; that path is unreachable for a turn (D-03 requires parsed `uuid`
  and `timestamp`) and exists so the fallback can never reintroduce
  scaffolding. Bump `DERIVED_SCHEMA` in `crates/verbatim-core/src/store/open.rs`
  from 2 to 3 with a doc comment naming the body-shape change (D-17), and leave
  `Store::open` exactly as it is - the rebuild belongs to
  `reindex::open_up_to_date` and to nothing else (D-18). Replace the entries in
  `testkit::FIXED_QUERIES` that only ever matched JSON scaffolding (`assistant`
  and `attachment` match `"type": "assistant"` today and match nothing after
  this change) with queries drawn from projected fixture text; the constant's
  own doc comment already states the rule that every query must match at least
  one fixture turn, so keep that true.
- **Verify:** A new `crates/verbatim-core/tests/index.rs` shows that over a
  store holding every fixture, `SELECT count(*) FROM turns_fts WHERE turns_fts
  MATCH 'toolUseResult'` is 0 and the same query for `"assistant"` returns only
  turns whose message text contains the word, while a phrase taken from a
  fixture's message text still returns its turn; `cargo test --workspace
  --features testkit` passes.

### Task 3: Expand turn text into camel, snake, kebab and path components

- **Files:** crates/verbatim-core/src/index/expand.rs,
  crates/verbatim-core/src/index/mod.rs,
  crates/verbatim-core/src/index/text.rs,
  crates/verbatim-core/tests/index.rs
- **Action:** Add the expansion half of RCL-01 as unit-testable pure functions
  (the design bars a custom FTS5 tokenizer precisely so every rule stays one):
  given the projected text, emit the additional tokens that let a query for one
  component find the whole. camelCase and PascalCase splitting is the rule that
  actually changes recall (D-13, measured: `MATCH 'manager'` does not return a
  `SearchManager` row under `unicode61`); snake, kebab and path-separator
  components are emitted for rule completeness even though `unicode61` already
  splits on those separators. Emit tokens deduplicated per turn - D-14's
  1.16x-1.37x measurement is over per-turn-unique tokens, and duplicates buy
  nothing under BM25 except body bytes. Append the expansion tokens to the
  projected text so one `turns_fts.body` value carries both; do not add a
  second FTS column, which would change the table declaration at
  `crates/verbatim-core/src/store/schema.rs:150-151` and the contentless-delete
  rebuild property with it. Leave digits attached to their token: no
  bare-integer handling is specified anywhere in this phase.
- **Verify:** `cargo test -p verbatim-core --features testkit --test index`
  shows that over a store holding every fixture, a `MATCH` for `SearchManager`
  and a `MATCH` for `manager` both return the same turn id, and a `MATCH` for
  `worker` returns the turn containing `src/worker/S.ts`.

### Task 4: Extract path, command, symbol and tool entities from tool records

- **Files:** crates/verbatim-core/src/index/entity.rs,
  crates/verbatim-core/src/index/mod.rs, crates/verbatim-core/src/derive.rs,
  crates/verbatim-core/tests/index.rs, crates/verbatim-core/tests/derive.rs
- **Action:** Extract entities from the record `Value` that task 2 already
  parses, reading only D-02's subtrees - `message.content[]` `tool_use` and
  `tool_result` blocks and the top-level `toolUseResult` object - and write
  them through `derive::derive_turn`, which already clears `entities` and
  `paths` for the turn before anything is inserted, so a re-derive stays
  idempotent by construction. Four kinds here; `error` is task 5. `tool`: the
  `tool_use` block's `name`, which D-16 measured as at most one per record and
  which `parse::record::tool_name` already lifts into `turns.tool_name` - emit
  the same value so a `kind = 'tool'` filter and the column cannot disagree.
  `path`: the `file_path`, `path` and `notebook_path` inputs of a `tool_use`,
  the `filePath` key of `toolUseResult`, and path-shaped argv words inside a
  `Bash` `command` string; normalized by trimming surrounding quotes and a
  trailing `:line:col`, and otherwise kept exactly as written - never
  canonicalized against the filesystem, because 52% of the corpus's `cwd`
  directories no longer exist (phase 2 D-05) and a path that resolves today
  would normalize differently tomorrow. Every `path` entity also gets a row in
  the `paths` table, which exists for exactly that lookup. `command`: from a
  `Bash` `tool_use`'s `command` input, the program token - the first argv word
  that is not a `NAME=value` assignment, reduced to its basename. `symbol`:
  identifier-shaped tokens taken only from a `Grep` `pattern` input and an
  `Edit` `old_string` / `new_string` input, where identifier-shaped means an
  internal case or underscore boundary (camelCase, PascalCase, snake_case with
  two or more segments) - a rule that admits `SearchManager` and refuses every
  ordinary English word, which is what keeps RCL-02's "not from prose" true for
  a field that can contain prose. Do NOT extract from the top-level
  `attachment` object even though task 2 projects its text: D-02 names three
  subtrees and an attachment is not one of them. Store each entity as a
  `(kind, value_norm)` pair and never reject one for being common - RCL-04
  forbids index-time rejection, and commonness is PLAN-2's query-time problem.
- **Verify:** `cargo test -p verbatim-core --features testkit --test index`
  shows that the `Read` `tool_use` turn in `session-recall.jsonl` has a `path`
  entity and a `paths` row naming the file while the prose turn naming the same
  path has neither, that the `Grep` turn in `subagents/agent-alpha.jsonl` has a
  `tool` entity equal to its `turns.tool_name`, and that the `Bash` turn in
  `session-basic.jsonl` has a `command` entity of `cargo`.

### Task 5: Extract and normalize error entities

- **Files:** crates/verbatim-core/src/index/entity.rs,
  crates/verbatim-core/tests/index.rs
- **Action:** Add the `error` kind on D-03's signal and nothing else: a
  `tool_result` block with `is_error: true`, or a non-empty
  `toolUseResult.stderr`. There is no exit-code field anywhere in the
  transcript - a scan of every `toolUseResult` in a 300-file sample found only
  `status`, `returnCodeInterpretation`, `success` and `code`, all on non-Bash
  tools - so do not look for one, and do not treat a `Bash` result's
  `interrupted` flag as an error. Normalize the error text per D-04 by
  replacing UUIDs, ISO-8601 timestamps, `0x` hex addresses and `:line:col`
  suffixes with a fixed placeholder each, and stop there: bare integers are
  deliberately NOT stripped, because the aggressive rule was measured to merge
  genuinely different errors (46 recurring values before, 39 after). Reduce the
  normalized text to a single-line value so two occurrences that differ only in
  trailing whitespace or line wrapping collapse, and bound its length so one
  enormous stderr cannot become an enormous entity value.
- **Verify:** `cargo test -p verbatim-core --features testkit --test index`
  shows that the two turns from `session-errors-a.jsonl` and
  `session-errors-b.jsonl` whose stderr differs only in a line number, address,
  timestamp and UUID share one `entities.value_norm` for `kind = 'error'`,
  while the pair differing only in a bare integer has two distinct values.

### Task 6: Cap the entities a single turn emits

- **Files:** crates/verbatim-core/src/index/entity.rs,
  crates/verbatim-core/src/index/mod.rs, crates/verbatim-core/tests/index.rs
- **Action:** Bound the entities one turn writes with a named constant carrying
  the measurement that set it: D-15 measured p50 3, p90 10, p99 38 and max 146
  per turn over a 300-file sample, and requires a cap in the tens, so the cap
  binds the tail and not the common turn. Apply it as a total per turn, after
  deduplicating on `(kind, value_norm)`, in the deterministic order the record
  walk produces - which is what keeps a rebuild reproducing the same rows, the
  property AC3 asserts and the reason the cap must never be applied by
  frequency, recency or any other data-dependent ordering. The cap is a bound
  on emitted rows and is not a stop-list: RCL-04 forbids rejecting an entity at
  index time for being common, and a value dropped by position because a turn
  carried 150 of them is not a value rejected for what it is.
- **Verify:** `cargo test -p verbatim-core --features testkit --test index`
  shows a synthetic turn carrying more tool-derived values than the cap writes
  exactly the cap's number of `entities` rows, and that deriving that same turn
  twice writes the same rows both times.

### Task 7: Prove the derived tables still rebuild to identical rows

- **Files:** crates/verbatim-core/src/testkit.rs,
  crates/verbatim-core/tests/reindex.rs
- **Action:** `testkit::query_set_json` is the fixed query set AC3 and phase
  1's AC4 both compare across a rebuild, and today it covers only `turns_fts`
  rowids plus a turn range and a turn-id lookup - so an entity extractor that
  rebuilt to different rows would leave it byte-identical. Extend it to include
  the `entities` rows (turn_id, kind, value_norm) and the `paths` rows
  (turn_id, path) in a stable order, keeping the output deterministic and
  byte-comparable across two runs the way it already is. Keep the existing FTS
  and turn-range sections unchanged so the phase 1 comparison keeps meaning
  what it meant.
- **Verify:** `cargo test -p verbatim-core --features testkit --test reindex`
  and `cargo test -p verbatim --features testkit --test cli` both pass,
  including
  `reindex_rebuilds_the_derived_tables_to_byte_identical_query_output`, and the
  captured query-set JSON contains non-empty entity and path sections (a
  comparison over two empty sections would pass on an extractor that emitted
  nothing).

### Task 8: Assert the derivation over the real corpus

- **Files:** crates/verbatim/tests/corpus.rs
- **Action:** Extend the existing corpus test - the one pass over the private
  tree, gated on `testkit::CORPUS_DIR_ENV` and already isolating both the data
  and the config directory - with AC3's corpus-level claims, against the store
  that pass produced: every one of the five entity kinds appears in `entities`
  with a non-zero count; no turn exceeds task 6's cap; the `paths` table is
  non-empty and every `paths` row has a matching `path` entity for the same
  turn. Print the per-kind counts and the per-turn entity distribution the way
  the file already prints its rebuild timing, because those numbers are what a
  later phase tunes against and a bare assertion records none of them. Assert
  no absolute count and no ratio: the corpus is live and grows during the run,
  which is why every existing assertion in this file is a set relation.
- **Verify:** With `VERBATIM_TEST_CORPUS` set to the real Claude config
  directory, `cargo test -p verbatim --features testkit --test corpus -- --nocapture`
  passes and prints a non-zero count for each of the five kinds; with the
  variable unset the test still skips loudly and CI stays green.

## Notes

- Structure deviates from the CONTEXT `Plan shape` directive: four plans rather
  than three. The middle seam (terminal recall and the JSON contract) needs
  fourteen single-concern tasks against a task ceiling of eight, so it is split
  into the core query layer (PLAN-2) and the CLI surface (PLAN-3). The seams
  CONTEXT named are otherwise honored, and the plans are sequential, not
  parallel.
- The projection this plan installs is what makes `testkit::FIXED_QUERIES`
  meaningful for the first time. Two of its five current entries only ever
  matched JSON scaffolding, so phase 1's AC4 comparison has been comparing
  hits nobody would ever search for.
- `session_meta.agent_meta` holds the sidecar's `agent-*.meta.json` bytes and
  phase 2 D-04 stored them so this phase could extract `description` for
  ranking without a reingest. Nothing in phase 3's locked decisions asks for
  that extraction, so no task does it; it is available to phase 5 unchanged.
