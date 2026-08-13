---
phase: 4
plan: 1
requirements:
  - ING-10
files:
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/spawn.rs
  - crates/verbatim/src/cmd/hook.rs
  - crates/verbatim/tests/hook.rs
  - tests/fixtures/README.md
  - tests/fixtures/hooks/session-start.json
  - tests/fixtures/hooks/user-prompt-submit.json
  - tests/fixtures/hooks/session-end.json
  - tests/fixtures/hooks/post-compact.json
---

# Phase 4: Hooks And Install - Plan 1 of 5 (the hook path)

**SEQUENTIAL: PLAN-1 -> PLAN-2 -> PLAN-3 -> PLAN-4.** All four write
`crates/verbatim/src/main.rs` and `crates/verbatim/src/cmd/mod.rs`; PLAN-2 and
PLAN-3 name the four event names this plan defines, and PLAN-4 spawns through
the helper this plan writes. Do not run them in parallel. PLAN-5 (npm) shares no
file with any of them and may run at any time.

## Goal

Claude Code can call `verbatim` on every one of its four transcript-affecting
events and pay nothing measurable for it: the hook returns to the harness in
single-digit milliseconds having written nothing, and the ingest it started is a
process the harness can no longer see, let alone kill.

## Must be true when done

- Feeding each of `SessionStart`, `UserPromptSubmit`, `SessionEnd` and
  `PostCompact` the exact JSON Claude Code writes on stdin exits 0 with empty
  stdout, at p99 under 10 ms wall over 100 runs (AC1).
- A caller that reads the hook's stdout to EOF gets an immediate empty read
  while the ingest is still running, so the child holds no handle the hook
  inherited.
- Killing the hook the way Claude Code kills one - SIGKILL to its process group,
  then SIGKILL to every descendant pid a `ps` walk finds - leaves the spawned
  ingest alive, and its pass commits (AC2).
- A hook whose stdin is malformed, empty, or never closed still exits 0 and
  still starts the ingest, because a hook that fails is a hook that blocks a
  prompt.
- `verbatim hook` with an event name install never writes exits 2.

## Context

- D-18 binds the work: all four hooks spawn the same argument-free
  `verbatim ingest` tree pass. `ingest::pass::run` already takes the lock before
  opening the store (`crates/verbatim-core/src/ingest/pass.rs:87-97`) and
  `crates/verbatim/src/cmd/ingest.rs:41-43` already maps `PassOutcome::LockHeld`
  to exit 0 with empty stdout, so concurrent spawns need nothing new.
- D-15 binds the output: nothing on stdout in this phase, no JSON, no
  `hookSpecificOutput`. Claude Code 2.1.231 validates any hook stdout that
  parses as JSON against the event name. The resume brief and prompt injection
  are phase 5 (INJ-01..INJ-06) and must not appear here.
- D-04 binds the mechanism: `std` alone. No `libc`, no `nix`, no `windows-sys`.
  `crates/verbatim-core/src/ingest/lock.rs:12-19` is the precedent for why.
- D-14 binds the input: exactly one line of JSON followed by EOF.
- Out of scope: reading any field out of the payload, any settings.json writing
  (PLAN-2), and any `PreCompact` handling (D-02).

## Tasks

### Task 1: A detached spawn that survives both of Claude Code's kills

