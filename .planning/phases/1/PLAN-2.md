---
phase: 1
plan: 2
requirements:
  - STOR-02
  - STOR-03
  - STOR-04
  - ING-01
  - ING-02
files:
  - Cargo.toml
  - Cargo.lock
  - crates/verbatim-core/Cargo.toml
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/src/parse/mod.rs
  - crates/verbatim-core/src/parse/record.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/src/ingest/lock.rs
  - crates/verbatim-core/src/derive.rs
  - crates/verbatim-core/src/verify.rs
  - crates/verbatim-core/src/reindex.rs
  - crates/verbatim-core/tests/parse.rs
  - crates/verbatim-core/tests/ingest.rs
  - crates/verbatim-core/tests/derive.rs
  - crates/verbatim-core/tests/verify.rs
  - crates/verbatim-core/tests/reindex.rs
  - crates/verbatim/Cargo.toml
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/ingest.rs
  - crates/verbatim/src/cmd/verify.rs
  - crates/verbatim/src/cmd/reindex.rs
  - crates/verbatim/tests/lock_race.rs
  - crates/verbatim/tests/crash.rs
---

# Phase 1: Archive Core - Plan 2 of 2 (ingest, integrity, rebuild)

**SEQUENTIAL: PLAN-1 must complete before this plan starts.** It shares
`Cargo.toml`, `Cargo.lock`, `crates/verbatim-core/Cargo.toml`,
`crates/verbatim-core/src/lib.rs`, `crates/verbatim-core/src/testkit.rs`,
`crates/verbatim-core/src/store/open.rs`, `crates/verbatim/Cargo.toml` and
`crates/verbatim/src/main.rs` with PLAN-1, and every task below builds on the
store, schema and blob layers PLAN-1 delivers. Do not run them in parallel.

## Goal

A store that can hold a session's turns losslessly, read one turn cheaply,
prove its own integrity, and rebuild everything derived from the blobs alone.
This plan wires the substrate into a working command: parse a transcript, tail
it from its watermark, commit everything in one transaction under an OS
try-lock, then prove the result with `verify` and rebuild it with `reindex`.

## Must be true when done

- `verbatim ingest <path.jsonl>` on a fixture stores the file's bytes verbatim:
  decompressing the session blob reproduces the source file byte for byte,
  including the eleven state record types that get no turn row (AC7).
- Reading one ingested turn decompresses only the blocks that turn occupies:
  one block for a turn that fits in one, N for a turn spanning N (AC1).
- A second `verbatim ingest` launched while the first holds `LOCK` exits 0 in
  under 50 ms over 10 consecutive runs and adds no row to any table (AC6).
- Killing ingest at 20 or more randomized points and reopening the store leaves
  no turn, FTS or entity row without a blob whose BLAKE3 matches, and no
  watermark past the last committed block boundary; rerunning converges to the
  contents of an uninterrupted pass (AC2).
- `verbatim verify` on a store with one byte flipped inside one session's blob
  exits non-zero, prints that session id, and prints no other session id (AC3).
- Dropping `turns`, `turns_fts`, `entities` and `paths` and running
  `verbatim reindex` produces byte-identical JSON for a fixed query set,
  compared before and after (AC4).

## Context

PLAN-1 delivered the workspace, the fixtures under `tests/fixtures/`, store
open with the D-09 version gate, the schema, and the blob writer, reader and
append path. This plan implements the remaining locked decisions: D-02, D-03,
D-11 (population), D-12 (the parser half), D-13, D-14, D-15, D-16, D-18, D-20,
and completes STOR-05's older-format branch. Out of scope for the phase: tree
discovery, subagent walking, project identity from `cwd`, exclusions,
compaction boundaries, `verbatim status` (phase 2); Rust-side text expansion,
entity extraction, search, `--json` on data commands, MCP (phase 3); hooks,
install, backfill (phase 4). Phase-1 CLI contract is only what the acceptance
criteria need: exit 0 on success, 1 on operational failure, 2 on misuse, data
to stdout, errors to stderr.

## Tasks

### Task 1: Parse a transcript byte range into records and a resume offset

- **Files:** crates/verbatim-core/src/parse/mod.rs,
  crates/verbatim-core/src/parse/record.rs, crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/tests/parse.rs
