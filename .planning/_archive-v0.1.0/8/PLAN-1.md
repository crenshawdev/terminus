---
phase: 8
plan: 1
requirements: [RET-01, RET-03]
files:
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/retention/mod.rs
  - crates/verbatim-core/src/retention/apply.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/tests/config.rs
  - crates/verbatim-core/tests/retention.rs
  - crates/verbatim-core/tests/pass.rs
---

# Phase 8: Retention And Lifecycle - Plan 1 (policy, evaluation, and the pass step)

## Goal

A store that can be told how much history to keep - `keep`, `evict` or
`delete`, globally or per project, off unless `verbatim.toml` says otherwise -
with the decision made once by one evaluation function and applied as a bounded
step at the end of every ingest pass, under the lock that pass already holds.

## Must be true when done

- A config directory with no `verbatim.toml`, and one whose `verbatim.toml` has
  no `[retention]` table, both resolve to a retention policy that selects
  nothing: an ingest pass over a corpus older than any plausible default evicts
  zero sessions, deletes zero, and leaves `verbatim sessions` returning the same
  count before and after.
- A `[retention.project."<path>"]` table takes effect for a session whose stored
  project key is that path or below it, and the global `[retention]` table
  applies to every other session, with the longest matching project key winning.
- With `action = "evict"` and an age configured, a pass empties the blob of
  every session past that age and sets `session_meta.is_evicted`, leaving the
  `sessions` row, the `session_meta` row and every derived row in place.
- `delete` acts only on sessions whose transcript file is no longer on disk, and
  removes that session's blob, metadata, turns, FTS rows, entities, paths,
  boundary rows, observation row and watermark.
- One pass never acts on more than a fixed bound of sessions, and what it did -
  and what it left for the next pass - is readable in `verbatim status` off
  `runs.error`, because there is no log file.
- A session that is still live is never touched: only sessions the idle rule has
  already closed (`session_meta.is_final = 1`) are eligible.

## Context

- D-01 binds the configuration: a `[retention]` table on `FileConfig`, hand-
  edited only, and per-project keys matched through the SAME normalizer
  `exclude` uses - never string equality against `session_meta.project`, whose
  values are canonical git toplevels like `/data/code/scratch` while the design
  brief's own example writes a bare `scratch`. Nothing in this workspace can
  write TOML: the `toml` dependency is `default-features = false` with the
  serializer half deliberately absent.
- D-11 binds the age test: `session_meta.last_turn_at` compared
  lexicographically in SQL against `strftime('%Y-%m-%dT%H:%M:%fZ','now','-N
  days')`, gated on `is_final = 1`. `crates/verbatim-core/src/feedback/finalize.rs`
  (`finalize`) is that statement shape already and says why.
- D-02 binds `delete`: it is a garbage collector for what Claude Code's own
  `cleanupPeriodDays` already removed, never an independent age rule. A session
  whose transcript still exists is rediscovered by `discover::discover` on the
  next hook spawn and re-ingested from offset 0, so deleting it reclaims nothing.
- D-10 binds the placement: a new step inside `pass::locked`, after
  `observe::observe_new` and before `record_pass`.
- Out of scope here: the `--dry-run` command (PLAN-2), the `is_evicted` arms in
  `verify`, `reindex` and the ingest append path (PLAN-2), and the brief's
  `max_size_gb` cap, which appears in no phase-8 requirement and no decision.

## Tasks

### Task 1: Resolve a retention policy from `verbatim.toml`

- **Files:** crates/verbatim-core/src/config.rs, crates/verbatim-core/tests/config.rs
- **Action:** Add a `[retention]` table to the private `FileConfig` struct in
  `config.rs` alongside `injection` and `provider`, with the same
  `#[serde(default)]` treatment so a file without it and a file without any
  `verbatim.toml` at all resolve identically. It carries an `action` of `keep`,
  `evict` or `delete` and an `age_days` integer, and a nested per-project map
  whose keys are project paths as the user writes them
  (`[retention.project."/data/code/scratch"]`), each carrying the same two
  optional fields. Resolve it in `Config::resolve` into a public retention
  policy on `Config` and expose it through accessor methods, following how
  `provider` and the injection budgets are already resolved and exposed. Every
  per-project key goes through the same `normalize` function `Config::from_parts`
  already applies to `exclude`, and a key that normalizes to `None` is dropped
  exactly as an exclusion is. Selecting the policy for a session's stored project
  key uses `config::covers` - the existing component-wise ancestor test that
  `Config::excludes_path` and `recall::scope` share - and where more than one
  configured key covers the session, the one `covers` reports deepest wins, which
  is the rule `recall::scope` already uses for project keys. Defaults are the
  off state: no `[retention]` table means `keep` with no age, and an `age_days`
  of 0 or absent means the policy selects nothing whatever the action says, so
  "off by default" is a property of the resolved value rather than of a caller
  remembering to check. An unrecognized `action` string resolves to `keep` under
  the same rule `ResponseFormat::parse` states for an unrecognized value: this
  file grows across phases and a typo must not make a store start deleting.
  Write nothing: no command in this workspace writes `verbatim.toml`, and there
  is no TOML serializer linked to write it with.
