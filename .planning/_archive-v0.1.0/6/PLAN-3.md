---
phase: 6
plan: 3
requirements: [FEED-03, FEED-04]
files:
  - crates/verbatim-core/src/inject/prompt.rs
  - crates/verbatim-core/src/recall/search.rs
  - crates/verbatim-core/src/feedback/replay.rs
  - crates/verbatim-core/src/feedback/stats.rs
  - crates/verbatim-core/src/feedback/label.rs
  - crates/verbatim-core/src/feedback/mod.rs
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/replay.rs
  - crates/verbatim/src/cmd/stats.rs
  - crates/verbatim/tests/cli.rs
  - crates/verbatim/tests/replay.rs
  - crates/verbatim-core/tests/feedback.rs
  - docs/json-shapes.md
---

# Phase 6: Feedback Loop - Plan 3 (replay and stats)

## Goal

A retrieval change is tested against history before it ships: `verbatim
replay` re-runs every logged prompt against the index as it stood and reports
the label diff, and `verbatim stats` turns the labels into precision, misses,
and chars injected versus referenced.

## Must be true when done

- `verbatim replay` with a threshold override flag exits 0, reports a
  per-label diff across the logged history, and leaves the store file
  byte-identical.
- Replay scores each historic prompt only against turns that were indexed at
  decision time, bounded by the decision's recorded watermark - never against
  hindsight.
- The live injection path still compiles its thresholds in; no config file or
  flag detunes it.
- `verbatim stats` and `verbatim stats --json` report precision, miss count
  and chars injected versus referenced, validating against the documented
  shape, and equal hand-computed values on a fixture with a known outcome mix.
- Both commands live under the `{command, ok, reason, data}` envelope with the
  0/1/2 exit contract, documented in `docs/json-shapes.md`.

## Context

- Runs after PLAN-1 and PLAN-2 (shares `prompt.rs`, `search.rs`,
  `feedback/mod.rs`, `label.rs`, `docs/json-shapes.md`; needs `decisions` and
  `labels` rows to exist).
- D-08: live thresholds stay the compile-time constants in
  `inject/prompt.rs:62-99`; replay alone accepts CLI overrides. D-09: replay
  is a shipped subcommand under `--json`. D-10: "the index as it stood" is a
  watermark bound on `sessions.session_no`, never a snapshot. D-15: replay
  opens with `Store::open_read_only`, so "without touching the live store" is
  connection flags, not discipline. D-14: stats is CLI-only, one
  `DATA_COMMANDS` row, never an MCP tool.
- Out of scope: any auto-tuner acting on labels; MCP exposure of either
  command.

## Tasks

### Task 1: Parameterize the injection thresholds without detuning the live path

- **Files:** crates/verbatim-core/src/inject/prompt.rs (symbols `RANKED`,
  `COMPACTED_RANKED`, `ENTITY_RANK`, `CO_OCCURRING`, `MAX_TURNS`,
  `MAX_CANDIDATES`, `eligible`, `candidates`, `query_of`)
- **Action:** Introduce a thresholds value type in `prompt.rs` whose `Default`
  is exactly the six constants, and thread it through the decision logic
  (`eligible`'s rank and co-occurrence tests, the ranked-window widths, the
  cap, the candidate cap) so replay can pass overrides. The live path
  (`user_prompt_submit`/`select`) constructs the default inline - no `Config`
  field, no flag, no `verbatim.toml` key reaches it, because the
  precision-first defaults stay non-detunable (D-08,
  `DESIGN-BRIEF.md:245`). Also give replay a public seam for the extraction:
  `candidates` is private today, and replay must re-run the same
  spelling-extraction over a stored prompt and cwd - expose it (or a wrapper)
  without changing its behavior. Decision records already log the values in
  force (PLAN-1 task 4); make sure the logged values come from the thresholds
  value actually used, not from re-reading the constants.
- **Verify:** `cargo test -p verbatim-core inject_prompt` passes unchanged
  (same fixtures, same outcomes - proof the live path did not move), plus a
  new test showing the same ranked input fires differently under an
  overridden rank threshold.

### Task 2: A search bounded to the index as it stood

- **Files:** crates/verbatim-core/src/recall/search.rs (symbols `Filters`,
  `Filters::push_onto`), crates/verbatim-core/tests/feedback.rs
- **Action:** Add an optional upper turn-id bound to `Filters`, rendered by
  `push_onto` as a conjunctive `t.id < ?` predicate. Callers translate a
  session-number watermark into it via `store::schema::turn_id(watermark + 1,
  0)` - sound because a turn id is `session_no << TURN_SEQ_BITS | turn_seq`
  (`schema.rs:21-46`), so every turn of every session up to and including the
  watermark sits below that bound and every later-ingested session sits above
  it. Do not filter on `turns.ts`: it is transcript time, not ingest time,
  and cannot answer "was this row indexed yet" (D-10). Default stays `None`
  so every existing caller is untouched.
- **Verify:** `cargo test -p verbatim-core` passes; a new test ingests two
  sessions, searches with the bound set to the first session's number, and
  gets hits only from the first.

### Task 3: The replay engine

- **Files:** crates/verbatim-core/src/feedback/replay.rs (new),
  crates/verbatim-core/src/feedback/label.rs,
  crates/verbatim-core/src/feedback/mod.rs,
  crates/verbatim-core/tests/feedback.rs
