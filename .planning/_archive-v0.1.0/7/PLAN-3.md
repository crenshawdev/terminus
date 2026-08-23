---
phase: 7
plan: 3
requirements: [OBS-02, OBS-03, OBS-04, OBS-06, OBS-08]
files:
  - crates/verbatim-core/src/observe/mod.rs
  - crates/verbatim-core/src/observe/judgment.rs
  - crates/verbatim-core/src/observe/cost.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/src/recall/mod.rs
  - crates/verbatim-core/src/recall/scope.rs
  - crates/verbatim-core/src/recall/search.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/judgment.rs
  - crates/verbatim-core/tests/cost.rs
  - crates/verbatim-core/tests/recall.rs
  - crates/verbatim/src/cmd/ingest.rs
  - crates/verbatim/src/cmd/observations.rs
  - crates/verbatim/src/cmd/mcp/tools.rs
  - crates/verbatim/src/cmd/search.rs
  - crates/verbatim/tests/mcp.rs
  - crates/verbatim/tests/observations.rs
  - crates/verbatim/tests/recall_cli.rs
  - docs/json-shapes.md
---

# Phase 7: Observations - Plan 3 (judgment, its cost, its failure arm, its reach)

## Goal

With judgment switched on, a finalized session costs exactly one provider call
and produces strict JSON whose every claim points at a real turn in the
archive; a response that will not parse is retried once and stored rather than
dropped; the cost controls hold; and the result is reachable from the model
through the recall tools it already has.

## Must be true when done

- With judgment enabled, a finalized session above the minimum turn count
  produces exactly one HTTP request, and the stored row's `decisions`,
  `learned` and `unresolved` entries each carry a `turn_id` that resolves to a
  real `turns` row of that session.
- A session below the minimum turn count, a session already carrying a
  judgment, and any session after the daily token budget is exhausted each
  produce zero requests.
- A response that will not parse produces exactly two requests and one row with
  `status = parse_failed` holding the raw response, and the ingest pass over
  the same session still completes and writes its `runs` row.
- A full ingest and hook run with judgment disabled records zero attempts at
  the connection seam, and a provider request in flight does not stop a second
  `verbatim ingest` from taking the ingest lock.
- `recall_search` with the observation `kind` returns hits carrying the
  anchoring `turn_id`, scoped and excluded like every other hit, and there is
  still no fourth MCP tool.
- No stream, error or stored column anywhere on these paths carries a
  credential value.

## Context

- D-07 (the provider call runs outside the ingest lock), D-11/D-12 (the daily
  token budget is a config key held in a `meta` row; the minimum turn count and
  the truncation budget are compile-time constants), D-17 (OBS-08 is one new
  value on the existing `kind` filter, switching `recall_search` to a second
  query branch), D-19 (the SessionStart resume brief does not read observations
  in this phase - `crates/verbatim-core/tests/inject_brief.rs` fails if any
  file under `src/inject/` contains the string).
- PLAN-1 declared the `observations` columns this plan fills - `status`,
  `model`, `prompt_version`, `topic`, `outcome`, `decisions`, `learned`,
  `unresolved`, `raw`, `tokens` - and PLAN-2 built `observe::net`,
  `observe::egress`, `observe::provider`, `credentials` and the `TcpListener`
  stub in `testkit`.
- Task 4 closes AC6's ingest half for PLAN-2's PRIV-03: the assertion is
  possible only once judgment is wired into a pass.

## Tasks

### Task 1: The judgment request and the anchoring contract

- **Files:** crates/verbatim-core/src/observe/judgment.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/tests/judgment.rs
- **Action:** Build the one call a finalized session gets. The schema sent as
  `response_format` is the design brief's, fixed and versioned: a one-line
  `topic`, an `outcome` of `completed | partial | abandoned | exploratory`, and
  `decisions`, `learned` and `unresolved` as arrays of at most five entries
  each, every entry carrying a `turn_id`. Carry a prompt version string with
  the request and store it in the row's `prompt_version`, because
  `verbatim observations regenerate --prompt-version` selects on it. The input
  is built from the session's own turns, read once out of the session blob the
  way PLAN-1's mechanical module reads it, with each turn presented beside its
  real `turns.id` - a model cannot anchor a claim to an id it was never shown.
  Validate the parsed response before it becomes a row: every entry of all
  three lists must carry a `turn_id` that resolves to a real `turns` row OF
  THAT SESSION, and a response that fails this takes the same path as a
  response that would not parse (task 3) rather than being stored with the bad
  anchors dropped. That is the whole of OBS-03: an unverifiable claim is worse
  than no claim, because the anchor is the only thing that makes the summary
  auditable. Store the model name in `model`, the returned token count in
  `tokens`, and set `status` to say the call succeeded. Write nothing into the
  `mechanical` column - PLAN-1 owns it and a judgment run must not disturb it.
- **Verify:** `cargo test -p verbatim-core --test judgment` passes: against the
  testkit stub serving a schema-shaped response whose claim `turn_id`s are real
  ids of the fixture session, exactly one request is made and the stored row's
  three claim lists each carry entries whose `turn_id` selects a row from
  `turns`; against a response whose claims are otherwise valid but name a
  `turn_id` from a different session or no session at all, no row with a
  success status is written.