- **Verify:** `cargo test -p verbatim-core --test config` passes with new cases
  showing (a) a config directory with no file and a file with no `[retention]`
  table both resolve to a policy that selects nothing, (b)
  `[retention.project."/data/code/scratch"]` is selected for a session project
  key of `/data/code/scratch/sub` and not for `/data/code/scratch-other`, (c)
  where both `/data/code` and `/data/code/scratch` are configured, a session
  under the latter gets the latter's action, and (d) `action = "nonsense"`
  resolves to keep rather than failing the load.

### Task 2: One evaluation function that names what retention would act on

- **Files:** crates/verbatim-core/src/retention/mod.rs, crates/verbatim-core/src/lib.rs, crates/verbatim-core/tests/retention.rs
- **Action:** Add a `retention` module, declared in `lib.rs` beside `recover`
  and `reindex`, holding ONE function that takes a `&rusqlite::Connection`, a
  `&Config` and an explicit `cutoff: &str` timestamp, and returns the session
  keys it would evict and the session keys it would delete, mutating nothing. It
  is the single evaluation both the pass step (task 4) and PLAN-2's `verbatim
  retention` report call. Sharing the function pins the RULE, not the instant:
  `'now'` re-evaluates on every call, so the cutoff is computed ONCE by the
  caller and passed in, and each caller reports the cutoff it used. Two calls
  straddling an age boundary still name different sets, and that is why the
  cutoff is reported rather than implied - a disagreement is then explainable
  instead of invisible. Callers compute it with
  `strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?)` and a `-N days` parameter; age is
  `session_meta.last_turn_at` compared lexicographically in SQL against it, the
  statement shape `feedback::finalize::finalize` already uses and for the reason
  it states: stored timestamps are fixed-width ISO-8601 UTC text, so string order
  is time order and no date parsing happens. Only sessions with
  `is_final = 1` are eligible, and a session with a null `last_turn_at` is never
  eligible - it is not idle, it is unknown. A session already carrying
  `is_evicted` is not offered for eviction again. Because the policy is
  per-project and resolved in Rust (task 1), the query selects the candidate
  rows with their project keys and the policy is applied per row rather than
  pushed into SQL. The `delete` arm adds D-02's rule on top of the age test: a
  session is deletable only when its transcript is no longer on disk, tested
  with `std::fs::metadata` on `session_meta.transcript_path`, because a session
  whose file still exists is rediscovered and re-ingested on the next hook
  spawn and deleting it reclaims nothing measurable. Both result lists are
  ordered oldest-first by `last_turn_at` and each is truncated to a
  `MAX_PER_PASS` constant declared in this module - set it to 100, mirroring
  `observe::MAX_PER_PASS`, for the reason that constant's own doc block gives:
  this work runs inside the ingest lock, so it is bounded the way the rest of the
  pass is, and the hook spawn is the scheduler so the remainder is picked up on
  the next prompt. The return value also reports how many eligible sessions were
  left over, so the caller can say so. With a policy that selects nothing the
  function issues no query at all and returns empty.
- **Verify:** `cargo test -p verbatim-core --test retention` passes with new
  cases over a real temp store showing (a) an off policy returns empty sets and
  the sets are empty for a store whose sessions are all older than any age, (b)
  a session with `is_final` null is never selected however old, (c) a session
  whose `last_turn_at` is inside the window is not selected and one outside it
  is, (d) with `action = "delete"` a session whose transcript file still exists
  is not selected while one whose file has been removed is, and (e) with more
  eligible sessions than the bound, exactly the bound are returned, oldest
  first, and the leftover count is the rest.

### Task 3: Apply an eviction and a deletion

- **Files:** crates/verbatim-core/src/retention/apply.rs, crates/verbatim-core/src/retention/mod.rs, crates/verbatim-core/tests/retention.rs
- **Action:** Add the mutating half of the `retention` module, taking the keys
  task 2 produced and a `&mut Store` (it opens transactions). Each session is
  its own transaction, for the reason `ingest::pass` gives for per-file
  transactions: a kill loses one session's worth of work and never leaves the
  store half-way through a batch. Eviction sets `sessions.blob` to an empty blob
  and `session_meta.is_evicted = 1` in that transaction and touches nothing else
  - `sessions.blob` is declared `BLOB NOT NULL` in
  `crates/verbatim-core/src/store/schema.rs`, so the column takes zero bytes and
  never a null - and leaves `session_meta.checksum` and `uncompressed_len` as
  they are, because `recover::recover`'s lowered-watermark sweep compares
  `watermarks.byte_offset` against `session_meta.uncompressed_len` and lowering
  it here would make every pass "repair" every evicted session forever.
  Deletion removes the session's rows child-first, because the bundled SQLite is
  compiled with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1` and the declared references in
  `schema::CREATE_SQL` really are enforced: `paths` and `entities` by turn id,
  `compaction_boundaries` by turn id, `turns_fts` by rowid (which IS `turns.id`),
  then `turns`, then the `observations` row (its `session_key` declares a
  reference to `sessions`), then `session_meta`, then `sessions`, and finally the
  `watermarks` row for that transcript path so nothing claims bytes for a session
  that is gone. `decisions` and `labels` are deliberately untouched: `decisions`
  is keyed on `session_id` with no declared reference, `labels.turn_id` carries
  no foreign key on purpose, and FEED-03's replay history must survive a
  deletion. Report per session what was done and, for a deletion that removed an
  `observations` row, say so in the note - that row is a paid model call and its
  loss is the one thing a delete destroys that no rebuild reproduces. A failure
  on one session is recorded against that session and the loop continues, the
  way `pass::walk` records a per-file failure and carries on: retention must not
  be able to wedge on one damaged row.
