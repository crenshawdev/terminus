---
status: testing
phase: 4
fields_version: 1
started: 2026-08-20
updated: 2026-08-20
---

## Items

### 1. Cold start from scratch
expected: With everything stopped and the data directory cleared, a fresh `verbatim backfill` (or install) boots clean, the store is created with its schema, and one primary query (`verbatim search` or `verbatim status`) returns real data.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: backfill on an absent data dir created the store and status returned 2 sessions / 14 turns / 2 watermarks with a real last-run row.

### 2. Four hooks: exit 0, empty stdout, under budget
expected: Feeding each of SessionStart, UserPromptSubmit, SessionEnd and PostCompact the exact JSON Claude Code sends on stdin returns exit 0 with completely empty stdout, at p99 under 10 ms wall over 100 runs.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: tests/hook.rs every_event_exits_zero_with_an_empty_stdout_inside_the_budget passed in release: p99 0.89 / 0.72 / 0.80 / 0.82 ms over 100 runs per event against 10 ms, stdout empty on all 400.

### 3. Spawned ingest survives Claude Code's kill
expected: A hook killed the way Claude Code kills one (SIGKILL to the process group, then SIGKILL to every descendant PID a `ps` walk finds) leaves the spawned ingest alive and its pass completes to a committed store.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: tests/hook.rs the_ingest_survives_a_group_kill_and_a_descendant_sweep passed; spawn.rs double fork with process_group(0) and null stdio.

### 4. Windows: no console window, no inherited handle
expected: On Windows the spawned child shows no console window and holds no handle inherited from the hook. (human-verify: needs a Windows machine)
criterion: AC3
status: skipped
reported: skip for now
reason: No Windows machine available on this host

### 5. Install twice is idempotent
expected: Running `verbatim install` twice leaves exactly one hook entry per hook in settings.json and exactly one `verbatim` entry in `~/.claude.json` `.mcpServers`, with one backup of each file written on the first run only, and every other key in both files byte-identical to before.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: second install --yes left both files md5-identical, 4 hook entries, one .mcpServers.verbatim, exactly one backup per file from the first run only.

### 6. Upgrade rewrites no hooks; foreign binary is refused
expected: Replacing the binary and rerunning install changes no byte of settings.json's `hooks` object. Install against a stable path holding a binary without verbatim's marker exits non-zero, writes nothing to either file, and prints a command that makes the same install succeed.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: an_upgrade_rewrites_no_byte_of_the_hooks_object passed; /bin/true at the stable path made install exit 1, change neither file, and print the `rm -f ... && ... install` fix.

### 7. Doctor is read-only and names its fix
expected: `verbatim doctor` against a data directory that does not exist creates no file or directory, and against a hook path pointing at a missing binary reports the problem and prints a command that, when run, makes the same check pass.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: doctor.rs tests 14/14 passed; manual run printed 14 named checks plus two fix lines and created nothing.

### 8. Uninstall restores and leaves data
expected: Uninstall on a settings.json unchanged since install restores it to its pre-install bytes, removes only the `verbatim` key from `.mcpServers`, and leaves the data directory present, printing its path.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: settings.json and .claude.json returned to their pre-install bytes, only the verbatim key left .mcpServers, data directory kept and its path printed.

### 9. Backfill estimates first, returns the shell, converges
expected: Backfill over the real corpus prints a session count, byte total and time estimate before any ingest work and returns the shell in under 100 ms; killed mid-pass and rerun, it converges to the same store contents as an uninterrupted run.
criterion: AC8
status: pass
first_pass: pass
source: verifier
evidence: estimate printed then a 0 ms return with the store filling behind it; the 8-kill convergence test reached and passed its assertions in 6 of 9 runs.

### 10. npm pack and tarball install
expected: `npm pack` on the in-repo workspace and installing the resulting tarball puts a working `verbatim` on the path whose `--version` matches the Rust build, with no postinstall script in the package.
criterion: AC9
status: pass
first_pass: pass
source: verifier
evidence: both tarballs packed and installed offline into a scratch prefix; the shim resolved @verbatim/linux-x64 and printed 0.1.0, matching the Rust build; no scripts key in either packed manifest.

### 11. Install run from the stable path cannot start its own backfill
expected: behavior wrong - install replaces its own executable inode by rename, then spawns std::env::current_exe(), which by then names a deleted file
origin: verifier
status: pass
first_pass: fail
source: model
evidence: release build, real repro: install once from the build dir -> 1 session; add a transcript; `<stable path>/verbatim install --yes` prints the estimate and "archiving that now, in a detached process" with no `problem:` line, and the store reaches 2 sessions. Unit test `install_from_the_stable_path_still_starts_its_backfill` (crates/verbatim/tests/install.rs) passes, and fails with the same ENOENT when the one-line call site is reverted. Fixed in 5c20a4c.
reported: behavior wrong - install replaces its own executable inode by rename, then spawns std::env::current_exe(), which by then names a deleted file
severity: major
cause: crates/verbatim/src/cmd/spawn.rs:106 spawns Command::new(std::env::current_exe()?). install/binary.rs:185 places the new binary at the stable path by atomic rename, unlinking the inode the running process was exec'd from. When install is itself run from the stable path, /proc/self/exe then resolves to a deleted file, so the exec fails ENOENT and the backfill install just announced never starts. The hook path is unaffected (nothing renames over the binary), and the npm path is unaffected (current_exe is the platform-package binary, not the stable path).
fix: 5c20a4c, retest

### 12. The AC8 convergence test panics about one run in three
expected: behavior wrong - the acceptance test that proves AC8's convergence half is nondeterministic and fails before it reaches its convergence assertion
origin: verifier
status: pass
first_pass: fail
source: model
evidence: a_killed_backfill_converges_on_the_store_an_uninterrupted_one_reaches: 0 failures in 12 release runs with Snapshot::started in place; 3 failures in 12 with the guard reverted and nothing else changed. Fixed in 77e22d1.
reported: behavior wrong - the acceptance test that proves AC8's convergence half is nondeterministic and fails before it reaches its convergence assertion
severity: major
cause: Test-only. crates/verbatim/tests/backfill.rs:325 Snapshot::of unconditionally prepares SELECT ... FROM sessions and unwraps. The mid-progress guard at :435 admits any data_dir whose DB file merely exists, and Store::open creates that file before it commits the schema, so a SIGKILL landing in that window hands Snapshot::of a table-less database and the unwrap panics before the convergence assertion runs. 3 failures in 9 release runs; the 6 that got past it all converged.
fix: 77e22d1, retest

### 13. On a Windows machine, fire a hook and watch the spawned ingest
expected: No console window appears, and the child holds no handle inherited from the hook (all three stdio null, DETACHED_PROCESS | CREATE_NO_WINDOW).
origin: verifier
why_human: Out of reach here: no Windows machine is present, so the creation flags in crates/verbatim/src/cmd/spawn.rs cannot be exercised at all on this host.
status: skipped
reported: skip for now
reason: Same Windows check as item 4; no Windows machine available on this host

## Summary

total: 13
passed: 11
failed: 0
pending: 0
skipped: 2
blocked: 0
reworked: 2
