---
phase: 2
plan: 1
requirements: [INST-09]
files:
  - crates/terminus-core/src/lib.rs
  - crates/terminus-core/tests/supported_systems.rs
  - crates/terminus-core/src/config.rs
  - crates/terminus-core/src/store/open.rs
  - crates/terminus-core/src/owner_only.rs
  - crates/terminus-core/src/project.rs
  - crates/terminus-core/src/inject/decision.rs
  - crates/terminus-core/src/inject/state.rs
  - crates/terminus-core/src/store/snapshot.rs
  - crates/terminus-core/src/recall/excerpt.rs
  - crates/terminus-core/src/testkit.rs
  - crates/terminus-core/src/ingest/lock.rs
  - crates/terminus-core/tests/compaction.rs
  - crates/terminus-core/tests/config.rs
  - crates/terminus-core/tests/discover.rs
  - crates/terminus-core/tests/ingest.rs
  - crates/terminus-core/tests/inject_state.rs
  - crates/terminus-core/tests/pass.rs
  - crates/terminus-core/tests/snapshot.rs
  - crates/terminus-core/tests/store_open.rs
  - crates/terminus-core/tests/recall_golden.rs
---

# Phase 2: Linux and macOS Only, No npm - Plan 1

## Goal

`terminus-core` builds on Linux and macOS only. A build for any other system stops at compile time with "Terminus supports Linux and macOS only.", and the core crate has no Windows branch and no Unix guard left in its source or its tests.

## Must be true when done

- `crates/terminus-core/src/lib.rs` carries a `compile_error!` with the message "Terminus supports Linux and macOS only.", guarded by `#[cfg(not(any(target_os = "linux", target_os = "macos")))]`. `cargo test -p terminus-core --test supported_systems` fails if either the guard or the message is changed.
- On a Windows machine, `cargo build` of Terminus fails and its output contains "Terminus supports Linux and macOS only." (human-verify: needs a Windows machine with Rust; see Notes)
- `git grep -nE` with the AC2 pattern from Context, run over `crates/terminus-core`, prints nothing.
- Exclusions still compare paths case-insensitively on macOS and case-sensitively on Linux. The data and config directories resolve exactly as before on both systems, and no data-directory arm for any other Unix remains.
- Every core test that sat behind `cfg(unix)` still runs with its body intact. `cargo test --workspace --all-features` passes and lists 750 tests: the 749 measured before the phase plus the one refusal test.

## Context

- Plan 1 of 4. The four plans touch disjoint files and none needs another's result, so they can run in any order or in parallel. The numbering follows CONTEXT's suggested order, with the code removal and the compile-time refusal first and the docs last.
- Binding decisions: D-01 (the refusal and where it lives), D-02 (Windows-motivated behaviour stays and only its comments change), D-04 (every Windows arm goes; every Unix guard goes and its code stays), D-05 (the macOS and Linux splits stay two-way), D-06 (the refusal is proved by a source-scan test), D-11 (the module doc in `owner_only.rs` and the no-op `tighten` arm).
- The AC2 pattern, used by every grep below: `'cfg\(windows\)|cfg!\(windows\)|target_os = "windows"|cfg\(not\(windows\)\)|cfg\(unix\)|cfg\(not\(unix\)\)|cfg\(all\(test, unix\)\)|APPDATA|USERPROFILE|\.exe"'`. `APPDATA` also matches `LOCALAPPDATA`.
- The comment rule, which is D-02 applied as written: a comment is reworded when it names Windows, a Windows API or a Windows path form, or says a platform-specific test is "Unix only". It stays when it recounts claude-mem's Windows history, such as the doc on `project.rs::git_program`, or when it describes data inside a transcript, such as the CRLF comment in `observe/egress.rs`. A reworded comment keeps the behaviour and gives a reason that holds on Linux and macOS. Where Windows was the only reason, it says the behaviour is kept and why that is harmless, and does not invent a Unix reason.
- D-11 warns that "D-15" labels about 40 unrelated decisions in `crates/` comments. Never search for "D-15" to find what to change; this plan names every site.
- Do not run `cargo fmt` over the workspace. `tests/inject_state.rs`, `tests/pass.rs` and other files carry rustfmt diffs from earlier phases, and reformatting them is not this phase's work.

## Tasks

### Task 1: A build for any system but Linux and macOS stops at compile time

