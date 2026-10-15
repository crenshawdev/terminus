---
phase: 2
status: complete
completed: 2026-10-13
---

# Phase 2: Linux and macOS Only, No npm - Summary

Terminus now builds only for Linux and macOS: a `compile_error!` in core refuses every other system, every Windows arm and Unix-only guard in `crates/` is gone with the guarded code kept, `npm/` is deleted, and the docs and planning files say so.

## What shipped

- The compile-time refusal, guarded by `not(any(target_os = "linux", target_os = "macos"))` with the message "Terminus supports Linux and macOS only." - `crates/terminus-core/src/lib.rs:22-23`, pinned by `crates/terminus-core/tests/supported_systems.rs`
- Core with one platform shape: the Windows config, home and data-dir arms, the other-Unix data-dir arm and the no-op Windows `tighten` are deleted, case folding is macOS only, and every `cfg(unix)` guard in core source and tests is removed with its body kept - `crates/terminus-core/`
- The binary with one platform shape: the detached spawn is a new process group alone, install has one home and one stable path, and doctor, install and uninstall print only `chmod` and `rm -f` fixes, with doctor's `--json` key set unchanged - `crates/terminus/src/`
- The binary's tests name the `terminus` binary, set only Unix variables and run without Unix guards - `crates/terminus/tests/`
- `npm/` (ten tracked files) and the npm block in `.gitignore` removed
- `DESIGN-BRIEF.md`, `.planning/PROJECT.md` and `.planning/REQUIREMENTS.md` name Linux and macOS as the supported systems and say there is no npm channel; `.planning/CAPTURE.md`'s npm items, its Windows item, its Publish milestone and the icacls proposal are closed as no longer applicable; `CHANGELOG.md`'s `[Unreleased]` records both removals

Removed from tests (no test was removed whole, and none covered only Windows):

- the two `#[cfg(not(unix))] fn drain` stubs, in `tests/corpus.rs` and `tests/hook.rs`
- the four `terminus.exe` arms, in `tests/doctor.rs`, `tests/hook.rs`, `tests/install.rs` and `tests/uninstall.rs`
- the `LOCALAPPDATA` env line and the Windows arm of `data_dir` in `tests/datamove.rs`
- the four `USERPROFILE` lines, in `tests/datamove.rs`, `tests/doctor.rs`, `tests/install.rs` and `tests/uninstall.rs`
- the `del` alternative in the install refusal assertion in `tests/install.rs`

The suite listed 749 tests before the phase and lists 750 after it. The one added is the refusal test in `tests/supported_systems.rs`.

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | e67a7ab | refuse at compile time to build for anything but linux and macos |
| 1 | 2 | cb5c945 | give core one platform shape, with no windows arm and no unix guard |
| 1 | 3 | d40a8dc | give core comments reasons that hold on linux and macos |
| 1 | 4 | db47a9a | run every core test without a unix guard |
| 1 | report | 94a800e | plan 1 executor report |
| 2 | 1 | 51a6ee4 | detach the ingest with a new process group alone |
| 2 | 2 | 7129fdf | give install one home and one stable path |
| 2 | 3 | 5095949 | print only the rm and chmod fixes |
| 2 | 4 | 2404121 | drop the npm package and its ignore rules |
| 2 | report | d342439 | plan 2 executor report |
| 3 | 1 | 83fd049 | name the test binary terminus and set only the unix variables |
| 3 | 2 | e3470ce | run the unix-guarded tests and helpers without their guards |
| 3 | 3 | e28c7b3 | stop the terminus crate's comments describing windows |
| 3 | report | 0a3c90c | plan 3 executor report |
| 4 | 1 | b923ffb | describe Linux and macOS with no npm channel in the brief |
| 4 | 2 | 0acb4fd | state the platform and distribution as they now are in planning files |
| 4 | 3 | 600d3ee | close the capture list's npm, Windows and icacls items |
| 4 | 4 | a23ee90 | record the npm and Windows removals in the changelog |
| 4 | report | 4adfc45 | plan 4 executor report |

Plans 1 to 3 ran in parallel worktrees and plan 4 after them. Plan 1 and plan 4 fast-forwarded; plans 2 and 3 merged as ed43d98 and 5e3c719.

