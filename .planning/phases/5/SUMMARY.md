---
phase: 5
status: complete
completed: 2026-10-14
---

# Phase 5: Install from GitHub Releases - Summary

`install.sh` now downloads `terminus-<os>-<arch>.manifest` and `terminus-<os>-<arch>` from `https://github.com/crenshawdev/terminus/releases/latest/download` when given no argument, takes one `https://` source in its place, runs from a pipe as one brace group, and is tested under both `/bin/sh` and dash.

## What shipped

- Flat release asset names for the two downloads - `install.sh:184-187`
- No argument downloads from the GitHub releases; one `https://` argument replaces it; a non-https argument or a second one is refused - `install.sh:122-126`
- The whole script body is one brace group, which the shell reads whole before running any of it, so a pipe cut off at any byte before the closing brace runs nothing - `install.sh:121`, `install.sh:255`. Plan 1 built it as a function, `main`, called on the last line; UAT found that a cut right after `main` ran it with the source dropped, and the brace group replaced it
- `valid_utf8` rewritten as a byte state machine over `od` output, so it behaves the same under dash and bash - `install.sh`
- The run is refused when its shell changes HOME's bytes as it copies them, which dash 0.5.13 started without a UTF-8 locale does to eleven UTF-8 continuation bytes (ü, à and ł among them); the message names `LANG=C.UTF-8 sh` or bash - `install.sh:138-142`. UAT found it
- Tests for the flat names, the default source, two-argument refusal, both download refusals naming the URL tried, the piped forms, a cut at every byte, and a HOME holding `ü`; every install.sh test runs under `/bin/sh` and dash (44 tests) - `crates/terminus/tests/install_script.rs`
- CHANGELOG Unreleased entry shows the curl line, the optional source and the flat names - `CHANGELOG.md`
- DESIGN-BRIEF §13 and the binary layout bullet describe the curl line, the optional source and the two flat files - `DESIGN-BRIEF.md`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | b33847d | Download the flat release asset names a GitHub release can hold |
| 1 | 2 | 050c66e | Default the source to the GitHub releases and refuse a second one |
| 1 | 3 | 03d20a0 | Prove both download refusals name the release URL they tried |
| 1 | 4 | da6bf05 | Run the install from one function so a cut-off pipe runs nothing |
| 2 | 1 | 0e3cefa | Run every install.sh test under dash and read HOME's bytes through `od` |
| 2 | 2 | 30e21d1 | Show the curl line, the optional source and the flat release names in CHANGELOG and DESIGN-BRIEF |

## Deviations

- [deviation] Plan 2 and CONTEXT expected dash to trip on the bytes 0x81 to 0x88 inside `valid_utf8`'s byte-range patterns. Under `LANG=en_US.UTF-8`, dash 0.5.13.4 refused every non-ASCII string, `\303\251` included: dash takes its locale once at startup, so the script's `LC_ALL=C` never reaches its own pattern matching. `valid_utf8` was rewritten as a byte state machine over `od` output, as the plan's action allowed; its name and call site are unchanged. (0e3cefa)
- [deviation] Plan 2's probe recipe built each test string with `$(printf ...)` inside the shell under test. Under dash started in the C locale, that adds a 0x81 byte before every 0x82, which damaged the input before `valid_utf8` saw it. The probe was run with the strings built outside the shell and passed through the environment, the way HOME arrives. (0e3cefa)

## Open items

None.

## Goal check

The commits plausibly deliver the phase goal. With no argument, `install.sh:123` sets the source to `https://github.com/crenshawdev/terminus/releases/latest/download`, and `install.sh:184-187` fetch `$source/terminus-$os-$arch.manifest` and `$source/terminus-$os-$arch`, which covers criteria 1 and 2 with the refusals at `install.sh:124-125`. The body is a brace group from `install.sh:121` to `install.sh:255`. Plan 1's report records the piped-form tests passing and a cut-off loop at every line break that left HOME empty and ran no `uname`, `mktemp` or `curl`; it did not cut at every byte, and UAT found two byte cuts that ran the old `main` with no arguments. `a_cut_off_script_runs_nothing` now cuts at every byte under both shells (criterion 3). The checksum check, version folders and link were not rewritten: plan 1's report says `git diff -w` on the wrapper commit shows only the wrapper and header comment, and the Phase 4 tests pass against the flat names under both shells (criterion 4). Plan 2's report records `sh -n` and `dash -n` exiting 0 and `cargo test --workspace --all-features` passing, 812 passed and 0 failed (criterion 5). UAT also found that dash 0.5.13 started without a UTF-8 locale changes eleven UTF-8 continuation bytes whenever it copies a value, which reached every path built from HOME; install.sh now refuses the run when its shell does that, and `a_home_dash_would_change_is_refused` covers `/bin/sh`, dash, and dash with `LANG=C.UTF-8`.