- **Verify:** `cargo test -p verbatim-core --test retention` passes with new
  cases showing (a) after an eviction the `sessions` row still exists with a
  zero-length blob, `session_meta.is_evicted` is set, and the session's `turns`,
  `turns_fts`, `entities` and `paths` row counts are unchanged, (b) after a
  deletion no row for that session remains in `sessions`, `session_meta`,
  `turns`, `entities`, `paths`, `compaction_boundaries`, `observations` or
  `watermarks`, and a `turns_fts` MATCH that previously returned one of its
  turns returns nothing, (c) a `decisions` row naming the deleted session's
  `session_id` survives the deletion, and (d) a second session in the same batch
  is still processed after one is made to fail.

### Task 4: Run retention as a bounded step at the end of every pass

- **Files:** crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/tests/pass.rs
- **Action:** Add the retention step to `pass::locked`, between the existing
  `summary.observations = crate::observe::observe_new(...)` call and
  `record_pass`. The position is D-10 and both halves of it matter: after
  `observe_new` because that step reads one blob per newly finalized session and
  an eviction that ran first would destroy the bytes it was about to read, and
  before `record_pass` because `runs.error` is the only textual channel this
  product has and a step placed after that row is written has nowhere to report.
  Add a field to `Summary` holding what the step did, following how
  `observations`, `outcomes` and `feedback` are already carried, give it a
  `lines()` method in the shape `Observed::lines` returns, and extend
  `record_pass`'s note chain to fold it in beside the others. Nothing here may
  fail the pass: the archive is the work, a retention step that could not run is
  a note, and the `?` that would propagate its error is exactly what would let
  one damaged row stop every future ingest of every other transcript. The step
  calls task 2's evaluation and then task 3's apply, and it reports the leftover
  count when the bound truncated the work so a reader can tell "there is nothing
  left to do" from "the rest is coming next pass". A pass whose policy selects
  nothing adds no note and leaves `runs.error` null, which is what makes AC1
  observable.
- **Verify:** `cargo test -p verbatim-core --test pass` passes with new cases
  showing (a) a pass with no `[retention]` configured leaves every session's blob
  non-empty, `is_evicted` unset, and `runs.error` null, (b) a pass with an evict
  policy and an aged corpus leaves the aged sessions evicted and writes a
  `runs.error` line naming what it did, and (c) with more eligible sessions than
  the bound, one pass acts on exactly the bound and the following pass acts on
  the rest, with the first pass's `runs.error` saying how many were left.

## Notes

- Plan shape: this phase's CONTEXT names three seams (retention, lifecycle ops,
  capture mode). The 4-task-per-plan ceiling splits the retention seam across
  PLAN-1 and PLAN-2; they share `crates/verbatim-core/tests/retention.rs` and are
  SEQUENTIAL, PLAN-1 first.
- `DESIGN-BRIEF.md:378` also specifies a `max_size_gb` cap applied oldest-first.
  It appears in none of RET-01..RET-05 and in no phase-8 decision, and D-11
  defines age and only age, so it is not planned here. Raise it as its own
  requirement if it is wanted.
- Deleting a session removes its `observations` row, and that is forced by the
  schema rather than chosen: `observations.session_key` declares a reference to
  `sessions(session_key)` and foreign keys are enforced in this build. D-02
  confines `delete` to sessions whose transcript is already gone, which is the
  only reason that loss is acceptable.
