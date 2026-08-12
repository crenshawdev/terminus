---
phase: 2
plan: 3
requirements:
  - ING-03
  - ING-04
  - ING-06
files:
  - crates/verbatim-core/src/parse/record.rs
  - crates/verbatim-core/src/parse/mod.rs
  - crates/verbatim-core/src/derive.rs
  - crates/verbatim-core/src/verify.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/src/ingest/pass.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/parse.rs
  - crates/verbatim-core/tests/derive.rs
  - crates/verbatim-core/tests/compaction.rs
  - crates/verbatim-core/tests/verify.rs
  - crates/verbatim-core/tests/fixtures.rs
  - crates/verbatim/tests/crash.rs
  - crates/verbatim/tests/corpus.rs
  - tests/fixtures/README.md
  - tests/fixtures/session-compacted.jsonl
---

# Phase 2: Ingest At Scale - Plan 3 of 3 (compaction, damage, convergence)

**SEQUENTIAL: PLAN-1 and PLAN-2 must both complete before this plan starts.**
All three write `crates/verbatim-core/src/ingest/mod.rs`,
`crates/verbatim-core/src/ingest/pass.rs`,
`crates/verbatim-core/src/testkit.rs`,
`crates/verbatim-core/tests/fixtures.rs` and `tests/fixtures/README.md`. This
plan's crash and corpus harnesses exercise the pass PLAN-1 built and the
identity PLAN-2 fills, so it cannot start earlier. Do not run in parallel.

## Goal

The tree pass holds up against the two things a real corpus does to it: a live
transcript that grows a compaction boundary underneath an already-archived
session, and a process that dies mid-pass. Plus the proof that the whole thing
runs against the real 1,896-file tree.

## Must be true when done

- Appending a `compact_boundary` record to an already-ingested transcript and
  re-running ingest adds a boundary row carrying that record's `compactMetadata`
  bytes verbatim, and leaves every turn row and every committed blob block from
  the earlier pass byte-identical.
- A transcript on disk that is shorter than its stored watermark is skipped with
  its error recorded and is never re-ingested from offset 0, and
  `verbatim verify` names that session's divergence.
- Killing a pass at ten or more randomized points and re-running to completion
  yields the same blob checksums, turn rows and watermarks as one uninterrupted
  pass over the same tree - including when the kills land on an append pass over
  a tree that was already ingested.
- The crash harness's watermark invariant states which property it asserts and
  fails when a watermark lands anywhere but a record boundary inside the
  committed blob.
- A pass over the real corpus tree archives every `<uuid>.jsonl` and every
  `agent-*.jsonl` beneath the root at any depth, archives no `journal.jsonl`,
  `.meta.json` or `tool-results/` file, and reports zero unparseable lines and
  zero panics.

## Context

Locked and binding here: D-21 (a compaction boundary is a `type: "system"`
record with `subtype: "compact_boundary"`, which phase 1's D-03 already
classifies as a turn, so ING-06 adds a derived boundary row rather than a new
record class), D-08 (the `compactMetadata` is stored verbatim and no
dropped-turn set is computed in this phase - the complement is phase 5's
INJ-05), D-13 (a transcript shorter than its stored watermark is skipped with
its error recorded, never re-ingested from offset 0, and the session is flagged
so `verbatim verify` reports the divergence), D-11 (per-file transactions and
per-file watermarks are what make a killed pass converge), D-25 (no closed
record-type set is asserted against the real corpus).

The boundary table and the divergence column both exist after PLAN-1 task 1;
this plan fills them.

Existing code this plan changes: `Record::parse` in
`crates/verbatim-core/src/parse/record.rs`, which extracts `type`, `sessionId`,
`session_id`, `cwd`, `gitBranch`, `uuid`, `timestamp`, `parentUuid` and the
first `tool_use` name and keeps a non-JSON line as a record with no fields;
`derive::derive_turn` in `crates/verbatim-core/src/derive.rs`, the one seam
ingest and `reindex` share, which writes the `turns` row and the `turns_fts`
row and clears `entities` and `paths` for the turn; `read_tail` in
`crates/verbatim-core/src/ingest/mod.rs`, which returns an IO error naming the
file length and the watermark when the file is shorter; `verify::walk` and
`verify::check` in `crates/verbatim-core/src/verify.rs`, which report per
session by `session_key`; and `crates/verbatim/tests/crash.rs`, whose
`Snapshot`, `check_invariants`, `kill_at`, `kill_after` and
`the_harness_catches_a_watermark_committed_outside_the_transaction` this plan
extends rather than replaces.

