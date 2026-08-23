---
phase: 1
plan: 3
requirements:
  - INJ-07
files:
  - crates/verbatim/tests/suppression.rs
  - crates/verbatim/tests/corpus.rs
---

# Phase 1: Prompt-True Resume Brief - Plan 3 of 3 (the two claims made downstream)

**ORDER: after PLAN-1 and PLAN-2.** Both tasks assert properties of the brief
PLAN-2 changes. It shares no file with either earlier plan.

## Goal

The two claims this phase makes outside the brief's own text hold: the session
state file names the turns the brief actually quoted under the new rule, and the
cold-start budget survives the new rule on the real corpus rather than on a
five-record fixture.

## Must be true when done

- After a `SessionStart` on a session whose last `user` record is not typed, the
  ids recorded under `brief` in the session state file are the typed turn's,
  and the not-typed turn's id is not among them - so INJ-04's suppression still
  covers exactly what the brief put on screen.
- Against a store built from the real corpus, 100 spawns of each of the four
  hook events stay under the same 10 ms p99 `crates/verbatim/tests/hook.rs`
  asserts, with `SessionStart` rendering a brief on every one of its runs.
- With `VERBATIM_TEST_CORPUS` unset the corpus test still passes and says out
  loud that it skipped.
- Nothing derived from the private corpus - no brief text, no project key, no
  path - reaches the test's output.

## Context

- D-12 predicts that no production change is needed for the state file, because
  the ids travel on the `Quote` that was assembled. Task 1 is the test that
  makes that prediction falsifiable rather than assumed.
- D-14 binds the corpus measurement: the existing `VERBATIM_TEST_CORPUS` gate
  and its loud-skip convention, no new mechanism, and `VERBATIM_CONFIG_DIR` kept
  temporary or the developer's real `roots` win.
- The two wall budgets already in the tree are both 10 ms: `hook.rs`'s
  `every_event_exits_zero_with_an_empty_stdout_inside_the_budget` and
  `brief.rs`'s hundred-run test.

## Tasks

### Task 1: The state file names the turn the brief actually quoted

- **Files:** crates/verbatim/tests/suppression.rs
- **Action:** `a_turn_the_brief_already_quoted_is_not_injected_again` already
  fires a `SessionStart` and reads the `brief` id list out of the session state
  file through the bench's `state` and `ids` helpers. Add a case beside it for a
  session whose LAST `user` record is not typed and whose typed prompt is
  earlier: the bench's `archive` helper takes records as JSON values, so the
  session can be written there without adding a fixture. Assert that the id
  recorded under `brief` is the typed turn's - resolved through the bench's
  `turn_of` helper, which maps a record uuid to a turn id - and that the
  not-typed turn's id is not in the list, and keep the existing control shape:
  assert first that the brief actually quoted something, or the test passes on a
  brief that rendered nothing. This is a test-only task; no production file
  changes, which is D-12's claim.
- **Verify:** `cargo test -p verbatim --test suppression` passes; reverting the
  one `AND` PLAN-2 added to `last_turn` makes the new case fail on the id it
  recorded.

### Task 2: The cold-start budget, measured on the real corpus

- **Files:** crates/verbatim/tests/corpus.rs
- **Action:** Extend the single existing corpus test with the measurement, as a
  helper called after the pass has built the store - not as a second `#[test]`.
  Two reasons, and the second is the binding one: the ~988 MB pass is the
  expensive part and this file's own doc already gives that as the reason it is
  one test and not six; and two tests in one binary run as parallel threads, so
  a second corpus ingest running beside a wall-clock measurement would destroy
  the measurement. Spawn `verbatim hook <event>` 100 times for each of the four
  events, timing each spawn and reporting p50 and p99 the way both existing
  budget tests do, and assert p99 under the same 10 ms they assert. Point
  `VERBATIM_DATA_DIR` at the corpus-built store, and point `CLAUDE_CONFIG_DIR`
  and `VERBATIM_CONFIG_DIR` at empty temporary directories: every hook detaches
  an ingest of its own (ING-10), so without that the 400 spawns would each start
  a walk of the corpus tree, and a config directory that is not temporary lets
  the developer's real `roots` decide what is walked (D-14). Take the
  `SessionStart` payload from `tests/fixtures/hooks/session-start.json` and
  rewrite its `cwd` to a project key read out of the store's own `session_meta`
  - the row with the greatest `last_turn_at` and a null `parent_session_key`,
  which is the row the brief will name - so the brief renders and the last-turn
  lookup scans a real session rather than resolving to no project and going
  silent. Assert `SessionStart`'s stdout is non-empty on every run. Print
  nothing but the per-event percentiles: no brief text, no project key, no
  session key, no path out of that store, and no assertion message that would
  interpolate one, because this repository is public and that store is private
  transcripts. Wait for the detached ingests to exit before the temporary
  directory is dropped, the way `crates/verbatim/tests/hook.rs`'s `drain` does.
- **Verify:** `cargo test -p verbatim --test corpus` with `VERBATIM_TEST_CORPUS`
  unset passes and prints the skip line; `VERBATIM_TEST_CORPUS="$HOME/.claude"
  cargo test -p verbatim --test corpus -- --nocapture` prints a p50 and p99 per
  event and passes, with p99 under 10 ms for all four and `SessionStart`
  emitting a brief on all 100 of its runs, and its output contains no text taken
  from the corpus.

## Notes

- AC4 names `crates/verbatim/tests/hook.rs`'s existing p99 assertion. The
  measurement is placed in `corpus.rs` instead, asserting the same budget over
  the same four events and the same 100 runs, because `hook.rs`'s `hook()`
  harness is shared with the kill and detach tests and re-pointing it at the
  real tree would change what those measure as well. The property AC4 and
  Success Criterion 3 name - `SessionStart` inside the 10 ms p99 on the real
  corpus rather than on a fixture - is what task 2 asserts.
- Task 2's gated arm is slow: it pays for one full ingest of the real tree
  before it measures anything, which the existing test already reports as
  `pass took ...`. Run it once, not in a loop.
- `$HOME/.claude` on this machine is a real Claude config directory with a
  `projects` tree, so the gate resolves; `testkit::corpus_dir` asserts that
  shape and fails loudly if it is ever pointed at the tree itself.
