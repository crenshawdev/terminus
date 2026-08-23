---
phase: 5
plan: 2
requirements:
  - INJ-01
  - INJ-02
files:
  - crates/verbatim-core/src/inject/brief.rs
  - crates/verbatim-core/tests/inject_brief.rs
  - crates/verbatim/tests/brief.rs
---

# Phase 5: Context Injection - Plan 2 of 4 (the resume brief)

**ORDER: after PLAN-1. May run in parallel with PLAN-3** - that plan writes
`inject/prompt.rs`, `recall/search.rs` and the retrieval tests and shares no
file with this one. PLAN-4 comes after both and edits this plan's
`inject/brief.rs`, so it must not run beside it.

`crates/verbatim-core/src/inject/mod.rs` is deliberately NOT in this plan's
lease: PLAN-1's seam already hands the brief the data directory, the `Config`
and all five payload fields, so this plan deepens one file.

## Goal

A session that starts in a project the archive knows opens with a short, stable
account of where the last one left off - the session, its date, its branch, what
was last said, and how much is indexed - inside its configured budget and inside
single-digit milliseconds, and identical to the byte on the next run.

## Must be true when done

- `verbatim hook SessionStart` in a project with indexed history emits a brief
  naming the last session in that project, the date it ended at day resolution,
  the branch it ended on, its last user prompt and its last assistant turn, and
  the sessions-and-turns index pointer.
- No git process runs on the SessionStart path.
- The brief never exceeds the configured character budget, and never exceeds
  10,000 characters whatever the config says.
- Two SessionStart runs against an unchanged store write byte-identical stdout,
  and no time-of-day substring appears anywhere in it (AC2).
- Over 100 runs against a store holding indexed history, SessionStart exits 0
  with exactly one JSON object on stdout at p99 under 10 ms (AC1).

## Context

- D-17 binds which session: the greatest `session_meta.last_turn_at` for the
  scoped project. `is_final` is declared in the schema and never written - a
  grep across `crates/verbatim-core/src` finds no writer - so it cannot be the
  test.
- D-10 binds the working-state delta: branch only, from `session_meta.branch`,
  which is the sole git fact the archive holds. No `std::process::Command` on
  this path; `project::Resolver`'s `git rev-parse` costs 10-30 ms against a
  single-digit-millisecond budget and phase 3 D-12 already bars it from the
  cold-start path. This knowingly under-delivers `DESIGN-BRIEF.md:230`; the
  model can check live git itself in one cheap tool call.
- D-16 binds the budget: a character proxy, never a tokenizer.
- D-18 binds observations: phase 7 owns the `observations` table and the brief
  must be complete without it. No query names it and no code in this plan
  mentions it.
- INJ-02 binds volatility: stable ordering, dates rounded to the day, nothing
  that reads a clock.
- Phase 3 D-22 measured intra-session timestamps running backwards in 31 of 62
  sampled transcripts, which is why `recall::context` orders by `turn_seq` and
  never by `ts`; the same rule applies to anything this brief orders inside a
  session.
- Out of scope: any retrieval (PLAN-3), the per-session state file and what the
  brief must record in it (PLAN-4), and the index pointer itself, which PLAN-1
  already wrote.

## Tasks

### Task 1: The last session in this project, and the branch it ended on

- **Files:** crates/verbatim-core/src/inject/brief.rs
- **Action:** add INJ-01's first two blocks to the brief PLAN-1 started. "The
  last session in this project" is the `session_meta` row with the greatest
  `last_turn_at` among the rows whose `project` is the one `scope::resolve`
  returned, excluding sessions whose `parent_session_key` is set - those are
  subagent sidecars and the brief carries no subagent turns
  (`DESIGN-BRIEF.md:239`). Order over the stored string directly: all 22,412
  turns of a 180-file sample carry exactly the shape `NNNN-NN-NNTNN:NN:NN.NNNZ`
  (phase 3 D-23), so the comparison is lexicographic and there is no date
  parsing on the stored side. Render its date as the first ten characters of
  that string, the way `cmd::search::day` already does and with the same
  guard against a multi-byte boundary - slicing `&ts[..10]` panicked the whole
  command once already. The block carries that session's last user turn and its
  last assistant turn: select them from `turns` by `session_key` and
  `record_type`, in `turn_seq` order and never in `ts` order, and read each
  one's bytes out of `sessions.blob` through `BlobReader::open` and
  `BlobReader::read_range` at the `stream_offset` and `byte_len` already on the
  turn row, then project them with `index::text::project` - the same projection
  ingest indexed them with and the excerpt path reads them through, so the brief
  says what the turn said rather than what its JSON looks like. One blob read
  for the pair, not two: both turns are in the same session. The working-state
  delta is `session_meta.branch` off that same row and nothing else (D-10); no
  `std::process::Command` appears anywhere in this file. Nothing here queries or
  names `observations` (D-18). A project whose last session has no meta row, no
  branch, no timestamp or an unreadable blob renders the blocks it can and drops
  the ones it cannot, because a brief that fails is a SessionStart that emits
  nothing.
