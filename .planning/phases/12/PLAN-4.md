---
phase: 12
plan: 4
requirements: ["#35", "#10", "#25"]
files:
  - crates/terminus-core/src/inject/brief.rs
  - crates/terminus/tests/lock_race.rs
  - docs/json-shapes.md
  - CHANGELOG.md
---

# Phase 12: Quick Issue Fixes - Plan 4

## Goal

The phase goal: twelve open GitHub issues that are small enough to fix inside one function or one doc section are fixed and closed before the release is announced, which cuts the open list from 39 to 27. #17 gets its type check here, and its second case moves to phase 13.

This plan does the three cleanups and closes the phase. `brief.rs` calls the shared `chars` and `clip` (#35). The lock test that claims a process death is renamed and points to the crash harness (#10). `docs/json-shapes.md` says which `sessions` and `show` fields can be null (#25). The `[Unreleased]` section of `CHANGELOG.md` then gets its Fixed entries for the changes users will see, which plans 1 and 3 made.

## Must be true when done

- `brief.rs` defines no `chars` or `clip` of its own, and the brief it produces is unchanged.
- No test is named `the_lock_is_released_by_process_death`. The renamed test's comment names `killing_ingest_anywhere_leaves_a_store_that_converges` in `crates/terminus/tests/crash.rs`, and `a_pass_that_dies_mid_walk_still_leaves_its_runs_row` keeps its name.
- `docs/json-shapes.md` marks every `sessions` and `show` field D-14 lists as nullable, and says when each one is null.
- The `[Unreleased]` section of `CHANGELOG.md` has a `### Fixed` list with one entry for each change D-15 names, and no others.
- `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-features --all-targets -- -D warnings` and `cargo fmt --all --check` are clean.

## Context

- D-12 to D-15 in `.planning/phases/12/CONTEXT.md` bind this plan exactly. Only the one lock test is renamed, and the `pass.rs` test keeps its name (D-13). No JSON field is added or removed.
- This plan runs after plans 1 to 3. It shares `docs/json-shapes.md` with plan 3, and its changelog task describes what plans 1 and 3 changed, so it reads their final code.
- The doc and changelog prose is John's. Read `~/.claude/john-voice.md` in full before writing either, every time, as `~/.claude/CLAUDE.md` requires.

## Tasks

### Task 1: Call the shared chars and clip from brief.rs (#35)

- **Files:** crates/terminus-core/src/inject/brief.rs (the private `chars` and `clip`)
- **Action:** Delete `brief.rs`'s private `chars` and `clip`, along with their doc comments. Make every call site in the file use the `pub(crate)` `chars` and `clip` in `inject/mod.rs`, which `brief.rs` reaches as `super::`. The two pairs count and cut the same way, so the brief's text and its budget arithmetic do not change. Keep the `excerpt` import, because `excerpt::Reads` and `excerpt::of_record_with` still use it. Recalled prior art: phase 5 left this pair behind as one pending deletion (`v0.1.0/phases/5/SUMMARY.md`, phase 5).
- **Verify:** `grep -nE 'fn (chars|clip)\b' crates/terminus-core/src/inject/brief.rs` prints nothing. `cargo test -p terminus-core --all-features --test inject_brief --test inject_compaction --test inject_prompt` and `cargo test -p terminus --all-features --test brief --test inject` pass. Clippy and fmt are clean.

### Task 2: Rename the lock test that claims a process death (#10)

- **Files:** crates/terminus/tests/lock_race.rs (`the_lock_is_released_by_process_death`)
- **Action:** Rename `the_lock_is_released_by_process_death` to a name that says what its body checks. A second `try_acquire` fails at once while the guard is held, and the lock is free again once the guard is dropped. The name must not claim that a process is killed or dies, because the test starts and kills nothing. Rewrite its doc comment to keep the reason the lock is an OS lock and not a PID file (`DESIGN-BRIEF.md:135`). It should say this test covers the guard being dropped inside one process. It should name `killing_ingest_anywhere_leaves_a_store_that_converges` in `crates/terminus/tests/crash.rs` as the test where real ingest processes are killed and the next ingest still runs. Do not change the test's body, and do not rename `a_pass_that_dies_mid_walk_still_leaves_its_runs_row` in `crates/terminus-core/tests/pass.rs` (D-13).
- **Verify:** `grep -rn the_lock_is_released_by_process_death crates` prints nothing. `grep -n killing_ingest_anywhere_leaves_a_store_that_converges crates/terminus/tests/lock_race.rs` finds the new comment. `grep -n 'fn a_pass_that_dies_mid_walk_still_leaves_its_runs_row' crates/terminus-core/tests/pass.rs` still matches. `cargo test -p terminus --all-features --test lock_race` passes. Clippy and fmt are clean.

### Task 3: Mark the nullable sessions and show fields in json-shapes.md (#25)

- **Files:** docs/json-shapes.md (the `show` and `sessions` sections)
- **Action:** Mark each field below as able to be null, in the bullet style the `search` section uses ("`ts` and `project` are null for a turn or a session that has none"). Each bullet says when the field is null, read from the code that emits it.
  - In `sessions`: `project`, `project_pre_worktree`, `branch`, `first_turn_at`, `last_turn_at` and `watermark`.
  - In `show`: `tool_name`, `ts`, `project`, `body` and `context`.
  - Inside `context`: `turns[].tool_name`, `turns[].ts`, `continues_from` and `reason`.

  The evidence D-14 cites is `crates/terminus/src/cmd/sessions.rs` (`Detail` and `value`), `crates/terminus/src/cmd/show.rs`, `crates/terminus-core/src/recall/get.rs` (`Record`) and `crates/terminus-core/src/recall/context.rs` (`ContextTurn` and `Window`). `body` and `context` already have bullets that mention null. Extend those rather than adding a second bullet for the same field. Group fields that are null for the same reason into one bullet, as the `search` section does. Mark no field D-14 does not list, and change neither JSON example. Read `~/.claude/john-voice.md` in full first.
- **Verify:** In the `sessions` and `show` sections of `docs/json-shapes.md`, each of the fifteen fields above appears in a bullet that says when it is null, and no other field is called nullable. You can check this with `perl -ne 'print if /^### .show./../^### .status./' docs/json-shapes.md`. `cargo test -p terminus --all-features --test cli` passes unchanged.

### Task 4: Write the Fixed entries in CHANGELOG.md and run the phase gate

- **Files:** CHANGELOG.md (the `[Unreleased]` section)
- **Action:** Under `## [Unreleased]`, add a `### Fixed` list with one entry for each change D-15 names, and no others:
  1. `terminus mcp` no longer agrees to protocol revision `2025-03-26`, and a client asking for it is answered `2025-06-18`.
  2. A request whose id is not an integer that fits is refused.
  3. Duplicate `recall_get` ids are removed before the 25-id cap.
  4. `terminus status` no longer fails with "database is locked" when it opens a store another process is creating at the same moment.
  5. `terminus status` reports the file committed after `terminus ingest <path>` on one transcript.
  6. `terminus reindex` reports its preserved count.
  7. `terminus verify` fails an evicted session that still has block rows.

  Write each entry the way the `[0.1.1]` Fixed entries are written: what went wrong for a user, and what happens now. Take every specific from the code plans 1 and 3 committed, never from this plan's wording. That covers the stderr line `reindex` prints, the key name `preserved`, and the error code and null id of a refused request. Item 5 also says that `runs` rows written before the fix keep their 0. Leave every released section untouched. Read `~/.claude/john-voice.md` in full first.
- **Verify:** `git diff CHANGELOG.md` shows additions only, all between `## [Unreleased]` and `## [0.1.1]`: one `### Fixed` heading and seven entries matching the seven items above. `cargo test --workspace --all-features` passes. `cargo clippy --workspace --all-features --all-targets -- -D warnings` and `cargo fmt --all --check` are clean.

## Notes

- **How the phase is closed.** D-15 and AC7 belong to the step that opens the pull request, not to an executor task. One PR from `hardin` closes the twelve issues. Its body has a `Fixes #N` line for each of #10, #14, #20, #25, #28, #29, #30, #33, #35, #39, #41 and #42, as PR #48 closed #13. #17 gets a reference line and no `Fixes`, so it stays open for its deferred second case in phase 13. AC7 is human-verify: after the merge, GitHub shows the twelve issues closed by that PR and #17 still open with a reference to it.
- **Why the plans run in order.** The four plans follow the four areas CONTEXT names. Plans 1 to 3 share no file. This plan shares `docs/json-shapes.md` with plan 3, and Task 4 describes what plans 1 and 3 changed. So the phase executes sequentially, in plan order.
- `docs/json-shapes.md` and `CHANGELOG.md` are docs John reviews himself. Open each one in RustRover once it is written.