- **Files:** crates/verbatim/src/cmd/spawn.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs
- **Action:** Add a `spawn` module to the binary crate that launches
  `std::env::current_exe()` with a given argument list as a fully detached
  child and returns as soon as the spawn call does, never waiting. All three
  stdio are `Stdio::null()`. On Unix set a new process group with
  `std::os::unix::process::CommandExt::process_group(0)`; on Windows set
  `std::os::windows::process::CommandExt::creation_flags` with
  `DETACHED_PROCESS | CREATE_NO_WINDOW` (0x00000008 | 0x08000000). `std` alone,
  no new dependency (D-04) - every workspace dependency in the root
  `Cargo.toml` carries a comment justifying its startup cost against a measured
  0.408 ms floor, and this path is the floor's whole reason to exist.
  The environment is inherited unchanged, which is how `VERBATIM_DATA_DIR`,
  `VERBATIM_CONFIG_DIR` and `CLAUDE_CONFIG_DIR` reach the child.
  A new process group is not sufficient on its own (D-03): Claude Code 2.1.231
  kills a timed-out hook with `process.kill(-pid)` AND then walks the descendant
  pids it parsed out of `ps` output, so a child still parented to the hook is
  found by the second mechanism whatever its process group. Close that by making
  the spawned process reparent before any post-kill `ps` snapshot can name it:
  the helper marks the child through an environment variable it sets on the
  spawn, and the binary, on startup and before argument parsing, notices that
  mark, clears it, re-spawns itself with the same arguments through this same
  helper, and exits 0 - so the process doing the work has an init parent within
  microseconds and appears in no descendant walk rooted at the hook. Choose the
  variable name and put the startup check in `main.rs` ahead of the `lexopt`
  parser; keep it undocumented in `USAGE`, because it is a mechanism and not a
  command. Any other mechanism that leaves the working process outside both the
  hook's process group and the hook's descendant set satisfies this task.
- **Verify:** `cargo test -p verbatim --test hook` includes a test that spawns
  `ingest` through the helper against a temp data dir and asserts, for the
  working process's pid, that `ps -o pgid= -p <pid>` prints that same pid and
  `ps -o ppid= -p <pid>` prints a pid that is neither the test process nor the
  spawning process. Windows (human-verify: needs a Windows machine, AC3): run
  `verbatim hook SessionEnd` from a Windows terminal with a payload on stdin and
  observe that no console window appears, and that Process Explorer shows the
  spawned `verbatim ingest` holding no handle whose name matches the parent's
  console or pipes.

### Task 2: `verbatim hook <event>` for the four events

- **Files:** crates/verbatim/src/cmd/hook.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs
- **Action:** Add a `hook` subcommand taking exactly one positional event name.
  Accept exactly `SessionStart`, `UserPromptSubmit`, `SessionEnd` and
  `PostCompact` and expose that set as a public constant in this module, so
  PLAN-2's install and PLAN-3's doctor name one list and cannot drift from what
  the binary answers to. `PostCompact`, never `PreCompact` (D-02): the
  `system`/`compact_boundary` record is appended after compaction, so a
  `PreCompact`-spawned ingest can never see the boundary it exists to record.
  Any other name is `Failure::Misuse` (exit 2) - install writes these entries, so
  an unknown one is a broken settings file and not a routine event.
  The order of work is: spawn the detached argument-free `ingest` first through
  Task 1's helper, then read stdin. A harness that never closes stdin must not be
  able to prevent the ingest from starting, and that ordering is the only thing
  that guarantees it. Then read one line from stdin and discard it: no field of
  the payload is consumed in this phase (D-15), and deserializing a shape phase 5
  owns would be inventing it a phase early. Reading it at all is what keeps the
  writer from seeing a closed pipe.
  Write nothing to stdout on any path, ever. Return exit 0 unconditionally once
  the event name is known - a stdin read error, a spawn failure, an unresolvable
  data directory each print one line on stderr and still exit 0, because a hook
  that exits non-zero is a hook that can block a prompt. Add `hook` to `USAGE` in
  `main.rs` and to the `dispatch` match beside the existing arms.
- **Verify:** `printf '{"session_id":"x"}\n' | verbatim hook SessionEnd` with
  `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` and `CLAUDE_CONFIG_DIR` pointed at
  temp directories exits 0 and prints zero bytes on stdout, and a `verbatim
  status` a moment later shows a `last run` where it showed `none`;
  `verbatim hook PreCompact` and `verbatim hook` with no event both exit 2;
  `printf 'not json' | verbatim hook SessionStart` and
  `verbatim hook SessionStart < /dev/null` both exit 0.

### Task 3: The four real payloads, the budget and the clean stdout