- **Action:** Scan a byte buffer of one transcript starting at a given offset,
  yielding one record per line terminated by `\n` and reporting the offset just
  past the last `\n` as the resume point, never end-of-file (D-14: all 1,221
  top-level and 812 sidecar transcripts end with `\n`, and a trailing partial
  line is a record still being written). Split on `\n` only and do not trim the
  record bytes, so what goes into the blob stays byte-identical to the source.
  Classify a record as a turn only when it carries both `uuid` and `timestamp`
  and its type is one of `user`, `assistant`, `attachment`, `system` (D-03);
  the other eleven types yield no turn and no derived row while remaining part
  of the byte stream (D-13). Assign `turn_seq` from byte order within the file,
  never by sorting on `timestamp` (D-02: 31 of 62 sampled transcripts contain
  an out-of-order timestamp, and a rebuild would reproduce the same wrong order
  on both sides so AC4 would still pass while neighbours are scrambled). A line
  that is not valid JSON, or is valid JSON without turn identity, is tolerated
  and never aborts the scan (D-12: `journal.jsonl` files sitting under
  `subagents/workflows/wf_*/` hold `{agentId, key, result, type}` records and
  recursive discovery in phase 2 will reach them). Extract per record what
  later tasks need and no more: `uuid`, `parentUuid`, type, `timestamp`, tool
  name when the record carries one, `sessionId`, the foreign `session_id` field
  that is D-11's file-level lineage signal, `cwd` and `gitBranch`.
- **Verify:** `cargo test -p verbatim-core --features testkit --test parse`
  passes, where the tests assert: `session-basic.jsonl` yields turn records for
  exactly the four turn types and none for the eleven state types, and the two
  records whose timestamps decrease keep their file order in `turn_seq`;
  `journal.jsonl` yields zero turns, zero records classified as turns, and no
  error; the resume offset for `session-truncated.jsonl` is the byte just past
  its last `\n` and is strictly less than the file length; truncating
  `session-basic.jsonl` at 200 pseudorandom offsets from a printed seed always
  yields a resume offset that lands immediately after a `\n` at or before the
  truncation point, and the records parsed are a prefix of the full file's
  parse.

### Task 2: Take the ingest lock with an immediate-fail OS try-lock

- **Files:** crates/verbatim-core/src/ingest/lock.rs,
  crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/Cargo.toml, Cargo.toml, Cargo.lock,
  crates/verbatim/Cargo.toml, crates/verbatim/tests/lock_race.rs
- **Action:** Add a dedicated `LOCK` file in the data directory, created by the
  process itself, taken with an exclusive OS try-lock that fails immediately:
  `flock(2)` with `LOCK_NB` on Unix, `LockFileEx` with
  `LOCKFILE_FAIL_IMMEDIATELY` on Windows (D-15). Never use `BEGIN EXCLUSIVE`
  and never hold a SQLite write transaction open for the duration of a pass:
  the second process would block on the busy handler and blow the 50 ms budget,
  and a long-held write transaction stops MCP readers, which is the reason redb
  was rejected (DESIGN-BRIEF.md:83). Acquire the lock before opening the store,
  so a contended run costs one file open and one lock syscall and touches no
  database. Lock held means exit 0 immediately with nothing written; the lock
  is released on drop and by process death, which is why an OS lock is used
  rather than a PID file (DESIGN-BRIEF.md:135). D-20 records that this holds on
  every target: APFS honors `flock` with `LOCK_NB`, the Windows path is
  filesystem-agnostic byte-range locking, and the overlayfs hazard cannot arise
  for a file the process creates in its own data directory. Any dependency used
  for the platform locking must be permissively licensed
  (DESIGN-BRIEF.md:512). Wire the guard into the ingest entry point so PLAN-2
  task 3's command inherits it.
- **Verify:** `cargo build --release` then `cargo test -p verbatim --release
  --test lock_race` passes, where the test holds `LOCK` through the library API
  in the test process, spawns the release `verbatim ingest
  tests/fixtures/session-basic.jsonl` binary 10 consecutive times against the
  same temp data dir, and asserts every run exits 0 with wall time under 50 ms
  and that the row counts of `sessions`, `session_meta`, `turns`, `turns_fts`,
  `entities`, `paths`, `watermarks` and `runs` are unchanged across all ten
  (AC6).

### Task 3: Ingest one named transcript in a single transaction

- **Files:** crates/verbatim-core/src/ingest/mod.rs,
  crates/verbatim/src/main.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/cmd/ingest.rs, crates/verbatim-core/tests/ingest.rs
