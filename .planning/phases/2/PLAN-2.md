---
phase: 2
plan: 2
requirements: [INST-09]
files:
  - crates/terminus/src/cmd/spawn.rs
  - crates/terminus/src/cmd/install/binary.rs
  - crates/terminus/src/cmd/install/targets.rs
  - crates/terminus/src/cmd/install/json_file.rs
  - crates/terminus/src/cmd/doctor.rs
  - crates/terminus/src/cmd/install/mod.rs
  - crates/terminus/src/cmd/uninstall.rs
  - npm/
  - .gitignore
---

# Phase 2: Linux and macOS Only, No npm - Plan 2

## Goal

The `terminus` binary's source has one platform shape. The hook's detached spawn, install's paths, and every fix that doctor, install and uninstall print are Unix only, and the npm package is gone from the repository.

## Must be true when done

- The detached ingest still leaves the hook's process group and parentage. The hook tests that prove it pass, and `spawn.rs` names no Windows creation flag.
- `terminus install` still places the binary at `$TERMINUS_BIN_DIR/terminus`, or `~/.local/bin/terminus` without the override, with mode 0755, copied from `std::env::current_exe()`. `install/binary.rs`'s docs name neither npm nor `%LOCALAPPDATA%`.
- `terminus doctor`, `terminus doctor --json` and `terminus uninstall` print no `attrib`, `del` or `.exe` fix and no mention of Windows. `doctor --json` carries the same top-level and `data` keys it did before this plan.
- `npm/` no longer exists, `.gitignore` has no npm block, and `git grep -n -i npm -- crates docs Cargo.toml .gitignore` prints nothing.
- `git grep -nE` with the AC2 pattern from Context, run over `crates/terminus/src`, prints nothing.

## Context

- Plan 2 of 4. The four plans touch disjoint files and none needs another's result. This plan's source changes do not depend on plan 1's refusal: each removed arm was already compiled out on Linux.
- Binding decisions: D-04 (Windows arms go; Unix guards go and their code stays), D-08 (`binary.rs` keeps copying from `current_exe()` and changes only the npm and `%LOCALAPPDATA%` doc text, besides its D-04 guards), D-12 (doctor gets no platform check and keeps its `--json` key set), D-02 (the `uninstall.rs` message goes; claude-mem history in comments stays), and D-13 (no CI configuration exists to change).
- The AC2 pattern: `'cfg\(windows\)|cfg!\(windows\)|target_os = "windows"|cfg\(not\(windows\)\)|cfg\(unix\)|cfg\(not\(unix\)\)|cfg\(all\(test, unix\)\)|APPDATA|USERPROFILE|\.exe"'`.
- The comment rule is plan 1's: reword a comment that names Windows, a Windows API or path form, or counts platforms. Keep claude-mem history, namely the exec-form section of `install/targets.rs` and the doc on `doctor.rs::on_path`. Keep Claude Code's own behaviour, namely the exec-form comment in doctor's wiring check that says a shell-form entry "runs through a shell on Windows".
- Out of scope: what install copies once versions sit behind the stable path (phase 4), and `--yes` and the confirmation step (phase 3).
- `crates/terminus/tests/hook.rs` has a known ETXTBSY flake at its spawn-the-copied-binary sites under parallel tests. It predates this phase (CAPTURE.md, phase 5 item). A failure with that errno is the known flake, not a regression from this plan; report it rather than chasing it.

## Tasks

### Task 1: The detached spawn is a new process group and nothing else

- **Files:** crates/terminus/src/cmd/spawn.rs (anchor: the `std alone (D-04)` module doc section, `spawn_from`)
- **Action:**
  - Delete `DETACHED_PROCESS` and `CREATE_NO_WINDOW` together with their `cfg(windows)` guards and the "Windows: no console" doc line.
  - In `spawn_from`, remove `#[cfg(unix)]` and keep its block (the `std::os::unix::process::CommandExt` import and `process_group(0)` with its comment). Delete the `#[cfg(windows)]` block that sets `creation_flags`.
  - In the module doc's `std alone (D-04)` section, drop `windows-sys` from the "No `libc`, no `nix`, no `windows-sys`" list. Also drop the `std::os::windows::process::CommandExt::creation_flags` link, so `process_group` alone is named as the mechanism. In the comparison with `ingest::lock`, drop `LockFileEx`, leaving `flock`. The startup-floor sentence stays.
  - The `# Two kills, not one (D-03)` section and the double fork are unchanged.
