---
phase: 4
plan: 1
requirements: [INST-12]
files:
  - crates/terminus/src/cmd/install/binary.rs
  - crates/terminus/src/cmd/install/mod.rs
  - crates/terminus/src/cmd/spawn.rs
  - crates/terminus/src/cmd/uninstall.rs
  - crates/terminus/tests/install.rs
  - crates/terminus/tests/uninstall.rs
---

# Phase 4: One-Command Installer - Plan 1

## Goal

`terminus install` no longer copies itself over the stable path. It stores the running build as a version at `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus` and points `~/.local/bin/terminus` at that file with a symbolic link. A local build is therefore installed with `cargo build --release && target/release/terminus install`. Uninstall removes the link and leaves the versions.

## Must be true when done

- **Install from a build outside the versions folder lands in the layout.** Run from the cargo-built test binary, install leaves `<stable folder>/terminus` a symbolic link to `$HOME/.local/lib/crenshawdev/terminus/versions/<the build's version>/terminus`. That file holds the build's bytes and is executable. The four hook entries and the MCP entry still name the stable path, never the version file.
- **Ownership is still decided by the marker.** A plain file at the stable path that carries the marker, like an install from before this phase, is replaced by the link. A file without the marker is refused: non-zero exit, the file unchanged, and no version folder written.
- **A rebuild at the same version replaces its file, and the same bytes are left alone.** Install from a build with the same version and different bytes replaces the version file, and the link still points at it. Install from a build whose bytes are already there leaves the version file's inode unchanged.
- **`<stable path> install` changes neither the link nor the version file.** Both keep their inodes and the link keeps its target. That also holds when the link points into a version folder whose name is not the build's own version, which is what `install.sh` makes when its manifest names the version.
- **Uninstall removes the link and keeps the versions.** Afterwards nothing is at the stable path, not even a dangling link. Every file in the versions folder is still there, and the output prints the versions folder's path.
- `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- This is plan 1 of 3. Plan 2 (`install.sh`) depends on task 2 here, and plan 3 (docs) describes what plans 1 and 2 built.
- Decisions that bind this plan: D-01 (layout), D-02 (install stores itself as a version), D-03 (same version, different bytes), D-04 (old versions are never deleted), D-05 (the marker through a link needs no Rust change), D-11 (uninstall), D-12 (doctor is not changed). D-02 replaces `binary::place`'s copy over the stable path, which phase 2 D-08 left open.
- Patterns to keep: every write goes through a temporary created exclusively in the destination folder, then a rename (`install::create_temporary`'s doc says why). Every refusal still runs before the first write (`install::run`).
- Out of scope: `install.sh` (plan 2), the docs (plan 3), pruning old versions, any change to `doctor.rs` (D-12), and any environment variable for the versions folder.
- Run `rustfmt --edition 2021` on the files you touch, and only on those. `cargo fmt --all --check` already reports diffs in files outside this phase.

## Tasks

### Task 1: Install stores this build as a version and links the stable path to it

- **Files:** crates/terminus/src/cmd/install/binary.rs (anchor: the module doc, `place`, `copy_into`, `stable_path`, `platform_dir`), crates/terminus/src/cmd/install/mod.rs (anchor: the module doc, `run`, `start_backfill`), crates/terminus/src/cmd/spawn.rs (anchor: the doc comment on `detached_from`), crates/terminus/tests/install.rs (anchor: `Fixture`, `install_places_this_build_at_the_stable_path`, `a_foreign_binary_at_the_stable_path_stops_everything`, `a_stale_temporary_name_is_stepped_over_rather_than_followed`)
- **Action:**
  - **The versions folder (D-01).** It is `$HOME/.local/lib/crenshawdev/terminus/versions`.
    - Resolve it from `HOME` whether or not `TERMINUS_BIN_DIR` is set. `TERMINUS_BIN_DIR` moves only the stable folder. Every test fixture that runs install already points `HOME` at its own temporary root, so no test can reach the developer's real versions folder.
    - A missing or empty `HOME` fails the way `platform_dir` fails.
    - Resolve it in `run`'s refusal block, before the first write.
    - Add no environment variable for it.
  - **Storing the build (D-02, D-03).**
    - The version folder's name is `env!("CARGO_PKG_VERSION")`. That is what `--version` prints and what doctor's `version_at` compares.
    - Copy `current_exe()` into a temporary created exclusively in that folder. Make it executable (0755) and flush it before the rename, as `copy_into` does today. Then rename it to `terminus` in that folder.
    - When a regular file there already has the same bytes, leave it alone and write nothing.
    - When the file has different bytes, the rename replaces it. A running process keeps the old inode.
    - Compare the bytes by streaming both files, the way `carries_marker` streams. Do not read two whole binaries into memory.
    - Never write through a symbolic link that sits at the version file's name.
  - **The link (D-01).**
    - Create a symbolic link to the version file's absolute path at a temporary name in the stable folder. Never follow or reuse anything already at that name. A stale name is stepped over, as `create_temporary` does for files.
    - Ask `occupant` again just before the rename, and refuse a foreign occupant, as `place` does today.
    - Rename the link over the stable path, and remove the temporary on any failure.
  - **Order.**
    - The `occupant` check stays first in `run`, before the version file is written, so a refused install creates no version folder.
    - A plain file carrying the marker is `Ours` and is replaced by the link. `occupant` already follows links (D-05), so it does not change.
  - **What the settings name.** The hook entries and the MCP entry keep naming the stable path: `targets::add_hooks` and `register` get `stable` as they do now. They never name the version file. Its path changes with every version, and the design writes the hooks once and never rewrites them.
  - **Old code.** `place` is replaced by this. Remove whatever loses its last caller, because clippy runs with `-D warnings`.
  - **The receipt.** The `binary` line names the stable path and the version file it links to. The module doc says the receipt lists every file install touched, and the version file is now one of them.
  - **Comments.**
    - Rewrite the binary.rs module doc ("One path, forever", "The source is this process") and the mod.rs module doc so they describe the link and the versions folder.
    - Rewrite the comment in `start_backfill` and the doc on `spawn::detached_from`. Install no longer renames a file over the one this process runs from, so the reason those comments give is no longer true.
    - `start_backfill` keeps spawning from the stable path, because the stable path is the build every hook entry runs. Only the stated reason changes.
  - **Tests in tests/install.rs.**
    - `install_places_this_build_at_the_stable_path` asserts the link, its target, and the target's bytes.
    - Add a test for a plain file carrying the marker: a copy of the test binary written as a regular file at the stable path. After install, the stable path is the link.
    - Add a rebuild test. A copy of the test binary with bytes appended still runs and reports the same version. Installing from that copy replaces the version file. A second run from the same copy leaves the version file's inode unchanged.
    - `a_foreign_binary_at_the_stable_path_stops_everything` also asserts that the versions folder does not exist.
    - `a_stale_temporary_name_is_stepped_over_rather_than_followed` plants its victims at the names install now builds from its pid: in the version folder and in the stable folder. It still asserts that no victim changes.
- **Verify:** `cargo test -p terminus --test install` passes, and it checks all of the following:
  - After one install from the test binary, `symlink_metadata` of the stable path is a symbolic link. Its `read_link` is `<root>/.local/lib/crenshawdev/terminus/versions/<CARGO_PKG_VERSION>/terminus`. That file's bytes equal the test binary's, and its mode has the executable bits.
  - Every hook entry's `command`, and the logged `add-json` entry's `command`, is the stable path.
  - A marked plain file at the stable path ends up as the link.
  - A foreign file is refused with its bytes unchanged and no versions folder.
  - The appended-bytes copy replaces the version file's bytes, and the link target is unchanged.
  - A rerun from that copy leaves the version file's inode unchanged.

  `cargo test -p terminus --test doctor --test uninstall` still passes.

### Task 2: Install run from the link's own target touches neither the link nor the file

- **Files:** crates/terminus/src/cmd/install/binary.rs, crates/terminus/src/cmd/install/mod.rs (anchor: `run`), crates/terminus/tests/install.rs (anchor: `install_from_the_stable_path_still_starts_its_backfill`)
- **Action:**
  - **The rule (D-02).** When both conditions below hold, install writes no version file and does not re-create the link:
    - The stable path is a symbolic link whose target sits in a folder directly under the versions folder. Compare paths component by component.
    - That target is the same file as `current_exe()`.
  - **"The same file" means device and inode.** Read them from the metadata of both paths (`std::os::unix::fs::MetadataExt`). Do not compare path strings. On Linux, `current_exe()` is the resolved path, while the link holds whatever spelling of `HOME` wrote it. A `HOME` reached through a linked folder spells the two differently.
  - **Never compare the folder's name with the build's version.** `install.sh` names the folder from its manifest, and every development artifact reports 0.1.1. A test that installs "version 2.0.0" therefore runs a build that reports 0.1.1. Comparing versions would store a second copy under 0.1.1 and move the link off the file `install.sh` placed.
  - **Everything else runs as usual.** The hooks, the MCP registration, the receipt (whose `binary` line still names both paths) and the backfill all run as for any install.
- **Verify:** `cargo test -p terminus --test install` passes, and it checks all of the following:
  - After a normal install, running `command_at(stable, ["install"])` exits 0. The link's inode and target, and the version file's inode, are the same as before the run.
  - In a second test, the test makes `<root>/.local/lib/crenshawdev/terminus/versions/9.9.9/terminus` a copy of the test binary and links the stable path to it. Running `<stable> install` then exits 0, creates no `versions/<CARGO_PKG_VERSION>` folder, and leaves the link pointing at `9.9.9` with the same inode.
  - `install_from_the_stable_path_still_starts_its_backfill` still passes.

### Task 3: Uninstall removes the link and leaves the versions folder

- **Files:** crates/terminus/src/cmd/uninstall.rs (anchor: the module doc, `run`, `unplace`), crates/terminus/tests/uninstall.rs (anchor: `install_then_uninstall_leaves_both_files_exactly_as_they_were`, `uninstall_on_a_machine_that_was_never_installed_changes_nothing`)
- **Action:**
  - **Removing the link (D-11).** `unplace` keeps removing the stable path with `remove_file`. That removes the link and not its target, and it still removes a marked plain file left by an older install.
  - **Printing the versions folder.** `run` prints the versions folder's path in the same form as the data directory line. It adds a line saying uninstall leaves the folder in place, or that nothing is there.
  - **`--purge` is unchanged.** It still deletes only the data directory, never the versions folder. A running session's MCP server and its hook's detached ingest run from those files (D-04).
  - **The module doc.** Update it to say that install adds the link and a version file, and that uninstall removes the link and keeps the versions, with the D-04 reason.
  - **Tests.**
    - `install_then_uninstall_leaves_both_files_exactly_as_they_were` checks that the stable path is gone with `symlink_metadata` returning `NotFound`, because `exists()` also reports false for a dangling link.
    - The same test asserts that `versions/<CARGO_PKG_VERSION>/terminus` is still there with the test binary's bytes, and that the output contains the versions folder's path.
    - `uninstall_on_a_machine_that_was_never_installed_changes_nothing` still exits 0.
- **Verify:** `cargo test -p terminus --test uninstall` passes with those assertions. `cargo test --workspace --all-features` passes. `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- Plans 1 and 2 both declare `crates/terminus/src/cmd/install/binary.rs`: this plan for the layout, plan 2 for the test that ties `install.sh`'s copy of the marker to `MARKER`. They run in order and not in parallel, as phase 3's plans did.
- The versions folder resolves from `HOME` even when `TERMINUS_BIN_DIR` is set. That is the planner's call, made because D-01 puts the folder under HOME and names no override.