### Task 2: The cost controls

- **Files:** crates/verbatim-core/src/observe/cost.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/tests/cost.rs
- **Action:** Four gates, and OBS-06 splits them across two homes (D-11). The
  minimum turn count and the truncation budget are compile-time constants in
  this module with their doc comments carrying the reasoning, following
  `feedback::finalize::IDLE_HOURS`: a quality threshold must not become a
  setting, and a tunable minimum would turn "one call per session" into a knob.
  The daily token budget is the config key PLAN-2 added, because a user must be
  able to cap what a remote provider charges them without rebuilding. Hold the
  spend in a `meta` row carrying the date and the tokens spent, through
  `Store::meta_int` and `Store::set_meta_int` or an equivalent pair over the
  same table, reset when the date changes: every generation is a separate
  short-lived process by design, so an in-process counter would bound nothing
  (D-12). Accumulate the token count the provider returned in its `usage`
  object. The fourth gate is "never more than one call per session": a session
  whose row already carries a judgment status - success or `parse_failed` -
  is not called again by the pass, and only `verbatim observations regenerate`
  may ask a second time. Truncate the input with EXPLICIT elision markers so
  the model is told what it is not seeing rather than shown a session that
  looks complete and is not.
- **Verify:** `cargo test -p verbatim-core --test cost` passes: a finalized
  session with fewer turns than the minimum produces zero attempts in
  `observe::net`'s log; with the daily budget set below one call's cost and the
  meta row already at the limit, a session above the minimum produces zero
  attempts; a call that succeeds moves the meta row by exactly the stub's
  reported total; a meta row stamped with yesterday's date is reset rather than
  carried forward; a session already carrying a status produces zero attempts
  on a second run; the truncated input carries the elision marker.

### Task 3: The parse-failure arm

- **Files:** crates/verbatim-core/src/observe/judgment.rs, crates/verbatim-core/tests/judgment.rs
- **Action:** A response whose `content` is not the strict JSON the schema asked
  for - and, by task 1, a response whose claims do not anchor - is retried
  exactly once and then stored with `status = parse_failed` and the raw
  response text in `raw`. Exactly once: not zero, which loses the transient
  case the retry exists for, and not more, which doubles the bill on a model
  that will never comply. Never dropped silently, because the session is still
  in the archive and regeneration is always available, and a silently dropped
  failure is how the incumbent's schema drift became permanent invisible loss.
  Never blocks ingest: the failure is this module's own return value, and no
  caller may turn it into a failed pass. Both requests count against the token
  budget, and the raw response goes through `observe::egress`'s error scrubber
  before it is stored - it is a provider response this build did not author,
  and `raw` is a column `verbatim observations` prints.
- **Verify:** `cargo test -p verbatim-core --test judgment` passes: against the
  stub's unparseable arm, exactly two requests are recorded in `observe::net`'s
  log for one session, one row exists with `status = parse_failed` and a `raw`
  column holding the stub's body, and no third request is made on a later run
  of the same session; a stub whose first response is unparseable and whose
  second is valid produces two requests and a row with a success status.

### Task 4: Generation runs after the pass, outside the lock

- **Files:** crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim-core/src/testkit.rs, crates/verbatim/src/cmd/ingest.rs, crates/verbatim/tests/observations.rs
- **Action:** Wire judgment into the pass so that it runs only after the `runs`
  row commits and the ingest lock drops (D-07). `ingest::pass::run_with` holds
  `_guard` for the whole of its body today, so the step has to sit past the
  point where the guard and the store are released - never as a fourth step
  beside the drain, the walk and `outcomes`, because a slow or hanging provider
  holding the lock for the length of an HTTP call would make every hook-spawned
  pass in that window exit `LockHeld` and archive nothing. With the provider
  block absent or not enabled, this step resolves no credential, builds no
  request and touches nothing - which is the default state. Its notes travel
  back to the caller and `crates/verbatim/src/cmd/ingest.rs` prints them on
  stderr beside the per-file failures, scrubbed through `observe::egress`
  first: this step runs after the `runs` row is written, so stderr is the
  channel it has, and D-16 covers it either way. Add a stalling arm to the
  testkit `TcpListener` stub - it accepts the connection and holds the response
  for a bounded time - so the lock property can be asserted rather than argued.
- **Verify:** `cargo test -p verbatim --test observations` passes: a pass with
  judgment disabled over the whole fixture tree, plus a `verbatim hook` run,
  records zero attempts in `observe::net`'s log; a pass with judgment enabled
  against the stalling stub is running in a background thread while
  `ingest::lock::try_acquire` on the same data directory returns
  `Attempt::Acquired` from the test thread; a pass against the unparseable stub
  completes, writes its `runs` row, and exits 0. AC4's remote half is
  human-verify: point `base_url`, `model` and the key at a remote
  OpenAI-compatible endpoint, run an ingest over a finalized session, and
  observe a stored observation with the same shape the local endpoint produced,
  with nothing else in `verbatim.toml` changed.

