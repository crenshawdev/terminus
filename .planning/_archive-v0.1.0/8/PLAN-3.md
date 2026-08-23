---
phase: 8
plan: 3
requirements: [RET-04, RET-05, PRIV-04]
files:
  - crates/verbatim/src/cmd/compact.rs
  - crates/verbatim/src/cmd/usage.rs
  - crates/verbatim/src/cmd/export.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/main.rs
  - crates/verbatim/tests/cli.rs
  - crates/verbatim/tests/lifecycle.rs
  - docs/json-shapes.md
---

# Phase 8: Retention And Lifecycle - Plan 3 (shrink it, measure it, take it with you)

## Goal

A store that actually gets smaller after retention has run, that can say where
its bytes went by project, by month and by table, and that can be written out in
a portable form that states what it contains.

## Must be true when done

- After a pass that deleted sessions, `verbatim compact` leaves the footprint
  `verbatim status` reports - `verbatim.db` plus its `-wal` and `-shm` sidecars -
  strictly smaller than it was before compaction ran.
- `verbatim compact` refuses rather than proceeds while another process holds
  the ingest lock, and says so.
- `verbatim usage --json` reports archive bytes by project and by month whose
  totals each equal `sum(length(sessions.blob))`, and a store-footprint table
  whose rows sum to the database file size.
- Every session with no project and every session with no `first_turn_at` lands
  in a named bucket rather than vanishing out of a total.
- `verbatim export <path>` writes the archived sessions in a portable form and
  states, on the terminal and in a manifest beside the output, exactly what it
  contains - including that it is the verbatim, unredacted transcripts.
- All three commands keep the `--json` envelope, the stream split and the 0/1/2
  exit codes, and all three are held to it by `DATA_COMMANDS`.

## Context

- D-08 fixes what `compact` is: `VACUUM` followed by
  `PRAGMA wal_checkpoint(TRUNCATE)`, taking the ingest lock the way
  `crates/verbatim/src/cmd/uninstall.rs` (`run`, the `--purge` arm) does.
  Measured: after a bare `VACUUM` the db halved while the WAL grew to 7.58 MB,
  a TOTAL up from 15.06 to 15.13 MB - and `cmd::status`'s `size_bytes` sums all
  three files, so without the checkpoint AC4 fails as written.
- D-09 fixes what `usage` is: TWO independent tables, archive bytes by project
  and by month, and store footprint by table, with the archive totals
  reconciling to `sum(length(sessions.blob))` and never to the on-disk file
  size. On the live store those are 465,985,120 and 1,103,396,864 bytes.
- D-18 fixes the buckets: one month per session by `substr(first_turn_at, 1, 7)`,
  with explicit buckets for the null-`first_turn_at` and null-`project` rows.
- D-17 puts all three in `DATA_COMMANDS` and `docs/json-shapes.md`.
- PRIV-04's shape is planner discretion - no phase-8 decision and no acceptance
  criterion constrains it. The choice recorded in task 3 is one JSONL file per
  session plus a manifest.
- Out of scope: snapshots and relocation (PLAN-4).

## Tasks

### Task 1: `verbatim compact`, which makes the store actually smaller

- **Files:** crates/verbatim/src/cmd/compact.rs, crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs, crates/verbatim/tests/lifecycle.rs
- **Action:** Add a `compact` subcommand, declared in `cmd/mod.rs` and
  dispatched from `main.rs` with a `USAGE` line, taking `--json` and nothing
  else through `cmd::json_flag`. It takes the ingest lock through
  `verbatim_core::ingest::lock::try_acquire` before opening anything, exactly as
  the `--purge` arm of `cmd::uninstall` does and for the same reason a plain
  `VACUUM` fails outright while another connection holds a write transaction; a
  held lock is an operational failure that names it, writes the envelope with
  `ok` false and exits 1, having changed nothing. It then opens the store,
  measures the footprint the same way `cmd::status::size_bytes` does - `verbatim.db`
  plus `-wal` plus `-shm` - runs `VACUUM`, runs `PRAGMA wal_checkpoint(TRUNCATE)`,
  and measures again. `VACUUM` cannot run inside a transaction, so it is issued
  on the connection directly and not through one. The checkpoint is not optional
  and not tidiness: a bare `VACUUM` on a WAL store moves the freed pages into the
  WAL, so the number this product itself reports goes UP immediately after
  compaction unless the WAL is truncated. Report the before and after footprints
  and the bytes reclaimed, and print them in the human mode too. Nothing here
  deletes anything: `compact` reclaims space retention already freed and has no
  opinion about what should be kept.
- **Verify:** `cargo test -p verbatim --test lifecycle` passes with new cases
  showing (a) on a store where a large share of the sessions have been deleted,
  `verbatim status --json`'s `size_bytes` after `verbatim compact` is strictly
  less than before it, (b) `verbatim compact --json` reports a `-wal` component
  of zero immediately after it returns, (c) with the ingest lock held by another
  handle the command exits 1, says an ingest is running, and leaves the
  footprint unchanged, and (d) `testkit::archive_digest` over the store is
  identical before and after a compaction.

### Task 2: `verbatim usage`, two tables that each reconcile

