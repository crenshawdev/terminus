---
phase: 8
plan: 2
requirements: [RET-02, RET-03]
files:
  - crates/verbatim-core/src/verify.rs
  - crates/verbatim-core/src/reindex.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/tests/verify.rs
  - crates/verbatim-core/tests/reindex.rs
  - crates/verbatim-core/tests/retention.rs
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/retention.rs
  - crates/verbatim/tests/cli.rs
  - crates/verbatim/tests/retention.rs
  - docs/json-shapes.md
---

# Phase 8: Retention And Lifecycle - Plan 2 (an evicted session survives everything, and you see it coming)

## Goal

An evicted session stays listed and stays searchable with its body flagged as
evicted - through `verify`, through `reindex`, and through every later ingest
pass - and `verbatim retention --dry-run --json` names the sessions the next
pass will act on before any of it happens.

## Must be true when done

- `verbatim verify` on a store holding evicted sessions reports no findings for
  them and still names a genuinely corrupt blob and only it.
- `verbatim reindex` on a store holding evicted sessions leaves those sessions
  listed by `verbatim sessions`, still matched by a search that matched one of
  their turns before, and still answering `body_evicted` from `recall_get` -
  while every other session is rebuilt from its blob exactly as before.
- An ingest pass that walks the transcript of an evicted session leaves it
  evicted: no bytes are read into it, no per-file failure is recorded for it,
  and its watermark does not move.
- `verbatim retention --dry-run --json` writes the `{command, ok, reason, data}`
  envelope naming the cutoff it judged against plus the session ids it would
  evict and the ones it would delete, changes no row in any table, and the
  ingest pass that follows acts on exactly that set for every session not
  sitting on the age boundary between the two.
- `verbatim retention` keeps the whole data-command contract: `--json` on
  stdout, diagnostics on stderr, exit 0 including for an empty result.

## Context

- D-03 is the whole of tasks 1 and 2: `verify.rs` selects `s.blob` for every row
  and calls `blob::read_all`, which fails on an empty blob and would report every
  evicted session as corrupt; `reindex.rs` rebuilds `turns`/`turns_fts` from
  `sessions.blob` alone and pushes an unreadable blob onto `Rebuilt::failed`; and
  `Existing::read` plus `prepare` call `blob::append` on the resume path.
  `recover::recover` calls `reindex::open_up_to_date` at the top of EVERY run, so
  an unhandled reindex is silent data loss on the next `DERIVED_SCHEMA` bump.
- D-04 is task 3: `--dry-run` is `verbatim retention --dry-run`, its own `--json`
  data command, sharing PLAN-1's ONE evaluation function with the pass-embedded
  step. Retention still RUNS in the pass; this command only reports.
- D-17 is task 4: `retention` joins `DATA_COMMANDS` in
  `crates/verbatim/tests/cli.rs` and `docs/json-shapes.md`, and that table's own
  comment says a later-phase data command "joins `DATA_COMMANDS` on one line and
  is held to the whole contract by that alone".
- Depends on PLAN-1: `Config`'s retention policy, the evaluation function and
  the apply half all land there.

## Tasks

### Task 1: The `is_evicted` arms in `verify` and on the ingest append path

- **Files:** crates/verbatim-core/src/verify.rs, crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/tests/verify.rs, crates/verbatim-core/tests/retention.rs
- **Action:** In `verify::walk`, carry `m.is_evicted` alongside the
  `m.transcript_diverged` column the statement already selects, and give an
  evicted session an arm of its own: it is still counted in `Report::checked`
  and it produces no `Failure`, because its blob was emptied on purpose and
  reporting it as corrupt is exactly the confusion the divergence arm's own
  comment exists to prevent. The divergence check stays independent of it, the
  way it is already independent of the checksum verdict. Leave the message and
  the behaviour for a genuinely corrupt blob untouched - the CONTEXT puts that
  explicitly out of scope. In `ingest::Existing::read`, read `is_evicted` from
  the same `session_meta` row the `agent_meta IS NOT NULL` and
  `coalesce(transcript_diverged, 0)` flags already come from, so the steady
  state costs no extra query, and in `ingest::prepare` give it an arm before the
  blob work: an evicted session returns `Prepared` with no `work`, so
  `ingest::apply` answers `Outcome::UpToDate`, no bytes are read, no watermark
  moves and no per-file failure is recorded. That is the only safe answer -
  `blob::append` parses a header out of the stored bytes and an emptied blob has
  none, so left alone it becomes a per-file failure on every pass forever, and
  writing the tail as a fresh blob instead would resurrect a session retention
  deliberately emptied while leaving every stored `turns.stream_offset` pointing
  into bytes that are no longer there.