- **Action:** Add `verbatim ingest <path.jsonl>` taking one explicit file
  (D-18: the no-arg tree walk is phase 2, and a single-file interface keeps the
  crash and lock-race tests on a small fixture instead of the real 962 MB
  tree). Canonicalize the argument before anything else, resolving symlinks:
  `~/.claude` is a symlink chain on the development machine, so the same
  transcript reached by two paths would otherwise get two archive rows
  (DESIGN-BRIEF.md:406 and CONTEXT's third flagged assumption; this plan
  resolves that assumption as "canonicalize"). Key the session on the
  transcript file identity, the canonical path, not on the record's
  `session_id` (D-01), and key the watermark on the same value. Read from the
  stored watermark to the resume offset task 1 computes, append exactly those
  bytes to the session blob through PLAN-1's append path, insert a turn row per
  turn record with its offset and length in the uncompressed stream, populate
  `session_meta` including `continues_from` from the record's foreign
  `session_id` field with `parentUuid` as the fallback only when a continuation
  file carries none (D-11), recompute the BLAKE3 over the full uncompressed
  session bytes (D-06), advance the watermark, and write one `runs` row for the
  pass. Every one of those writes commits in a single transaction (STOR-02), so
  a killed process leaves no partially indexed session; nothing outside that
  transaction may mutate the store. Recording a *failed* pass in `runs`
  requires a second transaction after a rollback and belongs to phase 2
  (ING-03, ING-09): phase 1 writes the row only for a pass that commits. A
  rerun with no new bytes is a no-op that adds no row.
- **Verify:** `cargo test -p verbatim-core --features testkit --test ingest`
  passes, where the tests assert: after ingesting `session-basic.jsonl`, the
  decompressed blob equals the source file byte for byte and contains every one
  of the fifteen record types (AC7), and the watermark equals the file length;
  after ingesting `session-large-record.jsonl`, reading a turn whose record is
  under 64 KB reports exactly 1 block decompressed and reading the 200 KB
  record's turn reports exactly 4 (AC1); re-running ingest on an unchanged file
  adds no row to any table; ingesting `session-truncated.jsonl`, then appending
  the remainder of that cut line plus the rest of `session-basic.jsonl` to it
  and ingesting again, produces a store whose blob, turn rows and watermark are
  identical to ingesting the complete file in one pass (ING-01).

### Task 4: Derive FTS rows through the single seam ingest and reindex share

- **Files:** crates/verbatim-core/src/derive.rs,
  crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/tests/derive.rs
- **Action:** Put the turn-to-derived-rows mapping behind one function that
  both the ingest path and the rebuild path call, so a rebuild cannot drift
  from what ingest wrote. In this phase the function inserts one `turns_fts`
  row per turn at rowid `turns.id` (D-10), carrying the turn's raw record line
  as UTF-8 text with no expansion, and emits no `entities` and no `paths` rows.
  This is the phase boundary, not a reduction: RCL-01's camel, snake, kebab and
  path expansion and RCL-02's entity extraction are phase 3 and replace the
  body of this one function; phase 1 owns the tables, the seam and the rebuild
  path, not what fills them. Deleting and reinserting the FTS row at a known
  rowid is what `contentless_delete=1` exists for, so re-deriving a turn is
  idempotent by construction rather than by a uniqueness check. The seam runs
  inside task 3's transaction; it must not open its own.
- **Verify:** `cargo test -p verbatim-core --features testkit --test derive`
  passes, where the tests assert after ingesting `session-basic.jsonl`: an FTS
  `MATCH` for a token that appears in exactly one fixture turn returns exactly
  that turn's `turns.id`; re-deriving the same turn leaves the `turns_fts` row
  count unchanged and the same rowid matching; the `entities` and `paths`
  tables have zero rows; and calling the seam directly with a parsed turn
  produces rows equal to those ingest wrote for that turn.

### Task 5: Add `verbatim verify` to walk blob checksums

- **Files:** crates/verbatim-core/src/verify.rs, crates/verbatim/src/cmd/verify.rs,
  crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs,
  crates/verbatim-core/tests/verify.rs
- **Action:** Walk every row in `sessions`, decompress its blob, compute BLAKE3
  over the uncompressed bytes and compare with the checksum `session_meta`
  holds (D-06). Print the session id of each failure and exit non-zero when any
  fail, exit 0 when none: corruption localizes to named sessions rather than
  "the store is gone" (DESIGN-BRIEF.md:98). A blob that fails to decompress
  counts as a failure for that session and must not abort the walk, so a second
  corrupt session is still reported. Do not run `PRAGMA integrity_check` here:
  D-16 assigns it to `doctor` (INST-06, phase 4), because a page-level
  complaint carries no session attribution and would ride alongside the named
  session, breaking AC3's "and no other" on a store that is otherwise fine.
- **Verify:** `cargo test -p verbatim-core --features testkit --test verify`
  passes, where the tests ingest three fixture transcripts and assert: with no
  corruption `verbatim verify` exits 0; after flipping one byte inside one
  session's blob column with a direct SQLite `UPDATE`, it exits non-zero,
  stdout contains that session id, and stdout contains no other ingested
  session id; after corrupting two sessions, both ids appear; the output never
  contains the string `integrity_check`.

### Task 6: Add `verbatim reindex` to rebuild derived tables from blobs alone