- **Files:** crates/verbatim/src/cmd/usage.rs, crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs, crates/verbatim/tests/lifecycle.rs
- **Action:** Add a `usage` subcommand on the same footing as task 1's, opening
  through `cmd::read::open` because it is a read and must not create a store.
  It emits two independent tables and never mixes them. The ARCHIVE table is
  `length(sessions.blob)` grouped by project and, separately, by month, and its
  totals reconcile to `sum(length(sessions.blob))` and to nothing else - never
  to the on-disk file size, which on the live store is 2.4x larger than
  everything the projects account for, and never to a pro-rata share of the
  derived tables, which would make every per-project number an estimate. The
  project side goes through `config::visible::sessions`, the gate every read
  path uses, because exclusion is retroactive (ING-08) and a project excluded
  after its sessions were archived must not appear. The month side buckets a
  session by `substr(first_turn_at, 1, 7)`; on the live store 0 of 3,416
  sessions have a `last_turn_at` in a different month, and `first_turn_at` is
  established by byte order and coalesced by `ingest::write_session_meta` so a
  tail pass never revises it. Sessions with a null `project` and sessions with a
  null `first_turn_at` each get their own named bucket rather than being dropped,
  because an unbucketed null silently breaks a total that is supposed to
  reconcile - the live store has one of each. The FOOTPRINT table is per-table
  bytes from SQLite's `dbstat` virtual table, which the bundled amalgamation
  compiles in (`-DSQLITE_ENABLE_DBSTAT_VTAB` in libsqlite3-sys's build script),
  summing `pgsize` grouped by `name`, plus one explicit row for the freelist
  (`freelist_count` times `page_size`) so the rows sum exactly to
  `page_count` times `page_size`, which is the database file's size. That
  freelist row is also what makes `compact` legible: it is the number `VACUUM`
  reclaims. Measure whether `dbstat`'s aggregate form is fast enough on a
  gigabyte store before settling on the per-page form; the reconciliation
  property is what this task owes, not a particular query.
- **Verify:** `cargo test -p verbatim --test lifecycle` passes with new cases
  showing (a) `verbatim usage --json`'s per-project bytes sum to
  `SELECT sum(length(blob)) FROM sessions` over the visible sessions and the
  per-month bytes sum to the same number, (b) a session whose `project` is null
  and one whose `first_turn_at` is null each appear in a named bucket and their
  bytes are inside those totals, (c) the footprint rows sum exactly to
  `page_count * page_size` read off the same store, and (d) a session in an
  excluded project contributes to neither archive total.

### Task 3: `verbatim export`, portable and self-describing

- **Files:** crates/verbatim/src/cmd/export.rs, crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs, crates/verbatim/tests/lifecycle.rs
- **Action:** Add an `export` subcommand taking a destination directory path and
  `--json`. Chosen shape, since nothing in CONTEXT constrains it: it writes one
  `.jsonl` file per session, holding that session's uncompressed stream exactly
  as the archive stores it - `blob::read_all` gives those bytes and they are the
  transcript, which is the portable form for this product because it is the form
  the data arrived in and the form any future importer would read - plus one
  `manifest.json` beside them. A store-file copy is deliberately NOT what export
  is: that is the snapshot, and PLAN-4 owns it. The destination must not already
  hold an export; refuse rather than merge into one. Sessions come from
  `config::visible::sessions`, so an excluded project is not exported - export is
  a read path and ING-08 binds every read path alike. The manifest is the
  "states what it contains" half of PRIV-04 and says so in words: how many
  sessions and turns, which projects and which date range, the total bytes, the
  archive format integer, which sessions were evicted and therefore carry no
  body, and an explicit statement that the output is the verbatim, unredacted
  transcripts and that the derived tables, the decision log and the observations
  are not in it. Print the same statement on the terminal, before the byte
  counts, so a user who exports to a shared directory learns what they just
  wrote there without opening the manifest. Nothing is redacted on the way out:
  redaction is keyed on destination and a path the user named is not egress
  (PRIV-01), which is exactly why the statement has to be there instead.
- **Verify:** `cargo test -p verbatim --test lifecycle` passes with new cases
  showing (a) after `verbatim export <dir>` the per-session files concatenate
  byte-for-byte to what `blob::read_all` returns for each session, (b) the
  manifest names every exported session and the session count matches the file
  count, (c) an evicted session is named in the manifest as carrying no body and
  its file is empty rather than absent, (d) a session in an excluded project is
  in neither the manifest nor the directory, and (e) exporting into a directory
  that already holds an export exits non-zero and overwrites nothing.

### Task 4: Hold all three to the swept contract and document their shapes

- **Files:** crates/verbatim/tests/cli.rs, docs/json-shapes.md
- **Action:** Add `compact`, `usage` and `export` to the `DATA_COMMANDS` table in
  `crates/verbatim/tests/cli.rs`, each with the arguments that make it answer and
  the `data` field names it emits, so the five swept properties hold all three
  without a test file each - which is what that table's comment says it is for.
  `export` needs a destination inside the bench's temp directory rather than a
  fixed path. Add a `data` section per command to `docs/json-shapes.md` under
  `## data by command`, transcribing the same field lists, and state in prose
  what each number means: that `usage`'s archive totals reconcile to
  `sum(length(sessions.blob))` and NOT to the file size, that its footprint rows
  including the freelist row sum to the file size, that `compact`'s before and
  after are the same three files `status` reports, and that `export`'s output is
  the unredacted transcripts.
- **Verify:** `cargo test -p verbatim --test cli` passes with all three in
  `DATA_COMMANDS`, including the property test that every documented `data` field
  is present in the emitted document and no undocumented one is, and the
  empty-result test that each exits 0 against a machine with no store.

## Notes

- `compact` returning exit 1 when the ingest lock is held is deliberate and is
  the one place this plan diverges from "0 including an empty result": a
  compaction that did not happen is an operational failure, not an empty answer,
  and `cmd::uninstall`'s `--purge` arm already answers a held lock the same way.
- PLAN-3 shares `crates/verbatim/src/main.rs`, `crates/verbatim/src/cmd/mod.rs`,
  `crates/verbatim/tests/cli.rs` and `docs/json-shapes.md` with PLAN-2 and
  PLAN-4, and is SEQUENTIAL with both.