Test invocation note: `cargo test -p verbatim-core` runs none of the
`#![cfg(feature = "testkit")]` test files and exits 0 (phase 1 open item), so
every Verify below uses `cargo test --workspace`.

## Tasks

### Task 1: The parser sees a record's subtype and its compaction metadata

- **Files:** crates/verbatim-core/src/parse/record.rs, crates/verbatim-core/src/parse/mod.rs, crates/verbatim-core/src/testkit.rs, crates/verbatim-core/tests/parse.rs, crates/verbatim-core/tests/fixtures.rs, tests/fixtures/README.md, tests/fixtures/session-compacted.jsonl
- **Action:** Extend `Record::parse` in
  `crates/verbatim-core/src/parse/record.rs` to carry two more things off a
  line: the record's `subtype` when it has one, and the raw bytes of its
  `compactMetadata` object when it has one. Raw bytes, not a parsed structure
  (D-08): the upstream semantics of `preservedMessages.uuids` versus `allUuids`
  versus `preservedSegment` are unsettled, exactly one `compact_boundary` exists
  in 300,556 measured records, and its own token counts contradict
  `DESIGN-BRIEF.md:140`'s reading of it, so storing the bytes is what makes a
  wrong reading fixable in phase 5 without a reingest. Add no new record class
  and change no classification rule: `system` is already in
  `parse::TURN_TYPES`, and the real `compact_boundary` record carries both
  `uuid` and `timestamp`, so D-03 already makes it a turn (D-21). A record with
  no `subtype` and no `compactMetadata` must be unchanged in every field, since
  every phase 1 assertion in `crates/verbatim-core/tests/parse.rs` and
  `crates/verbatim-core/tests/ingest.rs` compares whole `Record` values. Add
  `tests/fixtures/session-compacted.jsonl` holding a short session that ends
  with a real-shaped boundary: a `system` record with
  `subtype: "compact_boundary"`, a `uuid`, a `timestamp`, a
  `logicalParentUuid`, and a `compactMetadata` object carrying `preTokens`,
  `postTokens`, `cumulativeDroppedTokens` and a `preservedMessages` object whose
  `uuids` list is a proper subset of its `allUuids` list - that subset relation
  is the measured fact D-08 rests on. Register it in
  `testkit::TRANSCRIPT_FIXTURES`, document it in `tests/fixtures/README.md`, end
  it with a newline, and keep `testkit::UNIQUE_TOKEN` out of its bytes so
  `unique_token_appears_exactly_once_across_the_corpus` still holds. Do not add
  the boundary record to `session-basic.jsonl`:
  `session_basic_carries_all_fifteen_record_types` asserts an exact set on that
  file.
- **Verify:** `cargo test --workspace --test parse --test fixtures --test
  ingest` passes, including: parsing the boundary line yields a record whose
  subtype is `compact_boundary`, whose compaction-metadata bytes deserialize to
  an object whose `preservedMessages.uuids` is a subset of its `allUuids`, and
  which is classified as a turn; parsing any line of `session-basic.jsonl`
  yields no subtype and no compaction metadata; and the fixture's bytes parse as
  JSONL with no failures.

### Task 2: A compaction boundary becomes a row, through the derive seam

- **Files:** crates/verbatim-core/src/derive.rs, crates/verbatim-core/tests/derive.rs, crates/verbatim-core/tests/compaction.rs
- **Action:** Write the boundary row inside `derive::derive_turn`, the one seam
  ingest and `reindex` share, and nowhere else - a second write site is exactly
  what STOR-04 forbids, and the module's own contract is that the seam owns
  every derived row for a turn. When the turn's record carries
  `subtype: "compact_boundary"`, insert one row into the boundary table PLAN-1
  added, keyed on the same `turns.id`, holding that record's `compactMetadata`
  bytes verbatim. When it does not, clear any boundary row at that turn id the
  same way `derive_turn` already deletes from `entities` and `paths` before
  writing, so re-deriving a turn is idempotent by construction. Compute no
  dropped-turn set and interpret nothing: the complement of the preserved uuids
  is derived at query time in phase 5 (INJ-05, D-08). `derive_turn` runs inside
  the caller's transaction and must keep opening none of its own. The boundary
  row is derived, so a `reindex` that drops and rebuilds the derived tables must
  reproduce it from the blob alone with no help from the old rows.
