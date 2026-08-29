---
phase: 1
status: complete
completed: 2026-08-29
---

# Phase 1: Prompt-True Resume Brief - Summary

`SessionStart`'s "It last asked" now quotes the session's last turn the user
actually typed, by classifying every `user` record as typed or harness-written
at parse time into a new `turns.is_typed` column and adding one null-safe `AND`
to the brief's `last_turn` lookup.

## What shipped

- `turns.is_typed`, an `INTEGER` column set only on `record_type = "user"` -
  declared in the store schema (`DERIVED_SCHEMA` 3 -> 4) and listed in
  `BRING_FORWARD_COLUMNS`, so a previous build's store gains it and is
  backfilled from blobs rather than requiring a re-ingest.
- The classifier, in the parser (`crates/verbatim-core/src/parse.rs`): a
  `tool_result` block anywhere in `message.content`, `isMeta: true`, or a
  LEADING envelope tag from a 6-tag `const` marks a record not typed.
  `bash-input` is deliberately typed. It never reads `toolUseResult`.
- One production line in the brief: `AND is_typed IS NOT 0` inside `last_turn`
  (`crates/verbatim-core/src/inject/brief.rs`). No new index, no second query,
  no second blob read. `inject/state.rs` untouched, as D-12 predicted.
- `tests/fixtures/session-envelope.jsonl`, a rooted fixture whose last `user`
  record is not typed and whose typed prompt is earlier - the shape no existing
  fixture had.
- The two downstream claims, asserted rather than assumed: the session state
  file records the ids of the turns the brief actually quoted
  (`tests/suppression.rs`), and the cold-start budget holds against a store
  built from the real corpus (`tests/corpus.rs`).

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | e4611c4 | Declare `turns.is_typed` and force the rebuild that fills it |
| 1 | 2 | 0eb40b4 | Classify a `user` record as typed or harness-written |
| 1 | 3 | d5c21e4 | Write the discriminator through the one seam |
| 1 | 4 | bad6aa2 | Bring a previous build's store forward and backfill it from blobs |
| 2 | 1 | 875ad04 | A rooted fixture whose last `user` record is not typed |
| 2 | 2 | 4af66ab | Quote a typed prompt in the resume brief, never a tool result |
| 2 | 3 | b314507 | Assert byte identity on a brief whose quoted turn moved |
| 3 | 1 | 3c6a55a | The state file names the typed turn the brief quoted |
| 3 | 2 | c313888 | The cold-start budget, measured on the real corpus |

## Deviations

- [deviation] Plan 1 task 4's `Verify:` predicted that removing the
  `BRING_FORWARD_COLUMNS` entry fails with `no such column: is_typed`. SQLite
  raised its INSERT-path wording instead, `table turns has no column named
  is_typed`, from `derive_turn` inside `reindex::open_up_to_date`. Same defect,
  same site, only the message text differs - the falsification stands and
  nothing was changed to accommodate it. (bad6aa2)

- [deviation] Plan 3 task 2's `Verify:` and the plan's "Must be true when done"
  bullet 2 both assert a flat 10 ms p99 for the gated corpus arm, matching
  CONTEXT AC4. That is false in the DEBUG profile `cargo test` builds by default
  and true in RELEASE. What shipped is a profile-aware wall the user chose at a
  structural checkpoint: 10.0 ms asserted when `cfg!(debug_assertions)` is
  false, 20.0 ms when it is true, with the profile and the wall in force named
  on the printed per-event line. The overage is NOT this phase's `AND` - with
  it reverted the debug p50 measured 10.24 ms against 10.38 ms with it, inside
  the noise and consistent with D-11. It is v0.1.0's INJ-01 index pointer,
  `crates/verbatim-core/src/inject/brief.rs:580`, whose
  `SELECT count(*) FROM turns JOIN session_meta` scales with the scoped project.
  CONTEXT AC4 is left as written for `/cad-verify` to adjudicate. (c313888)

- [deviation] The premise the 20 ms debug wall was chosen on no longer holds.
  The choice was made against a measured debug p99 of 10.85 ms, making 20 ms
  roughly 2x headroom. Re-measured hours later on a freshly built store the
  debug numbers are p50 18.48 ms and p99 19.72 ms - the wall passes with 1.4%
  margin. The executor shipped the 20 ms as chosen rather than tuning it to the
  measurement. (c313888)

