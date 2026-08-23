---
phase: 1
plan: 2
requirements:
  - INJ-07
files:
  - tests/fixtures/session-envelope.jsonl
  - tests/fixtures/README.md
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/src/inject/brief.rs
  - crates/verbatim-core/tests/inject_brief.rs
  - crates/verbatim/tests/brief.rs
---

# Phase 1: Prompt-True Resume Brief - Plan 2 of 3 (the brief, and a fixture that can catch it)

**ORDER: after PLAN-1.** The one `AND` this plan adds names `turns.is_typed`,
which PLAN-1 declares and fills. It shares no file with PLAN-1 or PLAN-3.

## Goal

`SessionStart`'s "It last asked" quotes a turn the person typed, or renders no
prompt line at all - never the tool result or harness envelope Claude Code wrote
as a `user` record.

## Must be true when done

- A session whose last `user` record is a tool result renders "It last asked:"
  quoting that session's last typed prompt, and none of the tool result's text
  appears in the brief.
- A session whose last `user` record is a `<task-notification>` envelope or an
  `isMeta: true` record quotes the preceding typed prompt, not the envelope.
- A session with no typed `user` record at all renders no "It last asked" line,
  and still renders its date, its branch, "It last answered" and the index
  pointer.
- Two `SessionStart` spawns against an unchanged store are byte-identical, on a
  rooted fixture whose quoted turn moves under the new rule.
- The brief opens no additional blob and issues no additional statement: the
  last-turn lookup differs from today's by exactly one `AND`.

## Context

- D-11 binds the query: one extra `AND` on the existing `last_turn` statement
  and no new index. Measured against the live 1.16 GB store, the current lookup
  costs 0.002 ms and a variant forced to scan the same session's turns backwards
  without matching costs 0.362 ms, against a 10 ms wall; real scan depth is p50
  27, p90 181, p99 404, max 520 rows.
- D-12 binds what does not change: `inject/state.rs` needs no edit, because the
  turn ids travel on the `Quote` that was actually assembled.
- D-13 and D-15 bind the fixtures: the "no typed prompt at all" arm is
  fixture-only (0 of 700 real transcripts have one) and is constructible from
  `session-errors-a.jsonl`; the byte-identity assertion needs a new rooted
  fixture, because `session-recall.jsonl`'s only `user` record is a text block
  and cannot observe the change.
- Out of scope: everything the brief renders besides the quoted-prompt line, and
  `recall/search.rs`, `recall/get.rs` and `recall/context.rs`, which name
  `record_type` but not this column (D-10).

## Tasks

### Task 1: A rooted fixture whose last `user` record is not typed

- **Files:** tests/fixtures/session-envelope.jsonl, tests/fixtures/README.md,
  crates/verbatim-core/src/testkit.rs
- **Action:** Write one transcript fixture, five records, in this order: a
  `user` text record carrying a distinctive phrase (the prompt the brief must
  end up quoting); an `assistant` record with a `tool_use` block; a `user`
  record whose `message.content` carries a `tool_result` block and whose
  top-level `toolUseResult` carries a `stdout` string only - no `stderr`, no
  `is_error`, no `attachment`; an `assistant` text record; and last a `user`
  text record whose text opens with `<task-notification>`. That order is the one
  `session-capture.jsonl` already has and D-15 points at, plus the envelope
  record, so this one fixture serves the moved-quote case, the envelope case and
  the byte-identity case at once. Every record's `cwd` is
  `{{ROOT}}/project-gamma`, the `FIXTURE_ROOT_TOKEN` spelling every rooted
  fixture uses; the file ends with a newline and contains no CR; the `sessionId`
  is a UUID no other fixture uses; carry a `gitBranch` so the brief's head line
  renders whole. Register it in `TRANSCRIPT_FIXTURES` and in `ROOTED_FIXTURES`
  in `testkit.rs`, both appended at the end, and update the `ROOTED_FIXTURES`
  doc, which currently says two projects. A third project key is what keeps this
  fixture inert: `project-alpha` and `project-beta` are what every existing
  project-scoped assertion is written against, and `fixtures.rs` asserts only
  that the rooted fixtures name at least two projects. Choose its tokens so it
  collides with nothing another phase counts - no `SearchManager`, no
  `brillig`, none of the command lines or paths the entity tests match on, and a
  tool name whose entity count no test pins. Document it under `## What each one
  pins` and `## The rooted cwd` in `tests/fixtures/README.md`. If a swept count
  assertion anywhere else moves, change THIS FIXTURE and never the assertion:
  those counts are other phases' evidence.