- **Verify:** `cargo test -p verbatim-core --features testkit --test inject_brief`
  passes with cases showing: over a store holding two sessions of one project
  with different `last_turn_at`, the brief names the later session's date,
  branch and turn text and none of the earlier session's; a third session with
  `parent_session_key` set and a later `last_turn_at` does not become the named
  session; a session whose `branch` is null still renders the rest; and a grep
  of `crates/verbatim-core/src/inject/` finds no `Command` and no
  `observations`.

### Task 2: The brief inside its configured budget

- **Files:** crates/verbatim-core/src/inject/brief.rs
- **Action:** enforce the `brief_chars` budget PLAN-1 put in `Config` over the
  whole rendered brief, counted in characters (D-16). Cut the variable-length
  parts first - the quoted user prompt and assistant text, which are the only
  parts whose size the archive controls - before any block is dropped, so the
  index pointer and the branch line always survive: they are the cheapest and
  the most useful, and the pointer is the ~30 tokens that tell the model
  searchable memory exists at all. Mark every cut with `recall::excerpt::ELISION`
  rather than a second spelling of three dots, and cut on a character boundary.
  Independently of the configured number, never emit more than 10,000
  characters: bundle 2.1.237 persists a hook stdout longer than that to disk and
  replaces it with a reference, so a brief past the ceiling stops being context
  and becomes a file path.
- **Verify:** `cargo test -p verbatim-core --features testkit --test inject_brief`
  passes with cases showing: with a configured budget of a few hundred
  characters over a store whose last session holds a very long assistant turn,
  the rendered brief is at or under the budget and still names the date, the
  branch and the counts; with a configured budget above 10,000 the brief is at
  or under 10,000; and a brief that was cut carries the elision marker.

### Task 3: Nothing volatile, byte for byte

- **Files:** crates/verbatim-core/src/inject/brief.rs,
  crates/verbatim/tests/brief.rs
- **Action:** INJ-02. Every value in the brief comes from a stored row, in an
  order fixed by a stored key, at day resolution. Nothing on this path reads a
  clock: no `SystemTime`, no `Instant`, no elapsed-time or "as of" phrasing, no
  count that depends on when the read happened. Anything list-shaped is ordered
  by a stored value with a total order, for the same reason `recall::search`'s
  `TAIL` is total - a tie broken differently between two runs is a brief that
  changes without the archive changing. The stated rationale is the Anthropic
  prefix cache, which nothing local can observe; the byte-identity property is
  the thing being built and it stands on its own.
- **Verify:** AC2. `cargo test -p verbatim --test brief` passes with a test that
  runs `verbatim hook SessionStart` twice against an unchanged store, seeded
  through `testkit::copy_rooted_fixture_into` under a test-owned root and
  ingested to completion first so the hook's own spawned pass finds nothing new,
  and asserts the two stdouts are equal byte for byte and that neither contains
  any substring matching a two-digit-colon-two-digit time of day.

### Task 4: The brief inside its wall budget, over a hundred runs

- **Files:** crates/verbatim/tests/brief.rs
- **Action:** AC1's measurement. Seed a temp data directory with the rooted
  fixtures under a test-owned root, run one ingest to completion so the store
  holds indexed history for the project the payload's `cwd` names, then spawn
  `verbatim hook SessionStart` 100 times with that payload on stdin, timing each
  the way `crates/verbatim/tests/hook.rs` times its four events. Assert every
  run exits 0, writes exactly one line to stdout that parses as JSON with
  `hookSpecificOutput.hookEventName` equal to `SessionStart`, and carries an
  `additionalContext` at or under the configured budget; assert p99 under the
  same 10 ms that file already asserts, which D-11 says now absorbs a read-only
  store open plus the injection queries against the 0.65-0.76 ms phase 4
  measured. Print p50 and p99 rather than only asserting them, so a regression
  names a number a reader can compare against the next run.
- **Verify:** `cargo test -p verbatim --test brief` passes and prints a p50 and
  a p99, with the p99 under 10 ms.

## Notes

- The seeding rule matters twice over. The fixtures that hardcode
  `/data/code/verbatim` make a project-scoped assertion true only on a checkout
  at that literal path, and `testkit::FIXTURE_ROOT_TOKEN` exists to avoid
  exactly that; and seeding by running a full ingest BEFORE the hook runs is
  what keeps the store unchanged across AC2's two runs, since every hook spawns
  a tree pass of its own and a pass that finds new bytes would change the counts
  the brief prints.