### Task 5: `regenerate` re-requests the judgment half

- **Files:** crates/verbatim/src/cmd/observations.rs, crates/verbatim-core/src/observe/mod.rs, crates/verbatim/tests/observations.rs, docs/json-shapes.md
- **Action:** Extend PLAN-1's `verbatim observations regenerate` so that, for
  exactly the rows its `--since` and `--prompt-version` selectors chose, it
  also re-requests judgment when the provider block is enabled - which is what
  makes `--prompt-version` mean anything, since the reason to re-run is that
  the prompt changed. With the provider not enabled it recomputes the
  mechanical half and leaves the stored judgment columns exactly as they were,
  so a user without a provider can still rebuild facts. This is the one path
  allowed to call a second time for a session that already has a status (task
  2's fourth gate), and every other cost control still applies: the daily
  budget is checked per session and a run that exhausts it stops making
  requests and says how many sessions it left untouched, rather than either
  overspending silently or failing. Update the `### observations regenerate`
  section of `docs/json-shapes.md` with whatever fields the report gained, and
  keep `crates/verbatim/tests/cli.rs`'s transcription of that shape true.
- **Verify:** `cargo test -p verbatim --test observations` passes: with the
  provider enabled against the stub, `regenerate --since` over a store holding
  two rows with different `last_turn_at` values makes exactly one request and
  replaces exactly the later row's judgment columns, leaving the earlier row's
  bytes unchanged; with the provider disabled the same command makes zero
  requests and leaves both rows' judgment columns unchanged; with the daily
  budget exhausted it makes zero requests and reports that it stopped.

### Task 6: Observations reachable through the recall tools

- **Files:** crates/verbatim-core/src/recall/search.rs, crates/verbatim-core/src/recall/scope.rs, crates/verbatim-core/src/recall/mod.rs, crates/verbatim/src/cmd/mcp/tools.rs, crates/verbatim-core/tests/recall.rs, crates/verbatim/tests/mcp.rs, crates/verbatim/tests/recall_cli.rs, docs/json-shapes.md
- **Action:** One new value on `Filters::kind` and a second query branch, never
  a fourth MCP tool (OBS-08, and `DESIGN-BRIEF.md:184` bars one). `kind` today
  compiles to `AND t.record_type = ?` inside `Filters::push_onto`, and the
  search's `HEAD` reads `FROM turns_fts JOIN turns`, where `turns_fts` is
  contentless with `rowid IS turns.id` - so an observation has no natural row
  in it and the filter must switch `search::run` to a branch over the
  `observations` table rather than adding a clause to the existing statement.
  The branch returns `Hit` values shaped exactly like turn hits: one hit per
  CLAIM, walked out of the three claim columns with `json_each` the way
  `feedback::label` walks a decision's injected list, carrying that claim's own
  anchoring `turn_id`, the claim text as the excerpt, the observation's session
  key, the new value as `record_type`, and a deterministic total order so two
  runs over an unchanged store agree. It must not call `excerpt::attach`: the
  text is already in hand, and attaching would decompress a whole session for a
  hit whose `turn_id` points somewhere else. Scope and exclusion still apply -
  join `session_meta` and reuse the project clause and `push_exclusion` that
  the turn branch uses, because a branch that queried `observations` directly
  is how an excluded project becomes visible again. A store with no
  `observations` table answers empty with a new `recall::scope::Reason`
  variant, never a raw `no such table`: `kind` accepted and silently returning
  zero hits is indistinguishable from "nothing was found", which is the worst
  failure mode a recall tool has. Update the `kind` description in
  `crates/verbatim/src/cmd/mcp/tools.rs`, which today reads "Only turns of this
  record type: user, assistant, system or attachment", so the model is told the
  new value exists and what it returns; keep it one line, and add nothing that
  tells the model when to call the tool. `verbatim search --kind` reaches the
  same `Filters` field and gains the value with it - check its rendering of a
  hit still holds. Note in the branch's comment that `before_turn_id` is
  `verbatim replay`'s alone and replay never sets `kind`, so the branch does
  not implement it.
- **Verify:** `cargo test -p verbatim-core --test recall`,
  `cargo test -p verbatim --test mcp` and
  `cargo test -p verbatim --test recall_cli` pass: `recall_search` with the
  observation kind over a store holding a judged session returns at least one
  hit whose `turn_id` selects a real row from `turns`; the same query without
  that kind returns turn hits and no observation; a query matching no claim
  text returns zero hits and exit 0; an observation belonging to an excluded
  project is absent from the results while a sibling project's is present; the
  MCP server still advertises exactly three tools and
  `there_are_exactly_three_read_only_tools` still passes;
  `crates/verbatim-core/tests/inject_brief.rs`'s source assertion still passes,
  proving the resume brief was not quietly given observations.

## Notes

- The SessionStart resume brief is deliberately left without observations
  (D-19). `DESIGN-BRIEF.md:233` lists them as a brief block, so this is a
  recorded deferral and not an oversight; do not "fix" the brief.
- Retention, capture mode, compaction and export of observations are phase 8's.
