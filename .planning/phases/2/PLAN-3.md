---
phase: 2
plan: 3
requirements: [INST-09]
files:
  - crates/terminus/tests/doctor.rs
  - crates/terminus/tests/hook.rs
  - crates/terminus/tests/install.rs
  - crates/terminus/tests/uninstall.rs
  - crates/terminus/tests/datamove.rs
  - crates/terminus/tests/corpus.rs
  - crates/terminus/tests/owner_only.rs
  - crates/terminus/tests/tighten.rs
  - crates/terminus/src/cmd/data.rs
  - crates/terminus/src/cmd/export.rs
  - crates/terminus/tests/backfill.rs
  - crates/terminus/tests/crash.rs
---

# Phase 2: Linux and macOS Only, No npm - Plan 3

## Goal

The `terminus` crate's process-level tests have one platform shape. The fixtures name the Unix binary and set only Unix variables, every guarded test and helper runs without its guard, and the crate's remaining comments stop describing Windows.

## Must be true when done

- `git grep -nE` with the AC2 pattern from Context, run over `crates/terminus/tests`, prints nothing.
- Every test and helper that sat behind `cfg(unix)` keeps its body and runs. The two `#[cfg(not(unix))] fn drain` stubs are gone, and `git grep -n 'fn drain' -- crates/terminus/tests` prints one line in `corpus.rs` and one in `hook.rs`.
- The fixtures in `doctor.rs`, `hook.rs`, `install.rs` and `uninstall.rs` name the binary `terminus` and set no `USERPROFILE`. `datamove.rs` sets neither `LOCALAPPDATA` nor `USERPROFILE`, and its `data_dir` helper mirrors only the Linux and macOS arms.
- `git grep -n -i -E 'windows|TerminateProcess|first-class' -- crates/terminus/tests crates/terminus/src/cmd/data.rs crates/terminus/src/cmd/export.rs` prints only `.windows(` slice calls.
- `cargo test --workspace --all-features` passes, and its `--list` count is the same after this plan as before it.

## Context