- **Action:** For every `decisions` row: re-extract candidate spellings from
  the stored prompt and cwd through task 1's exposed seam, run the search
  with the same request shape the live arm builds (candidates, scope from
  the stored cwd, excerpts off) bounded by the row's watermark via task 2,
  apply the (possibly overridden) thresholds and - when the record says the
  compacted pool was in force - the recorded dropped-turn set, re-apply the
  record's own suppressed turn ids (session state files are long gone, and
  re-deriving suppression would attribute state noise to the rule change),
  and produce the would-inject set. Then recompute this decision's labels
  with the same rules PLAN-2's labeler applies - factor the label
  predicates in `label.rs` so replay and ingest share one definition rather
  than two that drift - and diff against the stored `labels` rows. The
  engine takes a `&Store` the caller opened; it executes only SELECTs.
  Output: per-label old and new counts, plus the decision ids whose labels
  changed. Determinism matters: two replays of an unchanged store with equal
  overrides must produce identical output.
- **Verify:** `cargo test -p verbatim-core feedback` shows: replay with
  default thresholds over a freshly-labeled fixture reports a zero diff;
  replay with a widened rank threshold on a fixture built to sit at rank 4
  reports the expected label movement; a decision whose watermark predates a
  later-ingested matching turn does not fire on it.

### Task 4: `verbatim replay` - the shipped subcommand

- **Files:** crates/verbatim/src/cmd/replay.rs (new),
  crates/verbatim/src/main.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/tests/replay.rs, docs/json-shapes.md
- **Action:** Wire a `replay` verb into `main.rs`'s match, parsed with lexopt
  like its siblings: `--json`, and one override flag per threshold constant
  (`--ranked`, `--compacted-ranked`, `--entity-rank`, `--co-occurring`,
  `--max-turns`, `--max-candidates`), each optional, defaulting to the
  compiled values. Open through `Store::open_read_only` exactly as
  `cmd/read.rs` does (reuse its `Opened` handling): a missing store is an
  empty result with a reason and exit 0, an unreadable one is operational
  exit 1, a store predating this build gets the stale warning on stderr.
  Human output prints the per-label old/new counts and the changed-decision
  count; `--json` emits the envelope through `cmd::json::Document` with the
  overrides in force, the per-label diff and the changed decision ids under
  `data`. Add a `replay` section to `docs/json-shapes.md` documenting the
  shape. The integration test proves the byte-identity claim: hash
  `verbatim.db` (and assert no `-wal`/`-shm` growth) before and after a
  replay run against a store with logged decisions.
- **Verify:** `cargo test -p verbatim replay` shows `verbatim replay
  --entity-rank 5 --json` exits 0, its stdout validates against the
  documented envelope with a per-label diff, and the store file's bytes hash
  identically before and after.

### Task 5: `verbatim stats` - precision, misses, chars injected versus referenced

- **Files:** crates/verbatim-core/src/feedback/stats.rs (new),
  crates/verbatim-core/src/feedback/mod.rs, crates/verbatim/src/cmd/stats.rs
  (new), crates/verbatim/src/main.rs, docs/json-shapes.md
- **Action:** A core aggregation over `decisions` and `labels`: decision
  count (non-fires included), injected-turn count, hit and false-positive
  counts, precision (hits over hits plus false positives, null when nothing
  was injected rather than a fabricated zero), miss count, wasted-budget
  count, total chars injected (sum over decisions), and chars referenced
  (sum of per-turn injected chars for turns labeled hit - the per-turn
  counts are in the decision record from PLAN-1). The command follows
  `cmd/status.rs`'s shape exactly: `cmd::json_flag` parsing, human lines and
  a `--json` `Document` carrying the same numbers and no others, opened
  read-only via `cmd/read.rs` (stats is a read; a missing store is a
  reasoned empty answer, exit 0). Wire into `main.rs` beside `status`. Add a
  `stats` section to `docs/json-shapes.md`. D-12: these are chars, the
  existing proxy - no tokenizer, and the output names them chars.
- **Verify:** `cargo test -p verbatim-core feedback` covers the aggregation
  against a hand-built label mix; `verbatim stats` and `verbatim stats
  --json` on that fixture print numbers equal to the hand-computed values
  (AC5), and `verbatim stats` on an empty store exits 0 with a reason.

### Task 6: `stats` joins the swept CLI contract

- **Files:** crates/verbatim/tests/cli.rs (symbol `DATA_COMMANDS`)
- **Action:** Add one `DATA_COMMANDS` row for `stats` naming its documented
  `data` fields, exactly as the table's comment instructs ("a seventh data
  command added in a later phase joins DATA_COMMANDS on one line and is held
  to the whole contract by that alone"). The sweep then enforces the
  envelope, the exit codes, the stdout/stderr split and the field list
  against `docs/json-shapes.md`'s transcription for free (D-14). `replay` is
  deliberately not added: the sweep runs every command against a store with
  no decisions, and its field expectations assume a data command's ordinary
  shape - if the sweep passes with a `replay` row too, add it; if not,
  record why in the test comment rather than bending the sweep.
- **Verify:** `cargo test -p verbatim cli` passes with the new row;
  removing a documented field from the `stats` emitter makes
  `every_data_command_emits_the_documented_shape` fail.

## Notes

- Replay re-applies recorded suppressions rather than recomputing them: the
  per-session state files are disposable scratch (deleted or stale by replay
  time), and the diff must attribute label movement to the extraction or
  threshold change under test, not to unreproducible session state.
- The phase's three plans are sequential (1 then 2 then 3), not parallel: the
  CONTEXT `Plan shape` directive of multiple plans is honored, but they share
  `pass.rs`, `prompt.rs`, `search.rs`, `feedback/mod.rs` and
  `docs/json-shapes.md`, so the file-independence test forbids parallel
  execution.