## Open items

- **The debug wall will not survive the next corpus growth, and the growth is
  this project's own.** The first store held 3,580 sessions / 456,038 turns and
  measured `SessionStart` at 10.85 ms p99; a store built hours later from the
  same `$HOME/.claude` holds 4,360 sessions / 661,622 records and measures
  19.72 ms p99, against a 20.0 ms wall. This is growth, not run-to-run noise:
  the p50-to-p99 spread is 1.24 ms, a tight distribution around a genuine
  ~18.5 ms cost, where contention would show a low p50 and a long tail. The
  cost scales with the SCOPED project, and the project the payload scopes to is
  `verbatim` itself, which every executor dispatch in this phase appends
  transcript to - so the measurement climbs each time the phase is worked on. A
  decision is owed on whether the debug arm should assert a wall at all or only
  print. The release arm at 10 ms is unaffected and had 58% margin.
- `cargo fmt --check` fails across the workspace on files committed by plans 1
  and 2 - `crates/verbatim/tests/{hook,retention}.rs` and
  `crates/verbatim-core/tests/{inject_brief,parse,reindex,retention,verify}.rs`.
  Fourteen diffs, all line-wrapping of `assert!` and `format!` calls, plus one
  import order. Outside plan 3's lease and not caught by the project's detected
  lint or typecheck commands, neither of which runs `rustfmt`.
- `crates/verbatim/tests/hook.rs`'s
  `the_ingest_survives_a_group_kill_and_a_descendant_sweep` is flaky under a
  full workspace run: failed once, then passed 3/3 targeted, 3/3 as the whole
  `hook` binary, and again on a clean `--workspace --no-fail-fast`. It reads the
  live process table for a `verbatim ingest` argv that other binaries in the
  suite also spawn, so a foreign ingest is the likely confound. Pre-existing.
- `UserPromptSubmit` injected nothing on all 100 corpus runs (the fixture's
  prompt, scoped to the newest project). Expected under INJ-05's
  precision-first thresholds and not asserted either way, but it means only
  `SessionStart` is measured doing injection work.
- The 6-tag envelope list is spelled a second time in `tests/parse.rs` rather
  than the parser's `const` being made `pub`. Deliberate: a test walking the
  production list would pass just as happily after a tag was deleted from it,
  and a `pub` const would put upstream's vocabulary in the crate's API.
- The corpus-built store is gone (built in a `tempfile::tempdir()` removed when
  the pass returns), so a further measurement pays the ~355 s ingest again.

## Goal check

The nine commits plausibly deliver the phase goal. The chain is complete and
each link is asserted by a test that was falsified in both directions: the
parser sets `is_typed` only on `user` records (`--test parse` 22 passed,
covering both content shapes and elision-invariance), `derive_turn` is the one
seam that writes it (`--test derive` 10 passed, with both null-count queries
returning 0 across every fixture), an existing store gains and backfills the
column rather than needing a re-ingest (`--test reindex` 12 passed, failing
inside `open_up_to_date` when the `BRING_FORWARD_COLUMNS` entry is removed),
and the brief's `last_turn` reads it (`--test inject_brief` 9 passed - three
new cases covering a tool result, a `<task-notification>` envelope and an
`isMeta: true` record all quote the earlier typed prompt, and all three fail
with the `AND` removed while the six older ones still pass). The claims made
outside the brief's own text hold too: byte identity at the process boundary on
a brief whose quoted turn moved (b314507), and the session state file naming the
typed turn's id and not the envelope's (3c6a55a). INJ-04's suppression therefore
still covers exactly what the brief put on screen, which was D-12's prediction
and is now a test.

What is NOT delivered as written is CONTEXT AC4's flat 10 ms p99 over the real
corpus. It holds in release (`SessionStart` p99 4.26 ms, 58% margin) and fails
in the debug profile `cargo test` builds by default, where the measurement is
now 19.72 ms p99 against the 20.0 ms wall that shipped. The cause is not this
phase - it is v0.1.0's INJ-01 index pointer at
`crates/verbatim-core/src/inject/brief.rs:580`, whose `count(*)` scales with
the scoped project - but the criterion as written is unmet and the debug wall
has 1.4% margin against a number that grows every time this project is worked
on. That is the one gap, and it is carried above as the first open item.