- **Files:** crates/terminus-core/src/lib.rs, crates/terminus-core/tests/supported_systems.rs
- **Action:**
  - **The refusal (D-01).** In `crates/terminus-core/src/lib.rs`, directly after the `//!` module doc and before the first `pub mod`, add `compile_error!("Terminus supports Linux and macOS only.");` under the attribute `#[cfg(not(any(target_os = "linux", target_os = "macos")))]`. Write the attribute and the macro call each on its own line, in that order, with nothing between them.
  - **Its comment.** One short comment above the pair. It says the refusal lives in core because the binary crate compiles core first and core uses `std::os::unix` without a guard, so a refusal in `main.rs` alone would never be reached. It also says the guard names Linux and macOS rather than `not(unix)` so that FreeBSD and other Unix systems are refused too. Like the code, the comment must contain no string the AC2 pattern matches, so write `not(unix)` and never `cfg(not(unix))`.
  - **The test (D-06), in a new file `crates/terminus-core/tests/supported_systems.rs`.**
    - One test with no `testkit` gate, so `cargo test -p terminus-core` always runs it. Follow `tests/no_network.rs`, which reads source through `env!("CARGO_MANIFEST_DIR")` for the same reason.
    - It reads `src/lib.rs` and finds the guard line and the `compile_error!` line, each compared as a whole trimmed line. It fails, naming what is missing, if either line is absent, if the macro line does not directly follow the guard line, or if the pair does not come before the first `pub mod` line.
    - The test file must not itself contain any string the AC2 pattern matches.
  - **Avoid** a `build.rs`, a second refusal in `main.rs`, and `target_os = "windows"` anywhere. D-01 places the refusal in core alone, and criterion 2 bars that string from `crates/`.
- **Verify:**
  - `cargo build --workspace` succeeds on this Linux machine, which shows the guard is false here.
  - `cargo test -p terminus-core --test supported_systems` passes.
  - Change the guard's predicate in `lib.rs` to `not(unix)`, and the test must fail. Restore it, then change one word of the message, and the test must fail again. Restore it, and the test passes.
  - `cargo test --workspace --all-features -- --list 2>/dev/null | grep -c ': test$'` prints 750.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 2: Core source keeps one platform shape

- **Files:** crates/terminus-core/src/config.rs (anchor: `fold`, `platform_config_dir`, `home_dir`), crates/terminus-core/src/store/open.rs (anchor: `tighten`, `narrow_directory`, `narrow_file`, `narrow`, `platform_data_dir`), crates/terminus-core/src/owner_only.rs (anchor: the module doc, `create_dir_all`, `options`, `mod tests`), crates/terminus-core/src/project.rs (anchor: `git_program`), crates/terminus-core/src/inject/decision.rs (anchor: `a_record_and_its_directory_are_owner_only`)
- **Action:**
  - **`config.rs`, case folding (D-05).** The `fold` that lowercases is guarded by `target_os = "macos"` alone. The `fold` that returns the string unchanged is guarded by `target_os = "linux"`, matching the arms of `store::open::platform_data_dir`, so there is no "elsewhere" case left (D-01). The doc above them says case-insensitive on macOS and case-sensitive on Linux, and keeps its `.planning/PROJECT.md` citation; plan 4 keeps macOS case folding in PROJECT.md's Compatibility constraint.
  - **`config.rs`, directories (D-04).** Delete the Windows `platform_config_dir` (the `APPDATA` one) and the Windows `home_dir` (the `USERPROFILE` one). Remove the `not(target_os = "windows")` guard from the remaining `platform_config_dir` and `home_dir`, and keep their bodies.
  - **`store/open.rs`, modes (D-04, D-11).** Remove `#[cfg(unix)]` from `tighten`, `narrow_directory`, `narrow_file` and `narrow`, and keep every body. Delete the no-op `#[cfg(not(unix))] fn tighten` and its "The Windows arm (D-15)" doc line.
  - **`store/open.rs`, data directory (D-01, D-05).** Delete the `target_os = "windows"` `platform_data_dir` (the `LOCALAPPDATA` one). Also delete the `not(any(target_os = "linux", target_os = "macos", target_os = "windows"))` arm and its comment: the refusal from task 1 makes it unreachable. The Linux and macOS arms keep their guards and bodies unchanged.
  - **`owner_only.rs` (D-04, D-11).**
    - Remove `#[cfg(unix)]` from the blocks in `create_dir_all` and `options`, and keep each block.
    - Change `#[cfg(all(test, unix))]` on `mod tests` to `#[cfg(test)]`.
    - In the module doc, delete the paragraph that begins "On Windows the equivalent of owner-only is an ACL", which says `terminus doctor` states the deferral. Also drop `windows-sys` from the "No `libc`, no `nix`, no `windows-sys`" list. The `std alone (D-01)` sentence and its two `std::os::unix` links stay.
  - **`project.rs` (D-04).** `git_program` looks for `git` with no `cfg!(windows)` branch. Its doc about the PATH-probing failure class is claude-mem history and stays word for word (D-02).
  - **`inject/decision.rs` (D-04).** Remove `#[cfg(unix)]` from the test `a_record_and_its_directory_are_owner_only` and keep the test whole. The comment at the rename in `save` is task 3's.
  - **Avoid** changing any Linux or macOS path, mode or message. On both systems this task changes which code compiles and nothing that runs.
