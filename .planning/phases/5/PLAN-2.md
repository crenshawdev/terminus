---
phase: 5
plan: 2
requirements: [INST-13]
files:
  - crates/terminus/tests/install_script.rs
  - install.sh
  - CHANGELOG.md
  - DESIGN-BRIEF.md
---

# Phase 5: Install from GitHub Releases - Plan 2

## Goal

The one-line install, `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`, runs under dash on Debian and Ubuntu. This plan qualifies `install.sh` under dash as well as `/bin/sh`, and brings `CHANGELOG.md` and `DESIGN-BRIEF.md` in line with the script plan 1 finished.

## Must be true when done

- **Every install.sh test runs under both shells.** Each test in `install_script.rs` runs once under `/bin/sh` and once under dash, as separate tests named by their shell, and all of them pass. `sh -n install.sh` and `dash -n install.sh` both exit 0.
- **A missing dash is a failure.** Where dash cannot be found, every dash test fails with a message naming dash. None passes, none is skipped, and none falls back to `/bin/sh`.
- **`valid_utf8` agrees across shells.** It gives the same verdict under both shells on valid and invalid UTF-8, including strings holding the bytes 0x81 to 0x88, and each verdict is UTF-8's.
- **The cut-off check holds under dash.** A copy of `install.sh` cut off at any line break before its last line, piped into dash, runs no command from the install and changes nothing under HOME.
- **The docs match the script.** The `CHANGELOG.md` Unreleased entry and the `DESIGN-BRIEF.md` install section show three things: the curl line, the optional `https://` source passed with `sh -s --`, and the flat names. Neither mentions `<source>/<os>-<arch>/manifest`, and both still say no release is published yet.
- `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- This is plan 2 of 2, and it runs after plan 1. It builds on plan 1's harness: argument lists, the fake `curl` and the stdin pipe.
- Decisions that bind this plan: D-12 (dash) and D-13 (docs), plus the two dash assumptions in CONTEXT's Flagged assumptions. D-07 says the docs keep saying no release is published yet.
- **Owner setup before this plan runs: `sudo pacman -S dash`.** dash was absent on 2026-10-13, and Task 1 cannot pass without it, by design.
- **Docs prose.** `CHANGELOG.md` and `DESIGN-BRIEF.md` are prose the owner puts his name on. His global CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing them.
- Out of scope: the README (phase 6), `.planning/PROJECT.md`, Baley's `install.sh` (D-14), publishing a release, signing and qualifying macOS.
- Run `rustfmt --edition 2021` on the Rust files you touch, and only on those.

## Tasks

### Task 1: Run every install.sh test under dash as well as /bin/sh

- **Files:** crates/terminus/tests/install_script.rs (anchor: the module doc comment, `Fixture`, `fixture`, `which`, the command builders), install.sh (anchor: `valid_utf8`; change it only if the probe below shows dash disagrees)
- **Action:**
  - **Both shells (D-12).** Every test in this file runs once under `/bin/sh` and once under dash. Each run is a separate test with its own fixture and the shell in its name. A failure then names its shell, and `cargo test dash` selects the dash half.
    - Every way the harness starts `install.sh` uses the test's shell: as a file with arguments, through a stdin pipe, and with `-s --`.
    - The shell applies to `install.sh` only. The fake scripts' shebangs and `copy_build` stay on `/bin/sh`.
  - **A missing dash (Flagged assumptions).** Look dash up on the test process's `PATH`, the way `which` finds the tools. When it is not there, the dash test fails with a message saying dash is not installed and the install.sh tests need it. It never passes, is never `#[ignore]`d, and never falls back to `/bin/sh`.
  - **Module doc.** It now says the tests run real `sh`. Make it say they run under `/bin/sh` and dash, and why: `curl | sh` runs under dash on Debian and Ubuntu, and `/bin/sh` here is bash.
  - **Probe `valid_utf8` under dash (Flagged assumptions).** The tests feed only ASCII HOMEs, so they never reach its byte-range `case` patterns. dash uses the bytes 0x81 to 0x88 as internal control codes. Run the probe in Verify below.
    - If dash disagrees with bash or with UTF-8, rewrite `valid_utf8` in `install.sh` so both shells give UTF-8's verdicts. Keep it POSIX `sh` under `LC_ALL=C`, and keep its name and its call site.
  - **Any other dash failure.** If any other test shows `install.sh` behaving differently under dash, fix `install.sh` so both shells behave the same, and record the fix as a deviation.