## Deviations

- [deviation] Plan 2 task 4: the plan counted nine tracked files under `npm/` and `git ls-files npm` listed ten. All ten were removed, since the goal is the whole tree (2404121).
- [deviation] Plan 3 task 1: two `hook.rs` latency-budget tests failed on the first run, at p99 11.92 ms and 11.08 ms against 10 ms, with a load average of 13.5 on 16 cores while three executors ran at once. The change only renames a path on a branch Linux already took. `--test hook --test-threads=1` passed 8 of 8 and the Verify re-run passed all five binaries (83fd049).
- [deviation] Plan 3 task 2: the plan's `git grep -n 'fn drain'` check prints three lines, not two, because it also matches the existing `fn drained` in `tests/decisions.rs:176`. `git grep -nw 'fn drain'` prints exactly the two kept helpers in `corpus.rs` and `hook.rs`, so both stubs are gone (e3470ce).
- [deviation] Plan 4 task 1 also narrowed the §6 normalization table's "case-folded on Win/mac" to macOS, a Windows mention the plan's `windows` grep could not see but AC7 covers (b923ffb).
- [deviation] Plan 4 task 2: the Verify expected `git diff --stat` over `.planning/reads.jsonl` and three other paths to print nothing, and it printed `reads.jsonl`, which the read-tracking hook appends to on every tool call. No commit staged it, and the committed range leaves the other three paths unchanged (0acb4fd).

## Open items

- AC4 needs a Windows machine: run `cargo build` of Terminus there with Rust and confirm the output contains "Terminus supports Linux and macOS only."
- AC5 in CONTEXT says the suite lists 749 tests. It lists 750, because AC3 adds a test. AC5 was left as written, and GitHub issue #6 tracks it.
- GitHub issues filed from this phase's reviews: #7 (doctor does not print that Linux and macOS are supported; D-12 says it gets no platform check), #8 (the plan-time CAPTURE diff check concern), and #9 (closing the whole Publish milestone also closed items that do not depend on npm or Windows: SECURITY.md and a private disclosure channel, the archive compatibility promise across 0.x, and the absence of release CI).
- The `hook.rs` latency budgets (10 ms p99) fail under heavy machine load, as seen with three executors running at once.
- Left as they are for the owner to decide: `DESIGN-BRIEF.md` §6 still says "no systemd/launchd/Task Scheduler"; `CAPTURE.md`'s phase 8 `data move` item about the `LOCK` file stays open, because D-02 keeps that behaviour; the file-name comments in `decision.rs` and `state.rs` listing `CON` and trailing dots stay, as does doctor's comment on how Claude Code runs a shell-form hook on Windows.

## Goal check

The commits deliver the goal. `npm/` is gone and `git grep -i npm -- crates docs Cargo.toml .gitignore` prints nothing (AC1). The AC2 pattern, covering `cfg(windows)`, `cfg!(windows)`, `target_os = "windows"`, `cfg(not(windows))`, `cfg(unix)`, `cfg(not(unix))`, `cfg(all(test, unix))`, `LOCALAPPDATA`, `APPDATA`, `USERPROFILE` and `.exe"`, prints nothing over `crates/` on the merged tree. The refusal sits at `crates/terminus-core/src/lib.rs:22-23` with its test in `tests/supported_systems.rs` (AC3). `cargo test --workspace --all-features` on the merged tree at 4adfc45 exits 0 with 750 passed and 0 failed. Plan 2's report records doctor's `--json` top-level keys unchanged and no `attrib`, `del`, `.exe` or Windows text in doctor, install or uninstall output (AC6, 5095949). `DESIGN-BRIEF.md:374` and `.planning/PROJECT.md:103` name Linux and macOS, and the only other Windows mentions in either file are about claude-mem (`DESIGN-BRIEF.md:22`, `DESIGN-BRIEF.md:119`, `.planning/PROJECT.md:80`) or say Windows was dropped (AC7). Two things stay open. AC4's real Windows build was not run, because this machine has no Windows target. AC5's literal 749 no longer matches the 750 the phase correctly produces.