- **Verify:** `cargo test -p verbatim-core --test verify` and
  `cargo test -p verbatim-core --test retention` pass with new cases showing
  (a) `verify` on a store where one session was evicted and another had a byte
  flipped inside its blob names the corrupt session and no other, and counts
  both as checked, (b) re-running an ingest of the evicted session's transcript
  after eviction returns `Outcome::UpToDate`, leaves the blob empty, leaves
  `is_evicted` set and leaves the watermark at the value it had, and (c) that
  pass records no entry in `Summary::failures`.

### Task 2: `reindex` preserves what an evicted session's blob can no longer produce

- **Files:** crates/verbatim-core/src/reindex.rs, crates/verbatim-core/tests/reindex.rs
- **Action:** `reindex::reindex` drops every table in `schema::DERIVED_TABLES`
  and rebuilds from `sessions.blob`. An evicted session has no blob, and its
  `turns_fts` row cannot be reconstructed by any other means: the table is
  declared `content=''`, so the projected body text is not readable back out of
  it, and the record bytes `index::project` built it from are gone. So give
  `reindex` a second path, taken exactly when the store holds at least one
  session with `session_meta.is_evicted` set: instead of `DROP TABLE`, delete the
  derived rows of the sessions that are about to be rebuilt - by turn id for
  `paths`, `entities`, `compaction_boundaries` and `turns_fts` (whose rowid IS
  `turns.id`), then the `turns` rows themselves, children first for the same
  enforced-foreign-key reason the existing drop order documents - and rebuild
  only those. The evicted sessions' rows are left standing. `derive::derive_turn`
  is already idempotent by delete-then-insert at a known rowid, which is what
  makes the two paths produce the same rows for every session that has a blob.
  Keep the existing drop-and-recreate path unchanged for a store with no evicted
  session, which is every store today, and keep the whole thing inside the one
  transaction and the `meta` version stamp it already commits with. Report the
  preserved sessions on `Rebuilt` as a count distinct from `failed` and say so in
  what `verbatim reindex` prints: rows carried across rather than rebuilt is a
  fact about the rebuild a reader is owed, and it is also the honest place to
  record that a future change to a derived table's SHAPE would reach the rebuilt
  sessions and not the preserved ones.
- **Verify:** `cargo test -p verbatim-core --test reindex` passes with new cases
  showing (a) after evicting one session and running `reindex`, that session's
  `turns`, `entities`, `paths` and `turns_fts` rows are all still present with
  the same ids, a `turns_fts` MATCH that returned one of its turns before still
  returns it, and it is not in `Rebuilt::failed`, (b) every other session's
  derived rows are identical before and after, compared through
  `testkit::query_set_json`, and (c) on a store with no evicted session the
  existing behaviour is unchanged, including that a corrupt blob still lands on
  `Rebuilt::failed` and no other session is lost.

### Task 3: `verbatim retention --dry-run`, reporting what the next pass will do