- **Files:** crates/verbatim/tests/hook.rs, tests/fixtures/hooks/session-start.json,
  tests/fixtures/hooks/user-prompt-submit.json, tests/fixtures/hooks/session-end.json,
  tests/fixtures/hooks/post-compact.json, tests/fixtures/README.md
- **Action:** Write one fixture per event, each a single line with a trailing
  newline, carrying the base fields Claude Code 2.1.231 builds for every hook -
  `session_id`, `transcript_path`, `cwd`, `permission_mode` - plus that event's
  extras (D-14): `SessionStart` adds `source`, `agent_type`, `model`,
  `session_title`; `UserPromptSubmit` adds `prompt` and `session_title`;
  `SessionEnd` adds `reason`; `PostCompact` adds `trigger` and
  `compact_summary`. Then a test binary that, for each event, feeds the fixture
  on stdin and asserts exit 0 and zero bytes of stdout, and measures p99 wall
  time over 100 runs against a 10 ms budget (AC1), printing the measured p99 per
  event so a regression names a number rather than a boolean.
  The same test proves no handle was inherited: run the hook with stdout as a
  pipe, read that pipe to EOF, and assert the read returns empty immediately
  while the spawned ingest is still running - a child holding the inherited
  write end would keep that read blocked until it exited.
  Every spawn must set `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` and
  `CLAUDE_CONFIG_DIR` at temp directories, following `crates/verbatim/tests/cli.rs`'s
  `Bench`: a spawn that sets only the data dir resolves the developer's real
  config and walks a live 2,000-file `~/.claude` tree. Note the new
  `tests/fixtures/hooks/` directory in `tests/fixtures/README.md`.
- **Verify:** `cargo test -p verbatim --test hook` passes and its output shows a
  measured p99 under 10 ms for each of the four events; deleting the
  `Stdio::null()` on stdout from Task 1's helper makes the pipe-to-EOF assertion
  fail.

### Task 4: The kill Claude Code actually performs

- **Files:** crates/verbatim/tests/hook.rs
- **Action:** Add a test that reproduces D-03's kill in full and proves the
  ingest survives it (AC2). Start the hook against a transcript tree large
  enough, or arrange the fault point
  `crates/verbatim-core/src/ingest/mod.rs`'s `stall_after_files`
  (`VERBATIM_FAULT_AFTER_FILES`, which needs the `testkit` feature) so the
  spawned ingest is provably mid-pass when the kill lands rather than already
  finished - a test that kills after the pass has committed proves nothing.
  Then, in this order: SIGKILL the hook's process group with `kill -KILL -<pgid>`,
  and separately build the descendant set the way Claude Code does, by parsing
  `pid ppid` pairs out of `ps -e -o pid=,ppid=` and breadth-first walking from
  the hook's pid, SIGKILLing every pid found. Assert the hook process is gone,
  the ingest process is still alive, and when it finishes the store holds the
  session rows and a `runs` row for the pass. Gate the test to Unix with
  `#[cfg(unix)]`; AC3 covers the Windows half as a human-verify on Task 1.
- **Verify:** `cargo test -p verbatim --features testkit --test hook` passes,
  including this test; removing the reparenting half of Task 1's helper (leaving
  only the new process group) makes it fail at the descendant sweep rather than
  at the group kill.

## Notes

- The `testkit` feature caveat from `.planning/CAPTURE.md` (phase 3) applies:
  a bare `cargo test --workspace` runs zero tests in a `#![cfg(feature =
  "testkit")]` binary and still reports a green run. Task 4's test needs the
  fault point, so either gate only that one test rather than the whole file, or
  state the required feature flag in the test's own module doc - do not leave
  `tests/hook.rs` in the shape where AC1's budget test silently does not run.
- Recalled from `.planning/CAPTURE.md`, phase 3: `cmd/json.rs`'s `Document::emit`
  panics with exit 101 on a closed stdout pipe. The hook path writes no stdout at
  all, so it does not touch that path - this is noted so that a later
  "hooks should emit JSON" reading of phase 5 does not inherit the bug silently.