- **Files:** crates/verbatim-core/src/reindex.rs,
  crates/verbatim/src/cmd/reindex.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs, crates/verbatim-core/src/store/open.rs,
  crates/verbatim-core/src/testkit.rs, crates/verbatim-core/tests/reindex.rs
- **Action:** Drop and recreate `turns`, `turns_fts`, `entities` and `paths`,
  then rebuild every row by decompressing each blob and re-running task 1's
  parser and task 4's seam over the resulting bytes, reading nothing from the
  dropped tables (STOR-04: the blobs are the only input). `sessions` and
  `session_meta` are never touched, because the archive table never migrates
  and STOR-05's older-format path rebuilds derived tables only. Turn ids must
  come out identical, which is why PLAN-1 task 4 derived them from the
  session's stable integer key and `turn_seq` (D-10): a renumbering would let
  AC4's fixed query set return the same row count while pointing at different
  turns. Run the rebuild in one transaction. Then wire the older-format branch
  PLAN-1 task 3 left returning a rebuild-required outcome: opening a store
  whose `derived_schema` or `archive_format` is lower than the binary's runs
  this rebuild and updates `meta` afterwards. Add the fixed query set to
  `testkit.rs`: a handful of FTS `MATCH` queries, a turn range for one session
  and a turn-id lookup, serialized to JSON in a stable order so two runs are
  byte-comparable.
- **Verify:** `cargo test -p verbatim-core --features testkit --test reindex`
  passes, where the tests ingest the fixtures and assert: the fixed query set's
  JSON is byte-identical before and after dropping the four derived tables and
  running `verbatim reindex` (AC4); a digest over all `sessions` and
  `session_meta` rows is unchanged across the rebuild; opening a store whose
  `meta.derived_schema` was set one lower than the binary's leaves the derived
  tables repopulated, the archive tables untouched, and `meta.derived_schema`
  equal to the binary's value.

### Task 7: Prove crash atomicity with a randomized kill harness

- **Files:** crates/verbatim/tests/crash.rs, crates/verbatim-core/src/testkit.rs
- **Action:** Build the harness AC2 names. It runs `verbatim ingest` against a
  fixture in a temp data dir and kills the child un-catchably (`SIGKILL` on
  Unix, `TerminateProcess` on Windows) at 20 or more randomized points, spread
  across the pass by both elapsed time and bytes written, driven by a printed
  seed so a failure reproduces. After each kill it reopens the store and
  asserts the invariants: every `turns`, `turns_fts`, `entities` and `paths`
  row belongs to a session whose blob decompresses and whose BLAKE3 matches
  `session_meta`; no watermark is ahead of the bytes the committed blob
  represents; and re-running ingest to completion converges to a store whose
  blob bytes, turn rows, FTS rowids and watermark match an uninterrupted single
  pass over the same file. This is the check that the single transaction in
  task 3 is real rather than intended, so it must be able to fail: confirm once
  that moving the watermark update out of the ingest transaction makes the
  harness fail, then revert that change.
- **Verify:** `cargo build --release` then `cargo test -p verbatim --release
  --test crash` passes with at least 20 kill points and its seed printed, and
  the recorded one-off check shows the harness failing when the watermark
  update is committed separately from the blob (AC2).

## Notes

- **Plan shape.** CONTEXT directs "multiple plans, same phase". This phase is
  two plans, but they are SEQUENTIAL rather than parallel: the slices share
  eight declared paths, listed at the top of this file. The split is driven by
  capacity (14 tasks against an 8-task ceiling), not by independence.
- **Requirement split.** STOR-01 is carried by PLAN-1 tasks 5-7 and completed
  end-to-end by task 3's AC1 clause here. STOR-05 is carried by PLAN-1 task 3
  (newer format refused) and completed by task 6 here (older format rebuilds
  derived tables only).
- **Cost surfaced during planning.** Task 3 recomputes BLAKE3 over the full
  uncompressed session bytes on every pass, which means decompressing the blob
  each time a live session grows: roughly 1-2 ms at the p90 session size of
  1.0 MB and around 15 ms at the observed 10.1 MB maximum. D-06 fixes the
  checksum as BLAKE3 over the uncompressed session bytes, and BLAKE3 exposes no
  resumable state, so an incremental alternative would change the stored
  semantic. Acceptable for a detached ingest; flagged in case phase 2's tree
  walk over 1,896 files makes it visible.
- **Phase-1 CLI scope.** `--json` on data commands, stable output shapes and
  the full exit-code contract are RCL-06 in phase 3. This phase adds only
  `ingest`, `verify` and `reindex` with exit 0/1/2 and the stdout/stderr split,
  because AC3, AC5 and AC6 assert on them.
