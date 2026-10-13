---
status: testing
phase: 3
fields_version: 1
started: 2026-10-13
updated: 2026-10-13
---

## Items

### 1. Install asks nothing with stdin closed
expected: With stdin closed and a fake claude that succeeds, `terminus install` exits 0, prints no question, and leaves the same settings.json, stable-path binary and claude call log as a run with a terminal attached
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: install_reads_nothing_from_stdin (tests/install.rs:969) passes; spot-check of the built binary with stdin at /dev/null and under a script(1) pty gave identical settings.json, claude.log, binary bytes and output, exit 0 both, no question printed.

### 2. install --yes is misuse
expected: `terminus install --yes` exits 2
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: install/mod.rs:93-98 sends any argument to crate::unexpected; install_takes_no_yes_flag (tests/install.rs:928) passes with exit 2 for --yes and -y.

### 3. .claude.json untouched by install, rerun and uninstall
expected: When the fake claude writes nothing, ~/.claude.json is byte-identical after install, a rerun and uninstall
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: Only reads of .claude.json in install/mod.rs:260-295 and uninstall.rs:300-345; install_then_uninstall_leaves_both_files_exactly_as_they_were (tests/uninstall.rs:301) and two_installs_leave_claude_json_alone_and_each_asks_claude_for_the_add (tests/install.rs:722) pass with byte comparisons.

### 4. Install calls claude mcp add-json with the stable path
expected: The fake's log shows `mcp add-json --scope user terminus` with command set to the stable path and args ["mcp"], and every claude process gets CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: install/mod.rs:285, targets.rs:214-228, claude.rs:89, doctor.rs:1179; two_installs_leave_claude_json_alone_and_each_asks_claude_for_the_add, every_claude_gets_the_traffic_variable and doctors_claude_version_runs_with_nonessential_traffic_off pass; spot-check log shows traffic=1 and the add with command = stable path, args ["mcp"].

### 5. Rerun with a correct entry runs no claude
expected: With the entry already correct in ~/.claude.json, a rerun logs no claude call
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: install/mod.rs:269-271; a_current_mcp_registration_runs_no_claude (tests/install.rs:784) passes with an empty call log.

### 6. Wrong entry is removed then added
expected: With a wrong-shaped user entry, the log shows `mcp remove --scope user terminus` and then the add
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: install/mod.rs:275-285; a_stale_mcp_registration_is_removed_and_added_again (tests/install.rs:752) passes: remove, then add keeping the user's env key.

### 7. No claude on PATH leaves install incomplete
expected: With no claude on PATH, install still places the binary and merges the hooks, prints that the install is incomplete with the exact `claude mcp add-json` command, and exits non-zero
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: install/mod.rs:121-173,212-218; without_claude_on_path_the_install_is_incomplete (tests/install.rs:838) passes: exit 1, binary and 4 hooks in place, 'incomplete' and the add-json command printed.

### 8. Failing claude leaves install incomplete
expected: With a fake claude that exits 1, install still places the binary and merges the hooks, prints that the install is incomplete with the exact `claude mcp add-json` command, and exits non-zero
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: claude.rs:129-141, install/mod.rs:285-293; a_claude_that_fails_leaves_the_install_incomplete_with_its_first_line (tests/install.rs:850) passes: exit 1, binary and hooks in place, first stderr line as cause, add-json command printed.

### 9. Hook merge makes one backup and reruns clean
expected: Install merges the hook entries into settings.json and makes one backup; a rerun leaves settings.json byte-identical and makes no new backup
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: install/mod.rs:118-130, json_file.rs:466-489; two_installs_leave_one_hook_entry_per_event_and_change_nothing_else (tests/install.rs:566) passes: one backup with the pre-install bytes, second run byte-identical.

### 10. Receipt lists changes, backfill and cleanupPeriodDays
expected: The receipt lists every file changed, the backfill estimate, `terminus status` as the place to watch, the current cleanupPeriodDays, the line saying older transcripts are already gone, and the line that raises it
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: install/mod.rs:135-208,313-351,362-398; install_reports_the_two_settings_it_never_changes (tests/install.rs:1059) and the_receipt_carries_the_backfill_estimate (tests/install.rs:1124) pass; spot-check receipt named every file the run changed and printed the value, the already-gone line, the raise line, the estimate and terminus status.

### 11. cleanupPeriodDays and autoCompactEnabled keep their values
expected: After install, cleanupPeriodDays and autoCompactEnabled in settings.json hold the values they had before
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: No writer of either key in crates/terminus/src; install_reports_the_two_settings_it_never_changes asserts both unchanged for three seeds and passes; spot-check settings.json kept 7 and true.

