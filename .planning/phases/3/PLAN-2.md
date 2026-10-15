---
phase: 3
plan: 2
requirements: [INST-10, INST-11]
files:
  - crates/terminus/src/cmd/install/mod.rs
  - crates/terminus/src/cmd/uninstall.rs
  - crates/terminus/src/cmd/doctor.rs
  - crates/terminus/src/cmd/install/claude.rs
  - crates/terminus/src/cmd/install/targets.rs
  - crates/terminus/tests/install.rs
  - crates/terminus/tests/uninstall.rs
  - crates/terminus/tests/doctor.rs
---

# Phase 3: Install Asks Nothing - Plan 2

## Goal

Install ends with a receipt. It lists every change, the backfill estimate, and the current `cleanupPeriodDays` with the line that raises it. Uninstall removes the registration with `claude mcp remove` and keeps its confirmation and `--purge`. When `claude` is not on `PATH`, doctor's MCP fix names the `claude` command.

## Must be true when done

- **The receipt of a complete install names everything it touched.** That means the stable path, `settings.json` and its backup, and `.claude.json` with how the entry was placed or that it was already correct. It also names the data directory, the backfill estimate (`transcript(s)` and `not yet archived`, or `nothing to archive yet`), and `terminus status` as the place to watch the backfill.
- **The receipt reports `cleanupPeriodDays` without changing it.** It states the current value, or says the key is unset and Claude Code's default of 30 applies. It says transcripts older than that are already gone. It prints `set "cleanupPeriodDays": 3650 in <settings.json path>` whatever the value is. After install, `cleanupPeriodDays` and `autoCompactEnabled` hold the values they had before.
- **Uninstall removes the registration only when there is one.** With a user-level `terminus` entry in `~/.claude.json`, it logs `claude mcp remove --scope user terminus` with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`. With no entry, it starts no `claude`. With a fake `claude` that writes nothing, `~/.claude.json` is byte-identical across install, rerun and uninstall.
- **Uninstall finishes its other steps when `claude` is missing or fails.** With no `claude` on `PATH`, or a `claude` that exits 1, it prints the exact `claude mcp remove --scope user terminus` command, still removes the hook entries and the binary, and exits 1.
- **An old `.claude.json.terminus-backup` is left alone.** Uninstall never restores or deletes it. It prints the backup's path, and both the backup and `~/.claude.json` are byte-identical afterwards.
- **`--purge` is unchanged.** It still shows the path and the size and asks before deleting, and `uninstall --purge --yes` still deletes.
- **Doctor's MCP fix depends on whether `claude` is on `PATH`.** When doctor finds the MCP entry missing or wrong and `claude` is not on `PATH`, its fix is the `claude mcp add-json --scope user terminus '...'` line. When `claude` is on `PATH`, the fix is `<this build> install`, as before.
- **Doctor's own `claude --version` runs with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` (D-04).** With the fake on `PATH`, the fake's log records `--version` with the variable at `1`.
- **Doctor's `--json` keeps its keys, and the workspace is clean.** `doctor --json` carries the same keys it did before the phase [recalled: phases/2/UAT.md, phase 2]. `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- This is plan 2 of 3, and it runs after plan 1. Plan 1 built four things this plan uses, so read them before starting:
  - the runner in `install/claude.rs`
  - the D-01 test, the placed entry and the finishing command, all in `install/targets.rs`
  - the fake `claude` in `terminus_core::testkit`
  - the fixture `PATH` in all three test files
- The decisions that bind this plan come from CONTEXT:
  - D-10, D-11, D-13, D-14, D-15 and D-16.
  - D-12's receipt line is already in the summary from plan 1, task 4.
  - From D-17, `Document::overwrite` stays for the `settings.json` restore.
  - From D-19, the module comments in `install/mod.rs` and `uninstall.rs`.
- `install/claude.rs` and `install/targets.rs` are declared only in case uninstall or doctor needs a visibility or call-shape change in them. None of their behaviour changes in this plan.
- Out of scope for this plan:
  - uninstall's own `--yes` for `--purge`, which stays as it is
  - doctor's `CLAUDE_FLOOR`
  - the project and local MCP scopes
- Run `rustfmt --edition 2021` on the files you touch, and only on those.

## Tasks

### Task 1: Install ends with a receipt

