---
phase: 5
status: complete
completed: 2026-10-14
---

# Phase 5: Install from GitHub Releases - Summary

`install.sh` now downloads `terminus-<os>-<arch>.manifest` and `terminus-<os>-<arch>` from `https://github.com/crenshawdev/terminus/releases/latest/download` when given no argument, takes one `https://` source in its place, runs from a pipe as one function, and is tested under both `/bin/sh` and dash.

## What shipped

- Flat release asset names for the two downloads - `install.sh:179-182`
- No argument downloads from the GitHub releases; one `https://` argument replaces it; a non-https argument or a second one is refused - `install.sh:122-126`
- The whole script body is one function, `main`, called on the last line, so a cut-off pipe runs nothing - `install.sh:121`, `install.sh:252`
- `valid_utf8` rewritten as a byte state machine over `od` output, so it behaves the same under dash and bash - `install.sh`
- Tests for the flat names, the default source, two-argument refusal, both download refusals naming the URL tried, and the piped forms; every install.sh test runs under `/bin/sh` and dash (40 tests) - `crates/terminus/tests/install_script.rs`
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

- dash 0.5.13 in a C startup locale (LANG unset, as in containers and cron) adds a 0x81 byte before any 0x82 in command-substitution and pattern-removal results. install.sh runs HOME through both (the HOME component loop in `main`, `$(mktemp "$versions/...")`, `$(readlink "$stable")`), so a HOME holding byte 0x82 (for example U+0102, `\304\202`) would build its paths with the extra byte under that shell and locale. Not touched. Debian and Ubuntu ship dash 0.5.12, which was not measured. Owner's call whether install.sh works around it or refuses such a HOME.
- No committed test feeds install.sh a non-ASCII HOME. The harness clears the environment, so dash always starts in the C locale in the tests, and the UTF-8 startup case is covered only by the one-off probe.

## Goal check

The commits plausibly deliver the phase goal. With no argument, `install.sh:123` sets the source to `https://github.com/crenshawdev/terminus/releases/latest/download`, and `install.sh:179-182` fetch `$source/terminus-$os-$arch.manifest` and `$source/terminus-$os-$arch`, which covers criteria 1 and 2 with the refusals at `install.sh:124-125`. The body is the function `main` (`install.sh:121`) called once at `install.sh:252`, and plan 1's report records the piped-form tests passing and a cut-off loop over every byte length that left HOME empty and ran no `uname`, `mktemp` or `curl` (criterion 3). The checksum check, version folders and link were not rewritten: plan 1's report says `git diff -w` on the wrapper commit shows only the wrapper and header comment, and the Phase 4 tests pass against the flat names under both shells (criterion 4). Plan 2's report records `sh -n` and `dash -n` exiting 0 and `cargo test --workspace --all-features` passing, 812 passed and 0 failed (criterion 5). The one gap is outside the stated criteria: the dash 0x82 byte issue for a non-ASCII HOME in a C locale, listed above.