- **Verify:** `cargo test --workspace --features testkit` passes with no edit to
  any assertion outside the three files above; and a store seeded from this
  fixture alone has exactly two `user` rows reading `is_typed = 0` (the tool
  result and the envelope) and one reading 1, with the typed one earlier in
  `turn_seq` than both.

### Task 2: The brief quotes a typed prompt

- **Files:** crates/verbatim-core/src/inject/brief.rs,
  crates/verbatim-core/tests/inject_brief.rs
- **Action:** `last_turn` is the only statement that changes: one extra `AND`
  that keeps a row whose `is_typed` is not 0, written so the same statement
  still serves the `assistant` lookup - those rows carry null by construction -
  and still serves a preserved evicted session, whose rows reindex skips and
  which keep null (D-04). For an evicted session the choice is unobservable
  anyway: `retention::apply` empties the blob, so `last_exchange` gets no reader
  and renders no quote either way. No new index, no second query, no second blob
  read - `last_exchange` reads one blob for the pair and that must stay true
  (D-11). Nothing else in the module changes: the "no typed prompt at all" arm
  needs no code, because `last_turn` returning `None` already leaves
  `Continuity`'s prompt `None` and `Continuity::text` already renders no "It
  last asked" line. Do not touch `inject/state.rs`: `quoted` reads the ids off
  the block that was actually assembled, so INJ-04's suppression follows the new
  rule for free (D-12). Update the module doc and `last_exchange`'s doc, which
  today describe the quoted turn as the last `user` turn. Then add the cases to
  `tests/inject_brief.rs`; its `archive` helper builds records from JSON values,
  so the envelope and tool-result shapes can be written there, and its bench
  already owns a root the rooted fixtures can be copied under for the
  `session-errors-a.jsonl` case.
- **Verify:** `cargo test -p verbatim-core --features testkit --test inject_brief`
  passes with new cases: a session ending in a tool result quotes the earlier
  typed prompt and the brief contains none of the tool result's text; a session
  ending in a `<task-notification>` envelope and one ending in an `isMeta: true`
  record each quote the preceding typed prompt; a store seeded only from
  `session-errors-a.jsonl` renders a brief that carries "It last answered" and
  no "It last asked". `git diff` on `inject/brief.rs` shows exactly one added
  `AND`, inside `last_turn`.

### Task 3: Byte identity at the process boundary, on a brief that moved

- **Files:** crates/verbatim/tests/brief.rs
- **Action:** `two_session_starts_against_an_unchanged_store_are_byte_identical`
  seeds from `session-recall.jsonl`, whose only `user` record is a text block,
  so today it cannot observe this phase's change at all (D-15). Point that test
  at task 1's fixture and its `project-gamma` directory. Keep
  `a_hundred_session_starts_stay_inside_the_budget_and_the_wall_clock` on
  `session-recall.jsonl`, so the wall-clock baseline that test has been measuring
  still means what it meant. Add the falsifying half to the byte-identity test:
  the brief must contain the fixture's typed prompt phrase and must contain
  neither the envelope's text nor the tool result's, or two identical empty
  briefs would satisfy the assertion. The existing rule about the harness
  environment stands - every spawn sets `VERBATIM_DATA_DIR`,
  `VERBATIM_CONFIG_DIR` and `CLAUDE_CONFIG_DIR` at temporary directories.
- **Verify:** `cargo test -p verbatim --test brief` passes; the byte-identity
  test's two runs produce identical stdout, that stdout carries the typed
  prompt phrase and neither of the two not-typed records' text, and it still
  carries no `NN:NN` time of day.

## Notes

- Prior art carried forward: v0.1.0 phase 5's D-10
  (`.planning/_archive-v0.1.0/5/CONTEXT.md`, phase 5) fixed the resume brief's
  cold-start budget by barring process spawn from this path entirely. Task 2
  stays inside that: one added `AND` on a statement already planned against
  `sqlite_autoindex_turns_1`, no second query, no second blob read, and nothing
  new opened.