### 12. Uninstall removes the entry through claude mcp remove
expected: Uninstall with a user-level entry logs `claude mcp remove --scope user terminus`
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: uninstall.rs:83,300-332; only_the_terminus_key_leaves_mcp_servers (tests/uninstall.rs:343) passes: one call, mcp remove --scope user terminus, traffic=1.

### 13. uninstall --purge still shows size and asks
expected: `terminus uninstall --purge` shows the data size and asks before deleting
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: uninstall.rs:381-451 changed only in comments; purge tests unchanged since 2d04ac0 and passing: the_size_is_shown_before_the_question, purge_with_nothing_to_read_refuses_and_keeps_the_archive, answering_no_keeps_the_archive_and_exits_zero, purge_with_yes_prints_the_size_and_the_path_then_deletes.

### 14. Doctor gives the claude mcp add-json fix without claude
expected: With no claude on PATH, `terminus doctor` prints the `claude mcp add-json` line as the MCP fix
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: doctor.rs:509-512; without_claude_on_path_the_mcp_fix_is_the_claude_mcp_command (tests/doctor.rs:529) passes against doctor's printed output.

### 15. Full workspace suite passes
expected: `cargo test --workspace --all-features` passes
criterion: AC6
status: pass
first_pass: pass
source: model
evidence: cargo test --workspace --all-features -> exit 0, 63 test binaries, 764 passed, 0 failed (brief.rs wall-clock test and the fake_claude tests included, no ETXTBSY)

### 16. Docs no longer describe the old install
expected: DESIGN-BRIEF.md and docs/json-shapes.md no longer describe an install diff, an install confirmation, install's --yes or an offer to raise cleanupPeriodDays
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: DESIGN-BRIEF.md:102,392,396-400 and docs/json-shapes.md:517-519 describe the new install; no install diff, confirmation, --yes or cleanupPeriodDays offer remains in either file.

### 17. Changelog records the change
expected: CHANGELOG.md [Unreleased] records that install asks nothing and places the MCP entry through claude mcp
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: CHANGELOG.md:27 and :32 under [Unreleased] (:10), with removals at :90-94.

### 18. Uninstall leaves an old .claude.json backup in place
expected: An existing .claude.json.terminus-backup is not restored, stays in place, and uninstall prints its path
status: pass
first_pass: pass
source: verifier
evidence: uninstall.rs:334-343; an_old_claude_json_backup_is_reported_and_left_alone (tests/uninstall.rs:407) passes: path printed, backup and .claude.json bytes unchanged.

### 19. Uninstall prints the old .claude.json backup path even when .claude.json cannot be parsed
expected: behavior wrong - D-15 says uninstall prints the path of an existing .claude.json.terminus-backup with no condition, but the print sits after an early return taken when .claude.json does not parse
origin: verifier
status: pass
first_pass: fail
source: model
evidence: After 5f01161, scratch fixture (fake claude on PATH): install, then .claude.json set to '{' with an old .claude.json.terminus-backup beside it, then terminus uninstall < /dev/null. Output: 'left alone: ... is not valid JSON ...' followed by 'an old backup from install is at .../claude/.claude.json.terminus-backup ... left in place and not restored'. sha256sum -c on the backup: OK. .claude.json still reads '{'. Test an_old_claude_json_backup_is_reported_when_claude_json_does_not_parse failed before the fix and passes after it.
reported: behavior wrong - D-15 says uninstall prints the path of an existing .claude.json.terminus-backup with no condition, but the print sits after an early return taken when .claude.json does not parse
severity: minor
cause: unregister (crates/terminus/src/cmd/uninstall.rs:300-345) gets the old backup's path from Document::backup_path, which needs a parsed Document. When ~/.claude.json does not parse, the Err arm at :306-309 returns before the backup report at :334-343, so the path is never printed. The path only depends on the file's location (json_file.rs:494-498: resolve the symlink, add .terminus-backup), not on its contents.
fix: 5f01161, retest

### 20. Run cargo test --workspace --all-features once, in the walk
expected: Exit 0 with no failed test. Two known risks to watch for: the SessionStart wall-clock test at crates/terminus/tests/brief.rs:256 (10 ms p99) failed once under parallel load in plan 3, and terminus_core::testkit::fake_claude has the same ETXTBSY window the runner unit tests were patched for in 01402d1 (SUMMARY open items).
origin: verifier
why_human: Not human-only. It is runnable here, but the dispatcher reserved the single full-suite run for the walk, so this pass did not exercise it. Every phase-3 test was run by name and passed.
status: pass
first_pass: pass
source: model
evidence: cargo test --workspace --all-features -> exit 0, 63 test binaries, 764 passed, 0 failed (brief.rs wall-clock test and the fake_claude tests included, no ETXTBSY)

## Summary

total: 20
passed: 20
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 1
