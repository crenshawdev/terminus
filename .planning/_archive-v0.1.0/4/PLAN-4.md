---
phase: 4
plan: 4
requirements:
  - ING-11
files:
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/backfill.rs
  - crates/verbatim/src/cmd/install/mod.rs
  - crates/verbatim/tests/backfill.rs
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/ingest/mod.rs
  - crates/verbatim-core/src/ingest/backfill.rs
  - crates/verbatim-core/tests/backfill.rs
---

# Phase 4: Hooks And Install - Plan 4 of 5 (backfill)

**SEQUENTIAL: PLAN-1 -> PLAN-2 -> PLAN-3 -> PLAN-4.** This plan spawns through
PLAN-1's detach helper and edits `crates/verbatim/src/cmd/install/mod.rs`, which
PLAN-2 creates. Do not run it before either.

## Goal

A new user's whole history is archived without them waiting for it: install tells
them how many sessions, how many bytes and roughly how long, hands the shell
straight back, and the work continues in a process that resumes where it stopped
if it is ever interrupted.

## Must be true when done

- `verbatim backfill` over the real corpus prints a session count, a byte total
  and a time estimate before any ingest work happens, and returns the shell in
  under 100 ms (AC8).
- Killing the backfill mid-pass and rerunning it converges to the same store
  contents as an uninterrupted run (AC8).
- The backfill's parse and compression run across a fixed number of threads while
  exactly one thread performs the SQLite writes, and no thread-pool crate is
  linked (D-11).
- A backfill running while a hook fires costs the hook nothing: the second process
  loses the lock race and exits 0 immediately.
- `verbatim install` prints the estimate and starts the backfill without the user
  waiting for it.

## Context

- D-22: backfill is `crates/verbatim-core/src/ingest/pass.rs`'s pass with an
  estimate printed in front of it and a detached spawn around it. Resumability
  needs no new state - it comes from the per-file watermarks
  (`crates/verbatim-core/src/ingest/mod.rs:380` `write_watermark`) and the
  per-file transactions phase 2 already ships, which
  `crates/verbatim/tests/crash.rs` resumes to a byte-identical reference store
  across 24 SIGKILLs.
- D-11: bounded parallelism is a fixed count of `std::thread` workers with a
  single thread owning the SQLite writes, started only by backfill. No `rayon`,
  no global pool - `Cargo.toml` has none and the hook path must not pay for one.
- D-23: the estimate is calibrated against a measured rate, not a guess. The full
  real corpus - 2,248 `.jsonl` files, 1.1 GB at `/data/claude/.claude/projects` -
  ingested single-threaded in 36,582 ms on 2026-08-13, producing a 735 MB store
  of 2,244 sessions and 274,422 turns; the following steady-state pass took 92 ms.
- `crates/verbatim-core/src/discover.rs:24-26` says outright that
  bounded-parallelism backfill is this phase's, and that its sequential walker is
  what phase 4 fans out.
- Out of scope: retention, capture mode and compaction (phase 8), and any change
  to what a hook-triggered pass does.

## Tasks

### Task 1: The estimate, before any work

- **Files:** crates/verbatim/src/cmd/backfill.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs
- **Action:** Add a `backfill` subcommand whose first output is an estimate:
  how many transcripts `discover::discover` yields, how many bytes of them are
  unread (each file's length minus its stored watermark, so a partly ingested
  corpus estimates the remainder and not the whole), and how long that is likely
  to take. Calibrate the time from D-23's measured rate - 1.1 GB in 36,582 ms
  single-threaded - divided by the worker count Task 3 fixes, and say in the
  output that it is an estimate from a measured rate rather than a promise.
  Compute it read-only: open through `crates/verbatim/src/cmd/read.rs`'s
  `open_in` so a machine that has never ingested estimates the whole tree and
  creates no store doing it, and take file sizes from directory metadata rather
  than by opening anything. The whole estimate must fit inside AC8's 100 ms
  budget over 2,248 files, so it is one tree walk, one `metadata` call per file
  and one query for the watermarks - never a per-file query.
  Add `backfill` to `USAGE` and to `dispatch` in `main.rs`.
- **Verify:** `verbatim backfill` against a temp `CLAUDE_CONFIG_DIR` holding a
  handful of fixture transcripts prints a session count, a byte total and a time
  estimate before anything else; run against the real corpus with
  `VERBATIM_TEST_CORPUS` set, the printed session count is within a file or two
  of `find <corpus> -name '*.jsonl' | wc -l` and the byte total within a percent
  of `du -sb`; against an empty tree it prints zeroes and exits 0.

### Task 2: Backfill runs detached and hands the shell back

- **Files:** crates/verbatim/src/cmd/backfill.rs, crates/verbatim/tests/backfill.rs
- **Action:** After printing the estimate, spawn the work through PLAN-1's detach
  helper and return, so the shell comes back in under 100 ms with the ingest
  continuing (AC8, ING-11). The spawned process must be invoked with an additional
  internal argument meaning "do the work, print no estimate, spawn nothing" -
  without it the child prints an estimate and spawns again, forever. Keep that
  argument out of `USAGE`: it is a mechanism, not a command.
  At this task the detached child runs the existing
  `crates/verbatim-core/src/ingest/pass.rs` `pass::run`, so backfill is working
  end to end before any parallelism exists and Task 3 has a sequential reference
  to be checked against. The pass already takes the ingest lock before opening the
  store, so a hook firing during a backfill loses the race and exits 0 with empty
  stdout (`crates/verbatim/src/cmd/ingest.rs:41-43`), and nothing new is needed
  for exclusivity (D-18).
