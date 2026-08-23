---
phase: 6
plan: 2
requirements: [FEED-02]
files:
  - crates/verbatim-core/src/index/entity.rs
  - crates/verbatim-core/src/feedback/finalize.rs
  - crates/verbatim-core/src/feedback/label.rs
  - crates/verbatim-core/src/feedback/mod.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/tests/index.rs
  - crates/verbatim-core/tests/label.rs
---

# Phase 6: Feedback Loop - Plan 2 (outcome labels)

## Goal

Ingest turns logged decisions into outcomes: idle sessions are finalized, and
each finalized decision is labeled hit, false positive, miss or wasted budget
by SQL joins against the transcript that followed - never by re-reading blobs.

## Must be true when done

- An ingest pass marks `session_meta.is_final` on sessions whose last turn is
  older than the idle threshold, and leaves younger sessions unset with zero
  labels.
- On a fixture session past the threshold with one injected turn referenced
  downstream and one never referenced, the pass writes `hit` and
  `false positive` labels respectively.
- A fixture transcript containing a `recall_search` tool call for an entity a
  logged decision declined to inject gets that decision labeled `miss`.
- Labeling decompresses no blob and re-reads no transcript: it is SQL over
  `decisions`, `entities`, `turns` and `session_meta`.
- Labeling is incremental: a second pass over the same store writes no
  duplicate label.

## Context

- Runs after PLAN-1 (shares `pass.rs` and `feedback/mod.rs`; needs the
  `decisions` and `labels` tables and drained rows).
- D-06: "finalized" is the idle rule only (~6 h), written at ingest; no
  SessionEnd path exists or may be added. `is_final` is declared and written
  by nothing today (`schema.rs:79`, the `write_session_meta` comment).
- D-07: the label join runs entirely in SQL over `entities` - the injected
  turn's `(kind, value_norm)` rows against rows of later turns in the same
  session. D-04: the downstream boundary is the decision's wall-clock `ts`
  against `turns.ts`, joined through `session_id`.
- D-13: `miss` is defined against `recall_search`/`recall_get` tool calls in
  the transcript; zero exist on real history, so fixtures synthesize them.
- Out of scope: replay and stats (PLAN-3), any auto-tuner, retention/eviction.

## Tasks

### Task 1: Extract entities from recall tool calls

- **Files:** crates/verbatim-core/src/index/entity.rs (symbol
  `Collector::tool_use`), crates/verbatim-core/tests/index.rs
- **Action:** A `tool_use` block whose `name` ends with `recall_search` gets
  its `input.query` treated exactly as a `Grep` `pattern` is (the `symbols`
  rule) plus path-shaped words as `path` entities, and its `input.paths`
  array entries normalized as `path` entities; suffix match because the
  harness registers MCP tools under a server-prefixed name
  (`mcp__verbatim__recall_search`), and the bare name must match too. Without
  this the `miss` join has nothing in `entities` to say what the model
  searched FOR, and D-07 forbids reading the blob to find out. Do NOT bump
  `DERIVED_SCHEMA`: zero recall tool calls exist across all 3,217 measured
  transcripts (D-13), so no upgraded store loses a row it would have had, and
  a bump forces the measured ~50 s in-lock rebuild D-02 exists to avoid. Keep
  the extraction order deterministic and inside `MAX_ENTITIES_PER_TURN`, as
  the `extract` doc requires.
- **Verify:** `cargo test -p verbatim-core index` passes with a new test: a
  synthesized record whose `tool_use` block is named
  `mcp__verbatim__recall_search` with a query naming `SearchManager` and a
  `paths` entry yields `symbol` and `path` entities for them, and the same
  record re-derived yields the identical list.

### Task 2: The idle rule writes `is_final`

- **Files:** crates/verbatim-core/src/feedback/finalize.rs (new),
  crates/verbatim-core/src/feedback/mod.rs,
  crates/verbatim-core/src/ingest/pass.rs (symbol `run_with`),
  crates/verbatim-core/src/ingest/mod.rs (symbol `write_session_meta` doc
  comment), crates/verbatim-core/tests/label.rs