- **Verify:** `cargo test --workspace --test compaction --test derive --test
  reindex --test cli` passes, including AC4 in full: ingest a transcript, record
  every `turns` row and the exact bytes of every committed blob block, append
  the boundary line from `tests/fixtures/session-compacted.jsonl` to that file,
  re-run ingest, and find one boundary row whose stored metadata bytes equal the
  `compactMetadata` bytes in the appended line, every pre-existing `turns` row
  unchanged, and every completed blob block from the first pass byte-identical;
  and a `reindex` after dropping the derived tables reproduces the boundary row
  with the same turn id and the same bytes.

### Task 3: A transcript shorter than its watermark is skipped and flagged

- **Files:** crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/ingest/pass.rs, crates/verbatim-core/src/verify.rs, crates/verbatim-core/tests/verify.rs, crates/verbatim-core/tests/compaction.rs
- **Action:** Complete D-13. `read_tail` in
  `crates/verbatim-core/src/ingest/mod.rs` already refuses a file shorter than
  its stored watermark rather than guessing which bytes are still ours; keep
  that refusal and never fall back to re-ingesting from offset 0, because the
  append-only property is an observation over one 25-day corpus
  (`DESIGN-BRIEF.md:139`) and not a documented guarantee, and re-ingesting would
  destroy archived bytes. What this task adds is the signal: the pass records
  the skip against that file (PLAN-1 task 4 already isolates it) and sets the
  divergence column PLAN-1 added on that session's `session_meta` row, in a
  transaction of its own since the file's own pass rolled back. Then make
  `verify` report it: `verify::walk` today reads `session_key`, `blob` and
  `checksum` and `verify::check` fails only on a missing metadata row, a blob
  that will not decompress, or a checksum mismatch. Add the divergence as a
  further per-session failure with a message saying the transcript on disk is
  shorter than the bytes the archive holds and that the archive was left
  untouched. This is a new signal path this phase adds - D-16 from phase 1
  scopes `verify` to checksums - so it must not disturb AC3: the failure is
  attributed to exactly the diverged session by `session_key` and names no
  other, and a clean store still prints nothing. Clear the flag when a later
  pass over that transcript succeeds, so a file that was mid-write stops being
  reported once it is whole again.
- **Verify:** `cargo test --workspace --test verify --test compaction --test
  pass --test cli` passes, including: ingest a transcript, truncate the file on
  disk below its watermark, run a pass, and find the session's blob, checksum
  and turn rows unchanged, its watermark unchanged, the divergence flag set, the
  pass's `runs` error naming that path, and `verbatim verify` exiting non-zero
  naming that session and no other; then restore the file to its full length,
  re-run the pass, and find the flag cleared and `verbatim verify` exiting 0
  with an empty stdout.

### Task 4: Killing a pass mid-tree converges, on a first pass and on an append

- **Files:** crates/verbatim-core/src/ingest/pass.rs, crates/verbatim/tests/crash.rs
- **Action:** Extend phase 1's harness in `crates/verbatim/tests/crash.rs` from
  one transcript to a tree, and from a first pass to an append pass. Add a
  pass-level fault point beside the existing ones in `ingest::fault` - the
  module is compiled only under the `testkit` feature and the shipped binary
  takes no branch - that stalls after a given number of files have committed, so
  a kill lands deterministically in the middle of a walk rather than only where
  a timed race happens to fall; the existing per-file points still fire inside
  whichever file is in flight. Build a tree of several transcripts of different
  sizes including a sidecar, take an uninterrupted pass as the reference, then
  kill at ten or more points spread across aimed and timed, re-run each killed
  store to completion, and require blob checksums, turn rows and watermarks to
  equal the reference (AC6). Then do the same again on the case phase 1 never
  covered and flagged: ingest the tree fully, append fresh records to several of
  the transcripts, and kill the *append* pass at the same spread - the phase 1
  harness only ever killed a first pass into a fresh data directory, so the
  resume path has never been killed. Keep `runs` out of the compared snapshot
  for the reason already documented there: an interrupted store reaches the same
  contents in more passes. And make the watermark invariant honest, which is the
  other phase 1 open item: `check_invariants` today only rejects a watermark
  past the committed byte count, the weaker of the two properties. Assert the
  stronger one - a watermark must land at a record boundary inside the committed
  blob, so the byte before it in the decompressed stream is a newline - and say
  in the function's documentation which property each check is. Keep
  `the_harness_catches_a_watermark_committed_outside_the_transaction` running
  and extend it so the negative confirms the stronger check can fail too.