- **Verify:**
  - `git grep -nE` with the AC2 pattern over these five files prints nothing.
  - `git grep -n 'target_os' -- crates/terminus-core/src` prints only lines in `lib.rs`, the two `fold` guards in `config.rs` (macOS and Linux), and the two `platform_data_dir` guards in `store/open.rs` (Linux and macOS).
  - `cargo test --workspace --all-features` passes, the `--list` count is still 750, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 3: Core comments stop describing Windows behaviour

- **Files:** crates/terminus-core/src/store/snapshot.rs (anchor: the module doc on the ISO-8601 basic format), crates/terminus-core/src/recall/excerpt.rs (anchor: `ELISION`), crates/terminus-core/src/inject/decision.rs (anchor: the close before the rename in `save`), crates/terminus-core/src/inject/state.rs (anchor: the close before the rename), crates/terminus-core/src/testkit.rs (anchor: `copy_rooted_fixture_into`), crates/terminus-core/src/ingest/lock.rs (anchor: the module doc, `impl Drop for IngestLock`), crates/terminus-core/src/config.rs (anchor: `is_filesystem_root`)
- **Action:** Comments only. No code changes, and every behaviour named here stays (D-02). Apply the comment rule from Context at each site.
  - **`store/snapshot.rs`.** Snapshot names stay ISO-8601 basic format. The reason becomes the one D-02 gives: the prune orders snapshots by name, and existing `terminus-<time>.db` files already use the basic format. Drop the claim that `:` is illegal in a filename because Windows is a first-class target.
  - **`recall/excerpt.rs`.** `ELISION` stays `...`. Drop the Windows console claim. The reason that holds on Linux and macOS is that a terminal's encoding is not Terminus's to choose, and a horizontal ellipsis arrives as mojibake under a non-UTF-8 locale.
  - **`inject/decision.rs` and `inject/state.rs`.** The file is still closed before the rename. Windows was the only reason for that order, so each comment says the order is kept and costs nothing, without inventing a Unix reason.
  - **`testkit.rs`.** Backslashes are still doubled. The reason becomes that the substitution lands inside a JSON string literal, where any backslash in the root would otherwise be an invalid escape. Drop "a Windows root".
  - **`ingest/lock.rs`.**
    - In the module doc, `File::try_lock` is `flock(2)` with `LOCK_EX | LOCK_NB`. Drop the `LockFileEx` half, and drop the clause about Windows byte-range locking and NTFS versus ReFS. The APFS and overlayfs reasons stay.
    - The "D-15 named exactly" phrase may stay as the `flock` half of that decision. It is this file's own lock decision, not the Windows ACL deferral (D-11).
    - In `impl Drop for IngestLock`, "on both platforms" becomes a statement that closing the file releases the lock, with no platform count.
  - **`config.rs`, `is_filesystem_root`.** Drop the Windows prefix example (`C:`, `\\server\share`). The doc keeps the rule (`RootDir` and nothing else) and its tie to `normalize` and `Config::excludes_path`, and makes no claim about `Path::parent` that is untrue on Unix.
  - **Leave alone:** the claude-mem PATH-probing doc on `project.rs::git_program`, the CRLF comment in `observe/egress.rs`, and the allow-list rationale in `decision.rs::file_stem` and `state.rs::file_name` (`CON`, a trailing dot), which argues for an allow-list and claims nothing about where Terminus runs.
- **Verify:**
  - `git grep -n -i -E 'windows|LockFileEx|NTFS|both platforms|first-class' -- crates/terminus-core/src` prints only the doc on `project.rs::git_program`, the CRLF comment in `observe/egress.rs`, and lines where `windows` is the slice method `.windows(` (for example in `recall/excerpt.rs`).
  - `git diff --stat` for this task lists only the seven files above, and `git diff -U0` for it shows only comment lines (`//` or `//!` or `///`) changed.
  - `cargo test --workspace --all-features` passes.

### Task 4: Core tests run without a Unix guard

