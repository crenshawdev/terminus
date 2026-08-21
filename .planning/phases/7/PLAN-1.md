---
phase: 7
plan: 1
requirements: [OBS-01, OBS-07]
files:
  - crates/verbatim-core/src/store/schema.rs
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/observe/mod.rs
  - crates/verbatim-core/src/observe/mechanical.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/tests/schema.rs
  - crates/verbatim-core/tests/reindex.rs
  - crates/verbatim-core/tests/observe.rs
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/observations.rs
  - crates/verbatim/src/cmd/read.rs
  - crates/verbatim/tests/cli.rs
  - crates/verbatim/tests/observations.rs
  - docs/json-shapes.md
---

# Phase 7: Observations - Plan 1 (mechanical facts, stored and listable)

## Goal

Every finalized session carries an auditable, parser-derived account of what
happened in it - files, tools, commands with their arguments, errors, branch,
commits, turn count, duration, compactions - written by ingest into a table
`reindex` never drops, listed by `verbatim observations`, and rebuilt only by
`verbatim observations regenerate`.

## Must be true when done

- An ingest pass over a fixture session old enough to be finalized leaves one
  `observations` row for it, and `verbatim observations` prints that session's
  files, tools, commands with their arguments, errors, branch, commits, turn
  count, duration and compactions.
- No model is called and no network connection is attempted on any path in this
  plan: the facts come from `turns`, `entities`, `paths`,
  `compaction_boundaries` and `session_meta`, plus one blob read per finalized
  session for the two facts SQL cannot answer.
- `verbatim reindex` on a store holding observations leaves every observation
  row present and unchanged.
- A store written before this build gains the table on its next write-mode
  open, with no forced rebuild of the derived tables; a read command against a
  store that still lacks it answers empty with a reason rather than a SQLite
  line.
- `verbatim observations regenerate --since <date>` recomputes only the
  sessions after that date and leaves every earlier row byte-identical.
- Both `observations` verbs keep the `--json` envelope, the stream split and
  the 0/1/2 exit codes the other data commands keep.

## Context

- D-01/D-02/D-03 bind the schema: declared in `CREATE_SQL` and `schema::TABLES`
  so `bring_forward`'s missing-table arm reaches existing stores, kept OUT of
  `schema::DERIVED_TABLES`, one table for both the mechanical and the judgment
  halves, and no declared reference to `turns`.