- **Action:** A finalize step, wired into the pass after the walk (so turns
  this pass just committed count toward `last_turn_at`) and before the `runs`
  row: one UPDATE setting `session_meta.is_final = 1` where `is_final IS
  NULL` and `last_turn_at` is older than a module constant of 6 hours,
  compared in SQL via `strftime` against `'now'` (the stored timestamps are
  ISO-8601 UTC text, so lexicographic comparison is sound); rows with a null
  `last_turn_at` are left alone. The threshold is one named constant with the
  calibration in its doc comment (measured 2026-08-20: 150 sampled
  transcripts, 0 internal idle gaps over 6 h). No un-finalize arm - D-06
  accepts the early-finalize risk and names raising the constant as the fix.
  Update `write_session_meta`'s comment, which still says `is_final` stays
  null until phase 8. Sidecar sessions inherit no special casing: they carry
  their own `last_turn_at`.
- **Verify:** `cargo test -p verbatim-core label` shows: after a pass, a
  fixture session whose last turn is 7 hours old has `is_final = 1`, one 5
  minutes old has `is_final` NULL, and a second pass changes neither.

### Task 3: The label join - hit, false positive, miss, wasted budget

- **Files:** crates/verbatim-core/src/feedback/label.rs (new),
  crates/verbatim-core/src/feedback/mod.rs,
  crates/verbatim-core/src/ingest/pass.rs (symbol `run_with`)
- **Action:** A labeling step wired into the pass directly after finalize,
  inside the same lock, operating only on decisions that (a) belong to a
  session `session_meta` marks final - joined through
  `session_meta.session_id = decisions.session_id`, which for sidecars means
  the parent's id and deliberately admits sidecar turns as "downstream" - and
  (b) have no `labels` row yet, which is what makes the step incremental and
  bounded. Per injected turn (unpack the decision's injected-ids JSON with
  SQLite's built-in `json_each`; that is still "entirely in SQL" under D-07):
  label `hit` when any of the injected turn's `entities` rows with kind
  `path`, `symbol`, `error` or `command` - never `tool`, whose values recur
  in every session and would label everything hit - also appears as an
  `entities` row of a turn in the same session with `turns.ts` greater than
  the decision's `ts`; label `false positive` otherwise. Label `miss` (one
  row per decision, null turn id) when a downstream turn of that session
  whose `turns.tool_name` ends with `recall_search` or `recall_get` carries
  an `entities` row whose value the decision's candidate spellings contain
  but its injected set does not. Label `wasted budget` (one row per decision,
  null turn id, chars in the detail column) when the decision injected a
  nonzero char count and none of its injected turns earned `hit`. Never read
  a blob, never parse transcript text; the whole step is SQL over stored
  rows. Report the label counts through `Summary` into the pass notes only if
  a count is nonzero.
- **Verify:** `cargo test -p verbatim-core label` shows on a hand-built
  store: a finalized decision with one downstream-referenced injected turn
  and one unreferenced gets exactly one `hit` and one `false positive` row; a
  non-final session's decision gets zero rows; rerunning the pass adds no
  duplicate; a decision whose every injected turn is unreferenced also gains
  a `wasted budget` row carrying its chars.

### Task 4: Fixture proof of AC2 and AC3 end to end

- **Files:** crates/verbatim-core/tests/label.rs
- **Action:** Two ingest-driven tests over synthesized transcript fixtures
  (the `testkit` temp-store pattern, real SQLite, no mocks). AC2: build a
  transcript whose turns carry entities, write two decision files via
  `inject::decision` naming two injected turns - one whose path entity a
  later turn's tool record repeats, one never repeated - age the session
  past the threshold (timestamps in the transcript text control
  `last_turn_at`), run `pass::run_with` twice (first drains, sessions
  finalize, labels land), and assert `hit` and `false positive`. AC3: a
  transcript containing a synthesized `mcp__verbatim__recall_search`
  `tool_use` record whose query names an entity that a logged decision's
  candidate spellings contain but its injected list does not; assert that
  decision is labeled `miss`. Both assert the labels arrived without any
  blob read beyond ingest itself - e.g. by labeling on a second pass that
  ingested nothing new.
- **Verify:** `cargo test -p verbatim-core label` passes; the two tests fail
  if the label rows are absent, duplicated, or attached to the wrong
  decision.

## Notes

- The `tool`-kind exclusion in the hit join is planner discretion under
  D-07's frame: a `tool` entity like `Read` appears in nearly every session,
  and including it would make `hit` structurally certain - the failure D-04's
  "every decision in a busy session labels hit" warns about.
- No `DERIVED_SCHEMA` bump for task 1 means turns ingested before this build
  lack recall-query entities until a manual `verbatim reindex`; on the
  measured corpus that set is empty (D-13), and the cost of the bump is the
  ~50 s in-lock rebuild D-02 rules out.