- **Files:** crates/terminus-core/tests/compaction.rs (anchor: `a_failure_that_is_not_a_divergence_flags_no_session`), crates/terminus-core/tests/config.rs (anchor: `a_symlinked_branch_is_unresolved_rather_than_absent`), crates/terminus-core/tests/discover.rs (anchor: `every_path_begins_with_the_canonical_root_even_through_a_symlink`, `an_unreadable_directory_is_reported_and_the_rest_is_still_walked`), crates/terminus-core/tests/ingest.rs (anchor: `two_paths_to_one_transcript_produce_one_session`), crates/terminus-core/tests/inject_state.rs (anchor: `a_write_replaces_the_target_rather_than_rewriting_it_in_place`, `the_scratch_the_hook_writes_is_owner_only_and_so_is_what_holds_it`), crates/terminus-core/tests/pass.rs (anchor: `config_file`, `the_data_directory_and_the_lock_are_owner_only_from_the_first_acquire`), crates/terminus-core/tests/snapshot.rs (anchor: `a_snapshot_taken_mid_pass_opens_verifies_and_excludes_the_uncommitted_session`, `a_snapshot_and_its_directory_are_owner_only`, `a_snapshot_that_cannot_be_written_is_a_note_and_not_a_failed_pass`), crates/terminus-core/tests/store_open.rs (anchor: `a_fresh_store_and_its_sidecars_are_owner_only`, `a_wide_store_is_tightened_by_a_writable_open_and_by_nothing_else`), crates/terminus-core/tests/recall_golden.rs (anchor: `rooted`)
- **Action:**
  - **Guards (D-04, D-07).** Remove every `#[cfg(unix)]` in these test files and keep everything it guarded: the eleven on whole tests, plus the two inside `a_write_replaces_the_target_rather_than_rewriting_it_in_place` in `inject_state.rs`, on the `let before` binding and on the block that compares inodes. No test, helper or assertion is deleted.
  - **Comments that described a guard.**
    - `compaction.rs`: "Unix only: ... Windows paths are UTF-16 where the equivalent is an unpaired surrogate" becomes a statement that the test needs a path that is bytes and not text.
    - `inject_state.rs`: drop "It is unix-only because the number is; the rest of the case holds on every platform", since nothing about the test is conditional any more.
  - **Comments under the rule from Context.**
    - `pass.rs`, `config_file`: the TOML literal string stays. The reason becomes that a literal string takes the path's bytes as written, with no escape processing. Drop "a Windows path".
    - `snapshot.rs`, `a_snapshot_taken_mid_pass_...`: `File::try_lock` is `flock(2)` with `LOCK_EX | LOCK_NB`. Drop the `LockFileEx` half, and keep the claim that two open file descriptions conflict whether or not they belong to the same process.
    - `recall_golden.rs`, `rooted`: the escaped spelling is still replaced first. The reason becomes a root containing a backslash, which JSON doubles. Drop "a Windows root". The `forward_slashes` helper and its doc stay as they are (D-02).
- **Verify:**
  - `git grep -nE` with the AC2 pattern over `crates/terminus-core` prints nothing.
  - `git grep -n -i -E 'windows|LockFileEx|unix only|unix-only' -- crates/terminus-core/tests` prints only lines that call the slice method `.windows(`, and lines in `blob.rs` about its 64 KB block windows and the `windows_touched` helper.
  - `cargo test --workspace --all-features` passes, the `--list` count is still 750, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **AC5's count.** CONTEXT's AC5 says the suite lists 749 tests, the count measured before the phase. That is correct as a baseline (re-measured 2026-10-13: 749). AC3 and D-06 add one test, so the suite after this plan lists 750. CONTEXT's third flagged assumption anticipates a moving count, and the phase summary should account for the one test gained. Removing the `cfg(unix)` guards changes nothing, because those tests already run on Linux.
- **AC4 is a human check.** On a Windows machine with Rust, run `cargo build` in the repository root. The output must contain "Terminus supports Linux and macOS only." This machine has only the `x86_64-unknown-linux-gnu` std and no mingw compiler (D-06). CONTEXT's first flagged assumption also stands: only a real Windows build shows whether that message appears before or among the unresolved `std::os::unix` errors.
- **Comment sites beyond D-02's evidence list.** D-02 states its rule generally, and these sites fall under it without being in its evidence list: `config.rs::is_filesystem_root`, the module doc and `Drop` comment in `ingest/lock.rs`, and the test comments in `pass.rs`, `snapshot.rs`, `recall_golden.rs`, `compaction.rs` and `inject_state.rs`.
- **Recalled.** v0.1.0 phase 7's D-15 (`owner_only.rs`'s module doc and the no-op `tighten` arm) is closed here in code. Plan 4 closes the icacls proposal in `CAPTURE.md`.