- **Files:** crates/terminus/src/cmd/install/mod.rs (anchor: `run`, `advise`, `start_backfill`, `LOW_CLEANUP_DAYS`, `CLAUDE_DEFAULT_CLEANUP_DAYS`, `KEEP_CLEANUP_DAYS`, the module doc), crates/terminus/tests/install.rs (anchor: `install_reports_the_two_settings_it_never_changes`, `the_summary_names_the_data_directory_and_the_backups_it_wrote`)
- **Action:**
  - **`cleanupPeriodDays` (D-11).** Replace the block in `advise` that runs only for a low value.
    - The receipt always states the current `cleanupPeriodDays`. Read it from the user `settings.json` only, re-read after the write, as `advise` does today.
    - A value that is not an integer reads as unset, as it does today. Unset reads as Claude Code's default of 30.
    - Say that transcripts older than that many days are already gone. That is what a low value loses.
    - Print doctor's raise line, `set "cleanupPeriodDays": 3650 in <settings.json path>`, at every value, 3650 included.
    - Remove `LOW_CLEANUP_DAYS` from install once nothing reads it.
    - Keep the `autoCompactEnabled` line, which says install never sets that key and recommends turning auto-compact off.
    - Install writes neither setting.
  - **The backfill (D-10).** The backfill part of the receipt is what `start_backfill` already prints: `backfill::print_estimate` and the `terminus status` line. Add no log file. `start_backfill` stays the last thing install does, and an incomplete install still returns `Failure::Silent` after it, as plan 1 task 3 set up.
  - **The receipt.** It lists every file the run changed or made a decision about, and keeps the data directory, transcript roots and undo lines:
    - the binary at the stable path
    - `settings.json`: whether entries were added or were already current, plus the D-12 rewrite line when it applies
    - each backup written
    - `.claude.json`, marked as one of: registered; replaced; already correct, with no `claude` run; or incomplete, with the cause and the command
  - **The module doc of `mod.rs`.** Rewrite it to describe install as it now runs:
    - it puts the binary at the stable path
    - it merges the four hook entries into `settings.json` with a one-time backup
    - it places the MCP entry through `claude mcp` and never writes `.claude.json`
    - it ends with a receipt and starts the backfill last
    - it asks nothing

    Keep it short and plain, per the "Code comments" rule in `~/.claude/john-voice.md`.
- **Verify:**
  - `cargo test -p terminus --all-features --test install` passes. Extend `install_reports_the_two_settings_it_never_changes`, or add a test beside it, to cover three seeded values of `cleanupPeriodDays`:
    - With the seeded `cleanupPeriodDays: 7`, the receipt contains `7`, the already-gone line, and `set "cleanupPeriodDays": 3650 in <fixture settings.json path>`.
    - With `3650` seeded, the raise line still appears.
    - With the key absent, the receipt says it is unset and the default of 30 applies, and the raise line appears.
  - For all three seeds, the receipt also contains the stable path, the `settings.json` path, the backup path, the `.claude.json` path and `terminus status`. `settings.json`'s `cleanupPeriodDays` and `autoCompactEnabled` equal the seed's.
  - A test with one transcript under the fixture's `projects` directory finds `transcript(s)` and `not yet archived` in the receipt.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 2: Uninstall removes the registration with `claude mcp remove`

- **Files:** crates/terminus/src/cmd/uninstall.rs (anchor: `run`, `finish`, `Part`, `clean`, `restore`, `strip`, `strip_server`, `answered`, module doc), crates/terminus/tests/uninstall.rs (anchor: `install_then_uninstall_leaves_both_files_exactly_as_they_were`, `only_the_terminus_key_leaves_mcp_servers`, `a_settings_file_deleted_since_install_is_one_reported_line`, `uninstall_on_a_machine_that_was_never_installed_changes_nothing`)
- **Action:**
  - **Uninstall stops writing `~/.claude.json` (D-14).** Delete `Part::McpServer`, `strip_server`, and the `.claude.json` arm of both `clean` and `restore`. Uninstall no longer edits, overwrites or restores `~/.claude.json`. With only one part left, `Part` can be folded away. The `settings.json` strip, and its byte restore through `Document::overwrite`, stay exactly as INST-07 has them.
  - **Removing the registration (D-13).** Add a registration step to `run`, where the `.claude.json` clean used to be. It reads `~/.claude.json` with `Document::read`, then:
    - If the read fails, uninstall reports it in one line and the run counts as not OK, the same way `clean` reports a failed read today.
    - If there is no file, or no `.mcpServers.terminus`, it says no registration of terminus's was there and starts no `claude`.
    - If an entry is there, it runs `mcp remove --scope user terminus` through plan 1's runner, always with `--scope user`. On success, it says the registration was removed with `claude mcp remove`.
    - If `claude` is not on `PATH`, times out or exits non-zero, it prints the cause and the exact command, `claude mcp remove --scope user terminus`. The steps after it still run, and `finish` returns `Failure::Silent`.
  - **The old backup (D-15).** If a `.claude.json.terminus-backup` sits beside the resolved `.claude.json` (at `Document::backup_path`), uninstall prints its path and says it is being left in place. It never restores it, reads from it or deletes it.
  - **What stays the same.** `--purge`, its question through `install::confirm`, and uninstall's own `--yes` do not change.
  - **Doc comments.**
    - `answered`'s doc compares this `--yes` with install's, which is gone, so rewrite it.
    - Rewrite the module doc per D-19. What install adds is now the hook entries and the binary, and the registration is removed through `claude mcp`.
    - Keep both short and plain.
  - **Tests (D-18).** Rewrite these existing tests:
    - `install_then_uninstall_leaves_both_files_exactly_as_they_were` keeps checking `settings.json`'s byte restore. It now also asserts that `.claude.json` is byte-identical after install, rerun and uninstall, with the fake writing nothing.
    - `only_the_terminus_key_leaves_mcp_servers` becomes the remove test, with a correct entry seeded:
      - install logs nothing
      - uninstall logs exactly `mcp remove --scope user terminus`, with the variable at `1`
      - `.claude.json` stays byte-identical, entry included, because Terminus did not write it
    - `a_settings_file_deleted_since_install_is_one_reported_line` takes its evidence that uninstall carried on from the logged remove and the removed binary.
    - The never-installed test asserts an empty log.

    Add these new tests:
    - With an entry seeded, and `claude` either missing from `PATH` or a fake that exits 1, uninstall exits 1 and prints the command. The hook entries are gone from `settings.json`, and the stable binary is gone.
    - With a `.claude.json.terminus-backup` already present and holding other bytes, uninstall prints its path. The backup and `.claude.json` are both byte-identical afterwards.