- **Verify:** `crates/verbatim/tests/backfill.rs` measures wall time from spawn to
  exit for `verbatim backfill` over a temp tree of fixture transcripts and asserts
  under 100 ms while the store is still empty at that instant, then waits and
  asserts the sessions arrived; `verbatim ingest` run immediately after the
  backfill returns exits 0 in under 50 ms with empty stdout, the way
  `crates/verbatim/tests/lock_race.rs` already asserts for two ingests.

### Task 3: The bounded-parallelism pipeline

- **Files:** crates/verbatim-core/src/ingest/backfill.rs,
  crates/verbatim-core/src/ingest/mod.rs, crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/tests/backfill.rs
- **Action:** Add a pass whose read, parse and zstd compression happen on a fixed
  count of `std::thread` workers while exactly one thread owns the writable
  SQLite connection and performs every write, including every per-file
  transaction and every watermark (D-11). Start the threads only from this entry
  point - nothing on the hook path may start one - and use no `rayon` and no
  global pool.
  Hold the invariants the sequential pass holds, because resumability is entirely
  theirs (D-22): the ingest lock taken once for the whole run before the store is
  opened; recovery at the top through `crate::recover::recover`; one transaction
  per file, so a kill loses at most one file; a per-file failure recorded against
  its path and skipped rather than propagated (D-12); one `runs` row for the whole
  pass. Reuse `pass.rs`'s `Summary`, `PassOutcome`, `note` and its `runs`-row
  writing rather than growing a second account of what a pass did.
  Bound the work in flight - the "chunked" half of ING-11 - so peak memory is the
  worker count times a bounded queue and not the corpus: real transcripts are p50
  290 KB, p90 1.03 MB, p99 3.1 MB, and an unbounded queue over 2,248 of them is
  the whole corpus resident.
  Amend `crates/verbatim-core/src/lib.rs`'s claim that the crate "links no async
  runtime, no HTTP client and no thread pool" so it stays true and precise: no
  thread-pool crate is linked, and the only threads the crate starts are this
  fixed set, started only by backfill. A doc comment that has quietly become false
  is worse than one that names the exception.
- **Verify:** `cargo test -p verbatim-core --features testkit --test backfill`
  passes: running the pipeline with four workers over the in-repo fixture set
  produces the same row counts in every table of `store::TABLES`, the same
  per-session blob checksums and the same watermarks as `pass::run_with` over the
  same tree, and the pipeline reports work having been done on more than one
  thread; a tree with more files than workers leaves no file unprocessed.

### Task 4: Backfill runs the pipeline, and still converges after a kill

- **Files:** crates/verbatim/src/cmd/backfill.rs, crates/verbatim/tests/backfill.rs
- **Action:** Point the detached child at Task 3's pipeline instead of
  `pass::run`, with the worker count fixed in one place and bounded well below a
  machine's core count - the ceiling is SQLite's single writer, not the CPU, and
  compression is the only stage that scales. Nothing else about the invocation
  changes: same lock, same recovery, same per-file transactions, same `runs` row,
  so a killed backfill resumes exactly as phase 2's crash harness already proves a
  killed pass does (D-22).
- **Verify:** `crates/verbatim/tests/backfill.rs` runs a backfill to completion
  over a fixture tree, records the row counts and blob checksums, then repeats
  against a fresh data directory while SIGKILLing the working process at
  randomized points and rerunning until it completes, and asserts the two stores
  match - the shape `crates/verbatim/tests/crash.rs` already uses. Against the
  real corpus with `VERBATIM_TEST_CORPUS` set, a full backfill produces the same
  session and turn counts as a `verbatim ingest` tree pass over the same corpus.

### Task 5: Install starts the backfill

- **Files:** crates/verbatim/src/cmd/install/mod.rs
- **Action:** After install has written both settings files and printed its
  summary, print the backfill estimate and start the detached backfill, so install
  returns immediately and the archive fills in behind it. This is the design
  brief's step 6 and the last thing install does; it must not gate any earlier
  step, and a backfill that cannot be started is one reported line and not a
  failed install - the settings are already written and the next hook would do the
  same work anyway.
  Say in the summary that it is running and that `verbatim status` is where to
  watch it, since the process is detached and there is no log file by design.
- **Verify:** `verbatim install --yes` against a temp `CLAUDE_CONFIG_DIR` holding
  fixture transcripts returns in under a second having printed the estimate, and a
  `verbatim status` a moment later shows a nonzero session count that was zero
  when install returned; with the transcript tree made unreadable, install still
  exits 0 and reports the backfill line as a problem.

## Notes

- `.planning/CAPTURE.md` (phase 1) records that `cargo test -p verbatim-core`
  runs zero of the `#![cfg(feature = "testkit")]` test files. Task 3's new test
  file inherits that hazard: if it is gated the same way, confirm it actually runs
  under the command in its Verify rather than reporting a green empty set.
- The AC8 100 ms budget is measured against the real corpus, which needs
  `VERBATIM_TEST_CORPUS` pointed at `/data/claude/.claude/projects`. The fixture
  tree proves the shape; only the real corpus proves the number.