- **Files:** crates/verbatim/src/cmd/retention.rs, crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs, crates/verbatim/tests/retention.rs
- **Action:** Add a `retention` subcommand to the binary, declared in
  `cmd/mod.rs`'s module list and dispatched from `main.rs` beside `stats` and
  `replay`, with a line in the `USAGE` string. It opens through
  `cmd::read::open` the way `stats` and `observations` do - a read must not
  create a store as the side effect of a question - computes the cutoff
  timestamp ONCE, calls PLAN-1's single evaluation function with it, then prints
  that cutoff alongside the session ids it would evict and the ones it would
  delete, plus the leftover count when the per-pass bound truncates the work.
  The cutoff is in the document because the shared evaluation function pins the
  rule and not the instant: a pass running later re-evaluates `'now'` and a
  session sitting on the boundary can legitimately fall on the other side of it,
  so the report states the instant it judged against rather than implying the
  next pass will judge against the same one. It applies nothing, ever: a second application path would race the
  pass's own, which is precisely what one shared evaluation exists to prevent,
  so `--dry-run` is accepted as the documented spelling of the only behaviour
  this command has and every other flag except `--json` is misuse. Follow the
  flag-loop shape `search`, `show` and `sessions` use rather than
  `cmd::json_flag`, since there are two flags. Report through
  `cmd::json::Document` with the command name `retention`, every value built by
  `serde_json` and never with `format!`, and keep the stream split: the document
  on stdout, every diagnostic on stderr. A store that does not exist, or a
  policy that selects nothing, is an empty result with a reason and exit 0 -
  never non-zero - through `cmd::read::empty`.
- **Verify:** `cargo test -p verbatim --test retention` passes with new cases
  showing (a) `verbatim retention --dry-run --json` against a store with an
  evict policy and an aged corpus lists exactly the session ids an immediately
  following `verbatim ingest` then evicts - the fixture placing every session
  at least a day clear of the age boundary on either side, so the assertion
  cannot flake on a `'now'` that advanced between the two commands - and carries
  a `cutoff` field holding the timestamp it judged against, (b) running the dry run twice leaves
  `sessions`, `session_meta`, `turns`, `turns_fts`, `entities`, `paths` and
  `watermarks` row-for-row identical, compared through
  `testkit::archive_digest` and `testkit::query_set_json`, (c) after that pass,
  `verbatim sessions` still lists the evicted session, a search for a token that
  matched one of its turns still returns that turn, `verbatim show` on that turn
  reports the body as evicted, and a deleted session appears in neither the
  listing nor the search, and (d) against a machine with no store the command
  exits 0 with a reason and creates no data directory.

### Task 4: Hold `retention` to the swept `--json` contract and document its shape

- **Files:** crates/verbatim/tests/cli.rs, docs/json-shapes.md
- **Action:** Add `retention` to the `DATA_COMMANDS` table in
  `crates/verbatim/tests/cli.rs` with the arguments that make it answer and the
  `data` field names it emits, so the five swept properties - the envelope, the
  stream split, the exit codes, the empty-result rule and the documented shape -
  hold it without a new test file. Add its `data` section to
  `docs/json-shapes.md` under `## data by command`, transcribing the same field
  list, and state there what the report means: that it describes what the NEXT
  ingest pass would do under the current `verbatim.toml`, that the command never
  applies anything, and that an evicted session keeps its listing and its search
  hits while a deleted one is gone from both. The transcription is the point -
  the sweep fails when the shape drifts from the doc and when the doc is updated
  without the code.
- **Verify:** `cargo test -p verbatim --test cli` passes with `retention` in
  `DATA_COMMANDS`, including the property test that every documented `data` field
  is present in the emitted document and no undocumented one is.

## Notes

- Task 2 changes a documented invariant of `reindex` - "every table is dropped
  before a single row is read" - for the one case where the blob cannot answer.
  The doc comment at the top of `reindex.rs` should say so, naming that the
  preserved rows are an evicted session's only remaining index and that the
  alternative is losing them inside the ingest lock with a `Rebuilt::failed` note
  to show for it.
- PLAN-1 and PLAN-2 share `crates/verbatim-core/tests/retention.rs` and are
  SEQUENTIAL: PLAN-1 first.