- **Verify:** `cargo test --workspace --test crash` passes with the seed it
  prints, reporting at least ten kills against a first pass and at least ten
  against an append pass, each followed by a convergence assertion against the
  uninterrupted reference; and the negative test still fails the invariant check
  when the watermark commits outside the pass transaction, now also when a
  watermark is moved to a non-boundary offset inside the committed blob.

### Task 5: A pass over the real corpus, measured

- **Files:** crates/verbatim/tests/corpus.rs, crates/verbatim-core/src/testkit.rs
- **Action:** Add an environment-gated integration test that runs one real pass
  over the private corpus - PROJECT.md's constraint is a private real-corpus
  fixture reached by env var, and no such var exists yet, so define one naming a
  Claude config directory whose `projects` subdirectory is the tree, and skip
  with a printed reason when it is unset so CI stays green without it. The test
  writes its store into a temporary data directory and never into the user's,
  and it only reads the corpus. Assert AC1: the archived session count equals
  the number of `<uuid>.jsonl` files at project depth plus the number of
  `agent-*.jsonl` files at any depth, counted independently by the test walking
  the tree itself rather than by asking the code under test; no session exists
  for any `journal.jsonl`, any `.meta.json`, any `workflows/wf_*.json` or
  anything under a `tool-results/` directory; and zero unparseable lines - the
  test decompresses each archived blob and parses every line with `serde_json`,
  counting failures, which is a check the product code cannot do for it because
  `Record::parse` deliberately keeps a non-JSON line as a record rather than
  erroring. Do not assert a closed record-type set (D-25): 16 distinct top-level
  types were measured including `pr-link` and `frame-link`, the set grows
  upstream, and an exact assertion fails on data that is correct - count the
  types and print them instead. Assert AC2 on the same store: the count of
  non-null `continues_from` values is greater than zero and the count of rows
  whose `continues_from` equals their own `session_id` is zero. Print the wall
  time of the pass and of the derived rebuild that a bumped `DERIVED_SCHEMA`
  triggers on the first open of an existing store, since D-24 names that as the
  scale consequence to plan around: on the real corpus that is roughly 2,071
  sessions rebuilt from 988 MB of blobs inside the ingest lock before the walk
  begins. Assert no panic by the test completing, and assert the pass exits 0.
- **Verify:** With the corpus env var set to the real Claude config directory,
  `cargo test --workspace --test corpus -- --nocapture` passes and prints the
  session count, the independently counted file totals it matched, the
  per-record-type histogram, the `continues_from` counts, and the pass and
  rebuild wall times. With the variable unset, the same command passes and
  prints the skip reason. (human-verify: the corpus run is the one check that
  needs the private tree; on the development machine the directory is
  `/data/claude/.claude`, which currently holds 2,077 `.jsonl` files, 1,254 at
  project depth, 819 `agent-*.jsonl` and 817 `agent-*.meta.json`.)

## Notes

- ING-07 stays deferred to phase 8 and appears in no task in any of the three
  plans; see PLAN-1's Notes for the reason and for the `.planning/ROADMAP.md`
  edit still owed.
- Task 5's counts will not match CONTEXT's exactly and should not be pinned to
  literals: CONTEXT measured 2,075 `.jsonl` files, 1,253 at project depth and
  818 sidecars, and the live tree has grown since. The assertion is that the
  store's session count equals what the test itself counted on the same tree in
  the same run, which is the only form that stays true as the corpus grows.
- Task 3 changes what `verbatim verify` can report. AC3 from phase 1 stays
  satisfiable because the new failure is attributed by `session_key` like every
  other, but the phase 1 test
  `verify_exits_zero_and_says_nothing_on_a_clean_store` is the control that
  proves the new signal does not fire spuriously - it must keep passing
  untouched.
- The two phase 1 open items this phase was asked to carry are both closed by
  task 4: the crash harness now kills an append/resume pass, and its watermark
  invariant asserts the stronger of the two properties.