- D-04 binds the source of the facts, D-18 binds the CLI shape (first two-word
  subcommand: nested dispatch arm, a `docs/json-shapes.md` section, and a line
  in `crates/verbatim/tests/cli.rs`'s cross-command contract test).
- The judgment half of the row stays null through this plan and is filled by
  PLAN-3; the provider, credentials and egress boundary are PLAN-2's. Nothing
  here reads a config provider block or constructs an HTTP client.
- Follow `crates/verbatim-core/src/feedback/` for the pass-step shape and
  `crates/verbatim/src/cmd/stats.rs` for the read-command shape.

## Tasks

### Task 1: Declare the `observations` table

- **Files:** crates/verbatim-core/src/store/schema.rs, crates/verbatim-core/tests/schema.rs, crates/verbatim-core/tests/reindex.rs
- **Action:** Add an `observations` table to `CREATE_SQL` (start reading at the
  `labels` table and its `turn_id` comment, which is the precedent this
  follows) and append its name to `schema::TABLES`. It must NOT appear in
  `schema::DERIVED_TABLES` and `DERIVED_SCHEMA` must not move: D-01 measured a
  bump at roughly 49 s of blob replay inside the ingest lock for a table
  nothing rebuilds, and D-02 records that a `DERIVED_TABLES` entry would make
  `reindex` delete purchased summaries. One row per session, keyed
  `session_key TEXT PRIMARY KEY REFERENCES sessions(session_key)` - the
  reference to `sessions` is safe and matches `turns`, because `sessions` is an
  archive table `reindex` never drops. Declare NO reference to `turns` anywhere
  on this table (D-03): the bundled SQLite is built with
  `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`, so a declared reference to a table
  `reindex` drops would make the first `reindex` after the first observation
  fail outright and take every subsequent ingest with it. The columns are
  `session_key`, `session_id`, `generated_at`, `mechanical` (one JSON document
  holding the OBS-01 facts, the way `decisions` carries its list-shaped fields
  as JSON because nothing joins on them), then the judgment half PLAN-3 fills
  and this task only declares: `status`, `model`, `prompt_version`, `topic`,
  `outcome`, `decisions`, `learned`, `unresolved`, `raw`, `tokens`. The three
  claim-list columns are JSON arrays whose entries carry their own `turn_id`,
  so the anchor lives inside a document and no column can declare a foreign key
  for it. Write the table's comment so it states why it is archival rather than
  derived, the way the `decisions` comment does. Do not add anything to
  `BRING_FORWARD_COLUMNS`: that list is for columns added to a table that
  already shipped, and this is a whole new table.
- **Verify:** `cargo test -p verbatim-core --test schema` passes with
  `a_fresh_store_carries_exactly_the_phase_two_tables` seeing the new name
  through `schema::TABLES`; a new test drops `observations` from an initialized
  store, reopens it with `Store::open`, and finds the table back with
  `Store::rebuild_required()` still `None`; a new test in
  `crates/verbatim-core/tests/reindex.rs` inserts an observation row, runs
  `reindex::reindex`, and finds the row present with the same column values.

### Task 2: Compute the mechanical facts for one session

- **Files:** crates/verbatim-core/src/lib.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/src/observe/mechanical.rs, crates/verbatim-core/tests/observe.rs
- **Action:** Add `pub mod observe;` to `crates/verbatim-core/src/lib.rs` and a
  module that produces, for one `session_key`, the OBS-01 fact set: files read
  and modified, tools used, commands run with their arguments, errors seen,
  branch, commits, turn count, duration, compactions. Everything except
  commands-with-arguments and commits is SQL over `turns`, `entities`, `paths`,
  `compaction_boundaries` and `session_meta` (D-04): `paths` gives the files,
  `entities` of kind `tool` and `error` give the tools and errors,
  `session_meta.branch` gives the branch, `session_meta.first_turn_at` and
  `last_turn_at` give the duration, `count(*)` over `turns` gives the turn
  count, and a count over `compaction_boundaries` joined through `turns` gives
  the compactions. Those two exceptions need one blob read per session, and
  only one: `index::entity::program_of` reduces a Bash command to its basename,
  so `entities` holds `git` with the subcommand and arguments discarded, and
  `parse::record::Record` parses `gitBranch` and no commit field. Read the
  session stream once through `blob::read_all` over the row `sessions` holds,
  rescan it with `parse::scan`, and take the full command line off each `Bash`
  tool_use block and the commit subjects off the records that ran one - the
  rejected alternative was widening `entities` command normalization, which is
  a reindex-visible change to a shipped table. Bound what one document can
  hold: cap each fact list and say in the stored document that a list was cut,
  so one pathological session cannot write a megabyte of JSON into a row every
  `verbatim observations` run then prints. Nothing in this module opens a
  network connection, spawns a process, or reads a transcript from disk - the
  blob is the only source.
- **Verify:** `cargo test -p verbatim-core --test observe` passes a test that
  ingests `session-edits.jsonl` and `session-errors-a.jsonl` from the testkit
  fixtures, computes the facts for each, and asserts the file the `Edit` call
  touched, the tool names, an error value, the branch, the turn count and a
  duration are all present, and that a command with arguments comes back with
  its arguments and not merely its basename.

### Task 3: Ingest writes an observation for each newly finalized session

- **Files:** crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/tests/observe.rs
- **Action:** In `ingest::pass::run_with`, after `crate::feedback::outcomes`
  (which is what sets `session_meta.is_final`) and before `record_pass` writes
  the `runs` row, insert an `observations` row for every visible session that
  is final and has no row yet. Insert only - never overwrite an existing row -
  because D-02 makes `verbatim observations regenerate` the only rebuild path
  and a pass that recomputed would silently discard a purchased judgment half.
  Add a summary field beside `Summary::outcomes` carrying what this step did
  and whatever it could not do, following `feedback::Labeled`: it reports lines
  that `record_pass` folds into `runs.error`, and a failure here is a note and
  never a failed pass, for the same reason the drain and the labeller are notes
  - the archive is the work. Bound the work per pass the way the rest of the
  pass is bounded, since this reads one blob per newly finalized session.
- **Verify:** `cargo test -p verbatim-core --test observe` passes a test that
  runs a pass over a fixture tree whose sessions are older than
  `feedback::finalize::IDLE_HOURS`, finds exactly one `observations` row per
  finalized session and none for a session still open, runs a second pass and
  finds no new or changed row, and finds the `runs` row written either way.

### Task 4: `verbatim observations` lists them

- **Files:** crates/verbatim/src/cmd/observations.rs, crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs, crates/verbatim/tests/observations.rs, docs/json-shapes.md
- **Action:** A read command in the shape of `crates/verbatim/src/cmd/stats.rs`:
  opened through `cmd::read::open`, so a machine that has never ingested gets an
  empty answer with a reason and exit 0 rather than a store created by being
  asked a question. It takes `--project` (through `cmd::read::scope`, so
  `project: "*"` opts into cross-project) and `--json`, and nothing else. Every
  listed row must pass the read-side exclusion predicate in
  `config::visible` - a read path that queried `session_meta` directly is how
  an excluded project becomes visible again (ING-08). A store that predates the
  table must get an empty answer with a reason naming it, not a raw
  `no such table: observations` escaping the envelope: use
  `Store::missing_tables`, which is what `cmd::read::decision_log` was built on
  for exactly this failure. Print the mechanical facts in human mode and emit
  the whole row - the judgment columns included, null until PLAN-3 fills them -
  under `data` in `--json` mode, built with `cmd::json::Document` and never
  with `format!`. Register `observations` in `cmd/mod.rs` and in `main.rs`'s
  `dispatch` and `USAGE`. Document the shape in a new `### observations`
  section of `docs/json-shapes.md`.
- **Verify:** `cargo test -p verbatim --test observations` passes: after an
  ingest pass over a finalized fixture, `verbatim observations --json` exits 0
  and its `data` names that session's files, tools, a command with its
  arguments, errors, branch, commits, turn count and duration; the same command
  against an empty data directory exits 0, writes the envelope with a reason
  and creates no data directory; the same command against a store whose
  `observations` table was dropped exits 0 with a reason naming the table and
  no SQLite text on stdout.

### Task 5: `verbatim observations regenerate` rebuilds a selected set

- **Files:** crates/verbatim/src/cmd/observations.rs, crates/verbatim/src/main.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim/tests/observations.rs, crates/verbatim/tests/cli.rs, docs/json-shapes.md
- **Action:** Add the nested dispatch arm D-18 calls for: `main.rs`'s
  `dispatch` matches a flat set of single-word names, so `observations` must
  consume the next value itself and route `regenerate` from inside its own
  parser, leaving a bare `observations` as the list and an unknown second word
  as misuse (exit 2). `regenerate` takes `--since <date>` and
  `--prompt-version <version>` and `--json`. `--since` is parsed with
  `cmd::time_bound`, which is where every other command's date bound is
  validated, and selects sessions whose `session_meta.last_turn_at` is at or
  after it; `--prompt-version` selects rows carrying that value in the
  `prompt_version` column; given both, they narrow conjunctively; given
  neither, every visible session is selected. Planner's choice, recorded here:
  regenerate recomputes the mechanical half in place for exactly the selected
  rows and touches no other row and no other column, so a run under a narrowed
  selector is provably scoped. This command opens the store for writing rather
  than through `cmd::read::open`, and it is the only rebuild path for this
  table (D-02). Add both `observations` verbs to `DATA_COMMANDS` in
  `crates/verbatim/tests/cli.rs` so the five swept properties hold them -
  `sweep_args` puts the tuple's first element first on the command line and
  `every_data_command_emits_the_documented_shape` compares it against
  `value["command"]`, so extend that helper to carry a second word rather than
  exempting the command from the sweep. Document the shape in a
  `### observations regenerate` section of `docs/json-shapes.md`.
- **Verify:** `cargo test -p verbatim --test cli` and
  `cargo test -p verbatim --test observations` pass: a store holding rows for
  two sessions with different `last_turn_at` values, each row's `mechanical`
  column hand-edited to a recognizable wrong value, is regenerated with
  `--since` set between the two dates - the later row comes back with the
  computed facts and the earlier row still holds the hand-edited bytes;
  `verbatim observations regenerate --definitely-not-a-flag` exits 2 with
  nothing on stdout; `verbatim observations no-such-verb` exits 2.

## Notes

- The `observations.decisions` column and the `decisions` table are different
  objects with the same name. Qualify the column in every statement that names
  it; the alternative spellings all read worse than the design brief's own
  field name.
- PLAN-3 extends `regenerate` to re-request the judgment half when a provider
  is configured. This plan deliberately makes no provider call from any path,
  which is what lets its whole surface be exercised with the network unplugged.
