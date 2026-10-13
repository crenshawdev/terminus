---
status: testing
phase: 2
fields_version: 1
started: 2026-10-13
updated: 2026-10-13
---

## Items

### 1. Fresh start from an empty home
expected: With HOME and XDG_DATA_HOME pointed at an empty temporary directory, terminus creates its data directory and database owner-only, and `terminus doctor` reports no failure caused by the fresh state.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Scratch env -i run: ingest created xdg/terminus 700 with terminus.db, LOCK and the snapshot at 600 and snapshots/ at 700. Doctor's fresh-state data checks were notes, not problems, and after install doctor printed 'no problems.' and exited 0.

### 2. npm is gone
expected: `npm/` does not exist. `git grep -i npm` over `crates/`, `docs/`, `Cargo.toml` and `.gitignore` finds nothing, and `DESIGN-BRIEF.md`, `PROJECT.md` (outside its shipped-cycle tables) and `REQUIREMENTS.md` (outside the INST-01 and INST-09 rows) name npm only to say it was removed.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: npm/ is absent and git ls-files npm lists 0. `git grep -i npm -- crates docs Cargo.toml .gitignore` returns rc=1. DESIGN-BRIEF.md:380, PROJECT.md:21/106/131 and REQUIREMENTS.md:120 each say npm was removed, and the other hits are the PROJECT shipped-cycle rows :35/:55 and the INST-09 and INST-01 rows.

### 3. No Windows or Unix guard left in crates
expected: `git grep -E` over `crates/` finds no `cfg(windows)`, `cfg!(windows)`, `target_os = "windows"`, `cfg(not(windows))`, `cfg(unix)`, `cfg(not(unix))`, `cfg(all(test, unix))`, `LOCALAPPDATA`, `APPDATA`, `USERPROFILE` or `.exe"` string literal.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: The AC2 pattern returns rc=1 over crates/ with both git grep and grep -r. target_os appears only in the refusal, the macOS and Linux fold, the Linux and macOS platform_data_dir, and two cfg!(target_os = "macos") test arms.

### 4. Compile-time refusal and its test
expected: `crates/terminus-core/src/lib.rs` carries a `compile_error!` guarded by `not(any(target_os = "linux", target_os = "macos"))` with the message "Terminus supports Linux and macOS only.", and a test in the suite fails if the guard or the message is removed.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: lib.rs:22-23 has the guard and message before the first pub mod. supported_systems.rs passes on the real file and fails on scratch copies with the guard changed to not(unix), with the message reworded, or with the pair deleted.

### 5. A Windows build fails with the message
expected: On a Windows machine, `cargo build` of Terminus fails and its output contains "Terminus supports Linux and macOS only." (human-verify: needs a Windows machine with Rust)
criterion: AC4
status: skipped
reported: no longer supporting windows   remember
reason: Terminus no longer supports Windows, so there is no Windows machine to run the build on.

### 6. Full suite passes at the stated count
expected: `cargo test --workspace --all-features` passes and lists 749 tests, and the phase summary lists every test helper and `.exe` arm removed.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: cargo test --workspace --all-features exited 0 with 750 passed and 0 failed, and --list shows 750. That is the 749 baseline plus the one AC3 test: the diff adds exactly one #[test] and removes none. The removed test lines match SUMMARY.md:20-26 item for item. The literal 749 in AC5 is tracked by issue #6.

### 7. Doctor and uninstall print only Unix fixes
expected: `terminus doctor`, `terminus doctor --json` and `terminus uninstall` print no `attrib`, `del` or `.exe` fix and no mention of Windows, and `doctor --json` carries the same top-level keys it did before the phase.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: Scratch run of install, doctor, doctor --json, uninstall and the occupied-path refusal: no attrib, del, .exe or Windows text. The read-only data dir fix is chmod u+w and the refusal prints rm -f. Envelope keys are [command, data, ok, reason], pinned by the_json_document_carries_every_check_by_name (passed), and the doctor diff adds and removes no check.

### 8. Docs name Linux and macOS and close the Windows items
expected: `DESIGN-BRIEF.md` and `PROJECT.md` name Linux and macOS as the supported systems, every remaining mention of Windows in them is about claude-mem or says Windows was dropped, and `CAPTURE.md`'s npm items, Windows items and icacls proposal are marked closed as no longer applicable.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: DESIGN-BRIEF.md mentions Windows only at :22 and :119 (claude-mem) and :374 (supported systems). PROJECT.md does so only at :80 (claude-mem) and :103 and :138 (dropped). In CAPTURE.md the npm items, the AC3 Windows item, the Publish milestone (0 unchecked bullets) and the icacls proposal (:298) are closed, and capture-check reports annotations [].

### 9. Changelog records both removals
expected: `CHANGELOG.md`'s `[Unreleased]` has Removed entries for the npm package and for Windows support, its rename entry names no `%LOCALAPPDATA%` path, and the Verbatim 0.1.1 line about the Windows deferral is unchanged.
status: pass
first_pass: pass
source: verifier
evidence: CHANGELOG.md:51-58 has the npm and Windows Removed entries under [Unreleased]. The rename entry has no %LOCALAPPDATA%. The 0.1.1 deferral line at :96-97 is unchanged in the 26b79d1..HEAD diff.

### 10. terminus doctor says Linux and macOS (ROADMAP success criterion 5)
expected: behavior wrong - doctor prints nothing naming Linux or macOS. CONTEXT D-12 decided doctor gets no platform check, which drops the doctor half of ROADMAP SC5 without amending the roadmap.
origin: verifier
status: pass
first_pass: fail
source: model
evidence: Retest against ROADMAP SC5 as amended in 3fa6ea7 (the docs say Linux and macOS, terminus doctor prints no Windows fix). `env -i HOME=<tmp>/home XDG_DATA_HOME=<tmp>/xdg terminus doctor` printed 26 lines and `doctor --json` printed 30 lines, with 0 lines matching windows, attrib, del or .exe in either. Both exit non-zero because the fresh home is not installed, which is doctor reporting that state, not a Windows fix. The docs half is item 8 (pass).
reported: behavior wrong - doctor prints nothing naming Linux or macOS. CONTEXT D-12 decided doctor gets no platform check, which drops the doctor half of ROADMAP SC5 without amending the roadmap.
severity: minor
cause: Not a code defect. CONTEXT D-12 decided doctor gets no platform check, because a check would always pass on a binary that only builds on Linux and macOS and the doctor --json key set is a contract. That dropped the doctor half of ROADMAP success criterion 5 on purpose, and the roadmap line was never amended to match. Issue #7 tracks the same gap.
fix: 3fa6ea7, retest

### 11. On a Windows machine with Rust installed, run `cargo build` in the Terminus repository root
expected: The build fails, and its output contains "Terminus supports Linux and macOS only." It should appear first or at least among the unresolved std::os::unix errors.
origin: verifier
why_human: Out-of-reach resource. This machine has only the x86_64-unknown-linux-gnu std (rustup target list --installed) and no x86_64-w64-mingw32-gcc. Adding a Windows target needs the network, and a cross-build also needs a C compiler for the bundled SQLite and zstd. The source-scan test proves the guard is present, but only a real Windows build shows the message is reached (CONTEXT's first flagged assumption).
status: skipped
reported: no longer supporting windows   remember
reason: Terminus no longer supports Windows, so there is no Windows machine to run the build on.

## Summary

total: 11
passed: 9
failed: 0
pending: 0
skipped: 2
blocked: 0
reworked: 1