- **Verify:**
  - `git grep -n -i -E 'windows|LockFileEx|cfg\(' -- crates/terminus/src/cmd/spawn.rs` prints nothing.
  - `cargo test -p terminus --all-features --test hook` passes, including `the_working_process_leaves_the_group_and_the_parentage_of_the_process_that_spawned_it` and `the_ingest_survives_a_group_kill_and_a_descendant_sweep`.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 2: Install's paths and copy are Unix only

- **Files:** crates/terminus/src/cmd/install/binary.rs (anchor: the module doc sections `One path, forever (D-08, INST-02)` and `The source is this process (D-09)`, `file_name`, `platform_dir`, `place`, `copy_into`), crates/terminus/src/cmd/install/targets.rs (anchor: `home_dir`), crates/terminus/src/cmd/install/json_file.rs (anchor: `writing_through_a_symlink_updates_the_target_and_keeps_the_link`, `the_backup_carries_the_bytes_and_the_permissions`)
- **Action:**
  - **`binary.rs`, code (D-04).**
    - `file_name` returns `terminus` with no `cfg!(windows)` branch, and its doc drops "on this platform".
    - Delete the `#[cfg(windows)]` `platform_dir` (the `LOCALAPPDATA` one). Remove `#[cfg(not(windows))]` from the remaining `platform_dir` and keep its body.
    - In `copy_into`, remove `#[cfg(unix)]` and keep the block that sets mode 0755.
  - **`binary.rs`, docs (D-08, plus D-02's rule at one site).**
    - In `One path, forever`, the path is `~/.local/bin/terminus`. Drop the `%LOCALAPPDATA%\Programs\Terminus\terminus.exe` line, and keep the paragraph about `PATH`.
    - In `The source is this process (D-09)`, install copies the running binary, found through `std::env::current_exe`, so the process running `install` is the artifact to copy. Drop both mentions of npm and of its shim. `place` still reads `current_exe()`; phase 4 decides whether that changes.
    - On `place`, "on every platform. That is atomic everywhere, and on Windows it is also the only way to replace an executable that may be running" becomes this: the rename is atomic, and it replaces an executable that may be running, because the running image keeps the old inode and the name points at the new one. The paragraph about asking `occupant` again before the rename stays.
  - **`targets.rs` (D-04).** Delete the `#[cfg(windows)]` `home_dir` (the `USERPROFILE` one). Remove `#[cfg(not(windows))]` from the remaining `home_dir` and keep its body. The `Exec form, always (D-01, ING-10)` module section stays word for word, because it is claude-mem and Claude Code history (D-02).
  - **`json_file.rs` (D-04).** Remove `#[cfg(unix)]` from `writing_through_a_symlink_updates_the_target_and_keeps_the_link` and `the_backup_carries_the_bytes_and_the_permissions`, and keep both tests whole.
  - **Recalled:** v0.1.0 phase 4's D-08 set the `%LOCALAPPDATA%` stable path removed here. Its D-09 justified `current_exe()` by the npm shim; the source stays and only that justification goes.
- **Verify:**
  - `git grep -nE` with the AC2 pattern over these three files prints nothing.
  - `git grep -n -i -E 'npm|LOCALAPPDATA|windows' -- crates/terminus/src/cmd/install/binary.rs` prints only the `.windows(` slice call in `carries_marker`.
  - `git grep -n -i windows -- crates/terminus/src/cmd/install/targets.rs` prints only lines inside the `Exec form, always` module section.
  - `cargo test -p terminus --all-features --bin terminus` passes, which runs `this_build_carries_its_own_marker` and the `json_file.rs` tests. `cargo test -p terminus --all-features --test install --test uninstall --test doctor` passes.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 3: Doctor, install and uninstall print only Unix fixes

- **Files:** crates/terminus/src/cmd/doctor.rs (anchor: `claude_code`, the `data_directory` check's read-only arm, `removal`), crates/terminus/src/cmd/install/mod.rs (anchor: `occupied`), crates/terminus/src/cmd/uninstall.rs (anchor: `unplace`)
- **Action:**
  - **Before any edit,** build with `cargo build -p terminus` and record the key set as Verify describes. The comparison needs the pre-change output.
  - **`doctor.rs` (D-12).**
    - `claude_code` looks for `claude` with no `cfg!(windows)` branch.
    - The read-only `data_directory` arm's fix is `chmod u+w '<dir>'` with no `attrib` branch.
    - `removal` returns `rm -f '<path>'` with no `del` branch. Its doc "on the shell the user is at" becomes a plain statement of what it returns.
    - No check is added or removed, so the `--json` key set does not change. The exec-form comment in the wiring check and the claude-mem doc on `on_path` stay.
  - **`install/mod.rs`.** `occupied` builds its removal command as `rm -f '<path>'` with no `del` branch. The rest of its message is unchanged.
  - **`uninstall.rs` (D-02).** In `unplace`, delete the block that compares `current_exe()` with the stable path and prints "that file is this running program; on Windows it can only be removed after this process exits", along with its comment. The `could not be removed: {e}` line and the `false` return stay.
- **Verify:**
  - `git grep -n -E 'attrib|"del |\.exe"|cfg!\(windows\)' -- crates/terminus/src/cmd/doctor.rs crates/terminus/src/cmd/install/mod.rs crates/terminus/src/cmd/uninstall.rs` prints nothing. The pattern carries the closing quote on purpose, because doctor's `self.exe` field is not a Windows name.
  - `git grep -n -i windows` over the same three files prints only `doctor.rs`'s exec-form comment in the wiring check and its doc on `on_path`.
  - **Key set (AC6).** Use a scratch directory `$S` containing `claude/projects`. Set `HOME=$S`, `TERMINUS_BIN_DIR=$S/bin`, `TERMINUS_DATA_DIR=$S/data`, `TERMINUS_CONFIG_DIR=$S/config` and `CLAUDE_CONFIG_DIR=$S/claude`, then run `target/debug/terminus doctor --json | jq -c '[keys, (.data|keys)]'`. Record its output before the edit, rebuild after it, and confirm the line is identical.
  - **Printed output (AC6).** In that environment, run `terminus install --yes`, `terminus doctor`, `terminus doctor --json` and `terminus uninstall --yes`, each with `2>&1`. Pipe the combined output through `grep -n -i -E 'attrib|\bdel |\.exe\b|windows'`, which prints nothing. Then reinstall, create `$S/data` if it is absent, run `chmod a-w "$S/data"`, and confirm `terminus doctor` names `chmod u+w` as the `data_directory` fix. Restore the mode afterwards.
  - `cargo test -p terminus --all-features --test doctor --test install --test uninstall` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 4: The npm package leaves the repository

- **Files:** npm/, .gitignore
- **Action:**
  - Delete the whole `npm/` tree with `git rm -r npm`. That is the `terminus` package with its README, `package.json` and `bin/terminus.js`, the five platform packages under `npm/platforms/`, `npm/README.md` and `pack-local.sh`. All nine files are tracked and nothing ignored sits under `npm/`, so the directory goes with them.
  - In `.gitignore`, delete the npm block: the comment starting "npm packaging artifacts" and its four `npm/` patterns. The Cadence trace entry and `/target` stay.
  - Nothing else reads `npm/`. D-13 found no CI or release configuration, and `pack-local.sh` was the only build script.
- **Verify:**
  - `test ! -e npm && echo gone` prints `gone`.
  - `git grep -n -i npm -- crates docs Cargo.toml .gitignore` prints nothing. This includes task 2's `binary.rs` change.
  - `git status --short` for this task shows nine deleted files under `npm/` and a modified `.gitignore`, and nothing else.

## Notes

- `binary.rs`'s `place` doc is changed under D-02's rule even though D-08 names only the two npm and `%LOCALAPPDATA%` passages. D-08's "only" concerns the npm half, so that `place` keeps `current_exe()`. The `place` sentence describes Terminus's own Windows behaviour, which is what D-02 rewords.
- AC6 is checked by running the built binary here, not left to the suite. No existing test greps doctor's or uninstall's output for Windows text.
