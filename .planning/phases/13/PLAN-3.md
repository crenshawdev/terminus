---
phase: 13
plan: 3
requirements: ["#38"]
files:
  - crates/terminus-core/src/feedback/mod.rs
  - crates/terminus-core/src/inject/decision.rs
  - crates/terminus-core/src/inject/state.rs
  - crates/terminus/src/cmd/hook.rs
  - crates/terminus-core/tests/feedback.rs
  - CHANGELOG.md
---

# Phase 13: CI and Data Integrity - Plan 3

## Goal

The phase goal: the test suite runs on GitHub for every push and pull request, and the archive and decision-log fixes (#15, #40, #38, the drain race, and #17's second case) land under it, so no known path writes wrong bytes into the archive or loses a decision's data.

This plan fixes the drain's two problems with half-finished files.

- **#38.** A decision or state write cut off at the hook's deadline leaves a `.tmp` file that nothing deletes, and the decision ones hold the whole prompt. Two comments say otherwise.
- **The drain race (not yet filed as an issue).** `write_file` reserves an empty `.json` and renames the record over it. A drain that reads the empty reservation and deletes the path after the rename has landed loses the record unread.

## Must be true when done

- An ingest pass deletes `decisions/.decision-*.tmp` and `injection/.inject-*.tmp` files older than the age limit and leaves younger ones, and the sweep puts nothing in `runs.error`.
- The sweep touches nothing else in either directory. A symlink, a directory or a file with any other name survives, whatever its age.
- A zero-length decision `.json` younger than the age limit survives a pass without being inserted, deleted or reported, and the record later renamed over it is inserted on the next pass. An older zero-length one is deleted and named as malformed, and a non-empty file that does not parse is still deleted and named at once.
- `hook.rs` no longer says the abandoned thread writes nothing, `state.rs` no longer says state files hold prompt text, and `decision.rs` no longer says the drain leaves `.tmp` files alone.

## Context

- Locked: D-16, D-17 and D-18. The rejected alternative to D-17 was publishing through a no-clobber hard link from the writer, so `write_file` stays as it is.
- The age value is the planner's call (CONTEXT flagged assumption). It is 10 minutes, reasoned in Task 1.
- The drain runs inside the ingest pass, after recovery and before the walk (`ingest/pass.rs`), and holds the ingest lock. Hooks write their files at the same time without the lock.
- Order: runs after plan 2, which shares `feedback/mod.rs`, `tests/feedback.rs` and `CHANGELOG.md`, and before plan 4, which shares `CHANGELOG.md`.
- `CHANGELOG.md` prose follows `~/.claude/john-voice.md`, and code comments follow its "Code comments" section. Read the file in full before writing.

## Tasks

### Task 1: Sweep the writers' abandoned temp files on every drain

- **Files:** crates/terminus-core/src/feedback/mod.rs, crates/terminus-core/src/inject/decision.rs, crates/terminus-core/src/inject/state.rs, crates/terminus-core/tests/feedback.rs, CHANGELOG.md
- **Action:** The drain deletes the writers' own temp files once their mtime is older than a fixed age (D-16). Candidates are files under `decision::DIR_NAME` whose names have the shape `decision::create_temporary` gives them (`.decision-` ... `.tmp`), and files under `state::DIR_NAME` whose names have the shape `state::create_temporary` gives them (`.inject-` ... `.tmp`). Nothing else in either directory is a candidate.
  - **Name match.** Take each prefix and suffix from the same place its writer's `create_temporary` builds the name from. A later change to a writer's pattern then cannot leave the sweep matching nothing.
  - **File type.** Check each candidate with `std::fs::symlink_metadata` and delete it only if it is a regular file. A symlink or directory with such a name is left alone.
  - **The age.** One constant in terminus-core, set to 10 minutes. The hook's deadlines live in the binary crate (`DRAIN_DEADLINE` 500 ms and `INJECT_DEADLINE` 50 ms, in crates/terminus/src/cmd/hook.rs) and the drain lives in core, which is why the constant is core's own. Ten minutes is more than a thousand times the 550 ms those deadlines give a hook process, plus its startup, so no hook still writing can own a file that old. It is also short enough that an abandoned copy of a prompt is gone by the first pass after ten minutes. A file whose mtime is in the future, or cannot be read, does not count as older and stays. The constant's doc comment gives this reasoning and names the two deadlines.
  - **When it runs.** On every drain, including one that finds no decision record (today's early return on an empty `found` must not skip it) and one whose insert fails.
  - **Not a failure.** The sweep is housekeeping. Nothing it deletes, or fails to delete, goes into `Drained::discarded`, so nothing reaches `runs.error`.

  Add an `[Unreleased]` entry to CHANGELOG.md. It says a decision or injection-state write cut off at the hook's deadline used to leave a `.tmp` file that nothing deleted, and that the decision ones held the whole prompt. It says the ingest now deletes those files once they are older than ten minutes (#38). Read `~/.claude/john-voice.md` in full before writing the entry.
- **Verify:** `cargo test -p terminus-core --features testkit --test feedback` passes, including a new test. In both `decisions/` and `injection/`, it plants four things:
  - a writer-named `.tmp` whose mtime is set 11 minutes back with `std::fs::File::set_modified`;
  - a writer-named `.tmp` with mtime now;
  - a symlink with a writer-shaped name pointing at an old regular file outside the directory;
  - an old regular file with some other name.

  After a pass, only the two old writer-named regular files are gone. The symlink and its target are both still there, the pass summary's `feedback.discarded` is empty, and the last `runs` row's `error` is null. The test also covers a pass where `decisions/` holds no `.json` at all. `cargo test -p terminus-core --lib inject::decision` passes, so `read_all` still never offers a `.tmp` as a record.

### Task 2: Leave a decision reservation that is still in flight for the next pass

- **Files:** crates/terminus-core/src/inject/decision.rs, crates/terminus-core/src/feedback/mod.rs, crates/terminus-core/tests/feedback.rs
- **Action:** Fix the race on the drain side (D-17). A zero-length `.json` whose mtime is younger than Task 1's age is a reservation still in flight. It is not inserted, not deleted and not reported, so the next pass reads the record renamed over it. An older zero-length `.json` is deleted and named as malformed, as today. A non-empty file that does not parse is still deleted and named at once.
  - **Where the facts come from.** The length and age that classify a file must describe the same file the bytes were read from. Take them from the metadata of the open handle the read went through, never from a second lookup of the path. The rename can land between the read and a second lookup, and the path would then look like a non-empty file that did not parse, which gets deleted. That is the race this task closes.
  - **What the drain gets.** `read_all`, or `Found`, carries what the drain needs to tell the three cases apart.
  - **What it deletes.** The delete loop after the commit removes only files whose bytes were inserted or judged malformed.

  Update the doc comments of `read_all`, `Found` and `drain` to describe the in-flight case. `write_file` does not change.
- **Verify:** `cargo test -p terminus-core --features testkit --test feedback` passes, including new tests:
  - A zero-length `.json` with a record-shaped name and mtime now survives a pass and adds no `decisions` row and no `discarded` entry. A record is then written to a temp file and renamed over it, and the next pass inserts that record and removes the file.
  - A zero-length `.json` whose mtime is 11 minutes back is removed and named in `feedback.discarded`.

  `a_file_that_is_not_a_record_is_removed_and_named` still passes. `cargo test -p terminus-core --lib inject::decision` passes.

### Task 3: Correct the comments that #38 and the drain change make false

- **Files:** crates/terminus/src/cmd/hook.rs, crates/terminus-core/src/inject/state.rs, crates/terminus-core/src/inject/decision.rs
- **Action:** D-18. Correct three comments.
  - **hook.rs.** The `inject` doc says abandoning the worker thread is safe because nothing on it writes, creates a directory or takes the ingest lock. That is false. The thread saves the session state file (`State::save`, called from prompt.rs), and the prompt arm saves a decision file (`Decision::save`, which also creates `decisions/`). Say what is true: both writes go through a temp file and a rename, so an abandoned write leaves at most a `.tmp` that the ingest's drain deletes once it is old. The thread never writes SQLite and never takes the ingest lock (INJ-06).
  - **state.rs.** The `create_temporary` doc says the temp file holds quoted prompt text. It holds the same document the target does: turn ids, suppressions and the compaction flag, which are `State`'s fields. Keep the owner-only reasoning, stated so it is true for that content.
  - **decision.rs.** In `read_all`, the comment on skipping `.tmp` files says skipping here and deleting in the drain are two different answers. Say instead that the drain sweeps them once they are older than the age limit (Task 1), and that `read_all` skips them because a `.tmp` is never a record.

  Then grep both crates for any other comment that says a `.tmp` is never deleted, that the abandoned thread writes nothing, or that state files hold prompt text. Correct any such comment in these three files. Report any outside them rather than editing it. Follow the "Code comments" section of `~/.claude/john-voice.md`.
- **Verify:** `grep -n "nothing on that thread writes" crates/terminus/src/cmd/hook.rs` and `grep -n "quoted prompt text" crates/terminus-core/src/inject/state.rs` both print nothing, and the `.tmp` comment in `read_all` names the drain's sweep. `cargo build --workspace --all-features` passes, and so does `cargo test -p terminus-core --lib inject`. `git diff --stat` for this task lists only the three leased files, and every changed line is a comment.