- **Verify:**
  - `cargo test -p terminus --all-features --test uninstall` passes. That includes the tests above, plus these existing ones, unchanged: `purge_with_nothing_to_read_refuses_and_keeps_the_archive`, `answering_no_keeps_the_archive_and_exits_zero`, `purge_with_yes_prints_the_size_and_the_path_then_deletes` and `the_size_is_shown_before_the_question`.
  - `git grep -n -e strip_server -e McpServer -- crates/terminus/src` prints nothing.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 3: Doctor's MCP fix names the `claude` command when `claude` is not on PATH

- **Files:** crates/terminus/src/cmd/doctor.rs (anchor: `mcp_server`, `install_command`, `on_path`, `version_of`, the `LOW_CLEANUP_DAYS` doc), crates/terminus/tests/doctor.rs (anchor: `an_mcp_registration_that_points_elsewhere_is_a_problem`, `the_json_document_carries_every_check_by_name`)
- **Action:**
  - **The fix (D-16).** `mcp_server` stays a read of `~/.claude.json`, with the same `mcp_server` key and the same states. Every problem branch that carries `install_command()` today now picks its fix by whether `on_path("claude")` finds `claude`:
    - **`claude` is not on `PATH`.** The fix is plan 1's finishing command, built from the value doctor found under the key, or from none. That is the `claude mcp add-json --scope user terminus '<JSON>'` line. When an entry is there, the line starts with the remove, because `add-json` refuses a name that already exists (D-06).
    - **`claude` is on `PATH`.** The fix stays `<this build> install`.
    - **The file cannot be read.** That branch keeps no fix.
  - **Doctor stays read only.** This check starts no new process.
  - **D-04 reaches doctor.** `version_of` sets `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` on the `claude --version` process it starts, so every `claude` Terminus starts carries the variable, as D-04 says and as plan 3's docs and changelog state. Nothing else about the call changes: same arguments, same `VERSION_BUDGET`, same handling of its output.
  - **The `LOW_CLEANUP_DAYS` doc.** It says `cmd::install` holds the same three numbers for its advisory. Since plan 2 task 1, install holds only the default and the raise value, for its receipt. Reword it to match.
  - **Tests.** Add tests that run with `PATH` pointing at an empty fixture directory:
    - With the fixture's entry deleted from `.claude.json`, `mcp_server` is `problem`, and its fix starts with `claude mcp add-json --scope user terminus '` and contains the stable path.
    - With the entry's `command` changed, the fix starts with `claude mcp remove --scope user terminus && `.

    With the fake on `PATH`, those same states print `<exe> install`. `an_mcp_registration_that_points_elsewhere_is_a_problem` already covers that; make sure it still asserts it.

    Add one test for D-04: with the fake on `PATH`, run `doctor` and assert the fake's log holds a `--version` record with the variable at `1`. The fixture removes the variable from the spawned environment (plan 1, task 1), so a logged `1` can only have come from doctor.
- **Verify:**
  - `cargo test -p terminus --all-features --test doctor` passes. That includes the new tests, plus `the_json_document_carries_every_check_by_name` unchanged.
  - `cargo test --workspace --all-features` passes.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **D-16 reading.** D-16 says the fix "prints the `claude mcp add-json` line". For an entry in the wrong shape, that line on its own fails with "already exists". So the fix puts the remove in front of it, which keeps doctor's own rule that every printed fix is a command that works (INST-06). Confirmed by the owner at planning, 2026-10-13.
- **Old backups (D-15).** The owner's `.claude.json.terminus-backup` is 119,638 bytes from 2026-10-05. Restoring it would roll back eight days of Claude Code's own state. That is why the D-15 test checks that the backup is byte-identical, not just that its path is printed.
- **D-04 and doctor.** Plan 1 left doctor's `claude --version` call out of D-04 because nobody measured whether it connects. The owner ruled at planning, 2026-10-13, that D-04 covers it too, so task 3 sets the variable in `version_of`.