- **Verify:**
  - **Setup.** `command -v dash` prints a path. If it does not, this is human-verify: the owner runs `sudo pacman -S dash`, and then the checks below run.
  - **Syntax.** `sh -n install.sh` and `dash -n install.sh` both exit 0.
  - **Both shells listed.** `cargo test -p terminus --test install_script -- --list` lists every test twice, once per shell, with the shell in its name.
  - **Both shells pass.** `cargo test -p terminus --test install_script` passes in full.
  - **A missing dash fails.**
    1. Build the tests with `cargo test -p terminus --test install_script --no-run`.
    2. Make a scratch folder of links to everything in `/usr/bin` except `dash`.
    3. Run the built test binary, filtered to dash, with `PATH` set to only that folder.
    4. Every dash test fails, and its message names dash.
  - **The `valid_utf8` probe.** Load the definition into each shell with `sed -n '/^valid_utf8()/,/^)/p' install.sh` under `LC_ALL=C`, then call it on each string below, written as octal escapes. Both shells accept the first group, refuse the second, and give identical verdicts.
    - **Accepted:** `abc`, `\303\251`, `\304\201`, `\304\210`, `\342\200\202`, `\360\237\230\200`
    - **Refused:** `\200`, `\201`, `\300\200`, `\341\200`, `\355\240\200`, `\364\220\200\200`, `\377`
  - **The cut-off check under dash.** Repeat plan 1 Task 4's cut-off loop with `dash -x -s --` in place of `/bin/sh -x -s --`, and with the same absolute scratch HOME from `h=$(mktemp -d)`. The scratch HOME stays empty, and no `+` trace line names `uname`, `mktemp` or `curl`.
  - `cargo test --workspace --all-features` passes.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

### Task 2: Bring the CHANGELOG and the design brief in line with the script

- **Files:** CHANGELOG.md (anchor: `## [Unreleased]`, `### Added`, its first entry), DESIGN-BRIEF.md (anchor: `## 13. Install and packaging`, its opening paragraph and its `Secondary:` line, and the `install.sh` bullet under `### Binary layout`)
- **Action:**
  - **Read `~/.claude/john-voice.md` in full first,** as the owner's CLAUDE.md requires for doc prose.
  - **`CHANGELOG.md` (D-13).** Rewrite the first `### Added` entry under `## [Unreleased]` in place. It should say:
    - The whole install is `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`. Keep that command on one line, so it copies whole.
    - With no argument, the script downloads `terminus-<os>-<arch>.manifest` and `terminus-<os>-<arch>` from `https://github.com/crenshawdev/terminus/releases/latest/download`. `<os>` is `linux` or `macos`, and `<arch>` is `x86_64` or `aarch64`.
    - One `https://` source, passed as `sh -s -- https://<source>`, replaces that default, and the same two names are fetched under it.

    Keep the entry's sentences on the two-line manifest, `verify_download`, the version folders, the link and the final `terminus install`. Add no `### Changed` line, because nothing has been released (D-13). Leave the "unsigned development artifacts only. No release is published" entry as it is.
  - **`DESIGN-BRIEF.md` (D-13).**
    - **Opening paragraph of §13.** The run form becomes the curl line, with the optional source passed through `sh -s --`. Keep "No release is published yet."
    - **The `Secondary:` line.** GitHub releases are now `install.sh`'s default source, not a secondary channel. Homebrew and deb/rpm stay secondary.
    - **The `install.sh` bullet under `### Binary layout`.** Name the two flat files it downloads, from the GitHub releases by default or from the given source. The rest of the bullet stays.
  - **Nothing else.** Change no other section and no other file. Add no sentence about the install failing before the first release: D-07 asks only that the docs keep saying no release is published.
- **Verify:**
  - `grep -n '<os>-<arch>/\|sh install.sh https' CHANGELOG.md DESIGN-BRIEF.md` prints nothing.
  - `grep -nF 'curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh' CHANGELOG.md DESIGN-BRIEF.md` shows at least one line in each file.
  - `grep -nF 'sh -s -- https://' CHANGELOG.md DESIGN-BRIEF.md` shows at least one line in each file.
  - `grep -nF 'terminus-<os>-<arch>.manifest' CHANGELOG.md DESIGN-BRIEF.md` shows at least one line in each file.
  - `grep -n 'No release is' CHANGELOG.md DESIGN-BRIEF.md` still shows at least one line in each file.
  - `grep -n '^Secondary:' DESIGN-BRIEF.md` no longer names GitHub releases.
  - `git diff CHANGELOG.md` changes only the first `### Added` entry under `## [Unreleased]`.

## Notes

- **The owner must install dash before this plan runs.** Without it, Task 1's dash tests fail. That is intended, because a missing dash must not read as a pass.
- **The `valid_utf8` probe is run once and not committed.** No test feeds `install.sh` a non-ASCII HOME under either shell. If the probe forces a rewrite, the owner may want a committed test. That is his call, and D-09 does not list one.
- **macOS stays unqualified,** as CONTEXT's Flagged assumptions leave it. Whether `releases/latest` skips pre-releases matters when the first release is published, not in this phase.