- Plan 3 of 4. The four plans touch disjoint files and none needs another's result. On Linux, every arm this plan removes was already compiled out or never taken, so the suite's behaviour does not change.
- Binding decisions: D-04 (every `cfg(unix)` guard goes and its code stays, tests and helpers included), D-05 (`datamove.rs` keeps its Linux and macOS arms, and `owner_only.rs`'s `cfg!(target_os = "macos")` arm in the bench stays), D-07 (what is removed from tests, which the phase summary lists), and D-02 (`data move` still skips the `LOCK` copy and still releases the lock before removing the source; only the comments change).
- The AC2 pattern: `'cfg\(windows\)|cfg!\(windows\)|target_os = "windows"|cfg\(not\(windows\)\)|cfg\(unix\)|cfg\(not\(unix\)\)|cfg\(all\(test, unix\)\)|APPDATA|USERPROFILE|\.exe"'`.
- The comment rule is plan 1's: reword a comment that names Windows, a Windows API or a Windows path form, or counts the platforms as three. Where Windows was the only reason for a behaviour, say it is kept and why that is harmless, and do not invent a Unix reason.
- Do not run `cargo fmt` over the workspace. `hook.rs` carries rustfmt diffs from earlier phases: CAPTURE.md's phase 4 item names two, and `cargo fmt --all --check` shows three today. They are not this plan's.
- `hook.rs` has a known ETXTBSY flake at its spawn-the-copied-binary sites under parallel tests (CAPTURE.md, phase 5 item). A failure with that errno is the known flake, not a regression; report it rather than chasing it.

## Tasks

### Task 1: Test fixtures name the Unix binary and set only Unix variables

- **Files:** crates/terminus/tests/doctor.rs (anchor: `Fixture::stable`, `Fixture::env`), crates/terminus/tests/hook.rs (anchor: the `exe` built in the hook fixture), crates/terminus/tests/install.rs (anchor: `Fixture::stable`, `Fixture::command_at`, the refusal assertion that a stranger at the stable path gets, `an_entry_of_ours_in_the_wrong_shape_is_repaired_rather_than_skipped`), crates/terminus/tests/uninstall.rs (anchor: `Fixture::stable`, `Fixture::command`), crates/terminus/tests/datamove.rs (anchor: the module doc, `Bench::data_dir`, `Bench::command`)
- **Action:**
  - **`terminus.exe` arms (D-07).** In `doctor.rs`, `install.rs` and `uninstall.rs`, `Fixture::stable` joins `terminus` with no `cfg!(windows)` branch. In `hook.rs`, the copied executable is named `terminus` the same way.
  - **`USERPROFILE` lines (D-07).** Delete the `.env("USERPROFILE", ...)` line from `doctor.rs`'s `Fixture::env`, `install.rs`'s `Fixture::command_at`, `uninstall.rs`'s `Fixture::command` and `datamove.rs`'s `Bench::command`. `HOME` stays in each of them.
  - **`LOCALAPPDATA` arms (D-07).**
    - In `datamove.rs`, delete the `.env("LOCALAPPDATA", ...)` line from `Bench::command`.
    - In `Bench::data_dir`, delete the `cfg!(target_os = "windows")` arm and keep the macOS and Linux arms (D-05).
    - In the module doc, the platform variables become `XDG_DATA_HOME` or `HOME` depending on the target.
  - **`install.rs`, the refusal assertion.** `said.contains("rm -f") || said.contains("del ")` becomes `said.contains("rm -f")`. The `del` disjunct accepted only the Windows command, which plan 2 removes from `install::occupied`. The assertion still passes before plan 2 lands, because on Linux the message already says `rm -f`.
  - **`install.rs`, the backslash comment.** In `an_entry_of_ours_in_the_wrong_shape_is_repaired_rather_than_skipped`, the `.replace('\\', "\\\\")` stays, as correct JSON escaping (D-02). Its comment becomes "JSON needs any backslash in the path doubled", with no "Windows separators".
- **Verify:**
  - `git grep -n -E 'USERPROFILE|APPDATA|\.exe"|cfg!\(windows\)|target_os = "windows"|"del ' -- crates/terminus/tests` prints nothing.
  - `cargo test -p terminus --all-features --test doctor --test hook --test install --test uninstall --test datamove` passes.

### Task 2: Guarded tests and helpers run without their Unix guards

- **Files:** crates/terminus/tests/corpus.rs (anchor: `drain`), crates/terminus/tests/hook.rs (anchor: `Proc`, `ps`, `await_process`, `drain`, `TREE`, `committed`, `descendants`, `sigkill`), crates/terminus/tests/doctor.rs (anchor: `Fixture::shell`, `a_missing_binary_is_a_problem_whose_printed_command_fixes_it`), crates/terminus/tests/install.rs (anchor: `a_stale_temporary_name_is_stepped_over_rather_than_followed`, `a_symlink_at_the_backup_name_is_not_written_through`), crates/terminus/tests/owner_only.rs (anchor: the inner `#![cfg(unix)]`), crates/terminus/tests/tighten.rs (anchor: the inner `#![cfg(unix)]`)
- **Action:**
  - **Guards (D-04, D-07).** Remove every `#[cfg(unix)]` and both inner `#![cfg(unix)]` attributes in these six files, and keep everything they guarded.
    - In `hook.rs`, that is the `Proc` struct, `ps`, `await_process`, `drain`, the `TREE` constant, `committed`, `descendants`, `sigkill`, and three tests: `the_working_process_leaves_the_group_and_the_parentage_of_the_process_that_spawned_it`, `reading_the_hooks_stdout_to_eof_returns_at_once_while_the_ingest_still_runs` and `the_ingest_survives_a_group_kill_and_a_descendant_sweep`.
    - In `corpus.rs`, it is `drain`.
    - In `doctor.rs`, it is the `shell` method and `a_missing_binary_is_a_problem_whose_printed_command_fixes_it`.
    - In `install.rs`, it is the two symlink tests.
    - In `owner_only.rs` and `tighten.rs`, it is the whole file.
  - **Stubs (D-07).** Delete `#[cfg(not(unix))] fn drain(_argv: &str) {}` from `corpus.rs` and from `hook.rs`. These two stubs, and the arms task 1 removes, are what the phase summary lists as removed from tests.
  - **Avoid** deleting or editing any test body, helper body or assertion. If something stops compiling once its guard is gone, that is a finding to report, not a reason to delete the code.
- **Verify:**
  - `git grep -nE` with the AC2 pattern over `crates/terminus/tests` prints nothing.
  - `git grep -n 'fn drain' -- crates/terminus/tests` prints exactly two lines, one in `corpus.rs` and one in `hook.rs`.
  - `cargo test --workspace --all-features` passes, its `--list` count equals the count before this plan, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 3: The terminus crate's comments stop describing Windows

- **Files:** crates/terminus/src/cmd/data.rs (anchor: the `D-16` module doc paragraph, the `LOCK` file comment in `run`, the comment above `drop(held)`), crates/terminus/src/cmd/export.rs (anchor: `file_name`), crates/terminus/tests/backfill.rs (anchor: `kill_and_reap`), crates/terminus/tests/crash.rs (anchor: the module doc, `kill_and_reap`)
- **Action:** Comments only, and no code changes. Apply the comment rule from Context at each site.
  - **`data.rs`, module doc.** "No `std` API detects a network mount on any of the three first-class targets" becomes "on Linux or macOS". The rest of the paragraph stays.
  - **`data.rs`, the `LOCK` comment (D-02).** Drop ", which Windows would refuse to read through a second handle". What remains is the reason that holds here: the lock file's bytes carry no meaning, so the destination gets a fresh empty one rather than a copy of the file this process holds locked. The owner-only paragraph below it stays.
  - **`data.rs`, above `drop(held)` (D-02).** Drop "and Windows will not unlink an open file". The lock is still released before `remove_dir_all`. Read `run` and state only what its order actually does, which is that the lock is held until the pointer names the destination. Where Windows was the only reason, say the order is kept.
  - **`export.rs`, `file_name`.** "the characters a filename may safely carry on all three platforms" loses the platform count. The allow-list and the reason that follows it stay.
  - **`backfill.rs` and `crash.rs`, `kill_and_reap`.** "`SIGKILL` on Unix, `TerminateProcess` on Windows: un-catchable either way" becomes `Child::kill` sending `SIGKILL`, which cannot be caught, and that is the point.
  - **`crash.rs`, module doc.** "`Child::kill` is `SIGKILL` on Unix and `TerminateProcess` on Windows" becomes "`Child::kill` is `SIGKILL`".
  - **Leave alone:** `cmd/json.rs`'s hostile-string test, which names a Windows path as transcript data.
- **Verify:**
  - `git grep -n -i -E 'windows|TerminateProcess|first-class|three platforms' -- crates/terminus/src/cmd/data.rs crates/terminus/src/cmd/export.rs crates/terminus/tests/backfill.rs crates/terminus/tests/crash.rs` prints nothing.
  - `git diff -U0` for this task shows only comment lines changed, in these four files.
  - `cargo test -p terminus --all-features --test datamove --test crash --test backfill` passes.

## Notes

- The `del` disjunct in `install.rs` is not in D-07's list. It goes under the phase goal ("remove every Windows branch") and AC6. Its removal belongs in the phase summary beside D-07's items.
- The phase summary's list of what was removed from tests (AC5, D-07): the two `#[cfg(not(unix))] fn drain` stubs, the four `terminus.exe` arms, the two `LOCALAPPDATA` lines in `datamove.rs` (the arm in `data_dir` and the variable in `command`), the four `USERPROFILE` lines, and the `del` disjunct. No test is removed, because none covered only Windows.
