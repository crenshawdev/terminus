---
phase: 13
plan: 1
requirements: ["#47", "#44"]
files:
  - crates/terminus-core/Cargo.toml
  - crates/terminus/Cargo.toml
  - .github/workflows/test.yml
  - README.md
  - SECURITY.md
---

# Phase 13: CI and Data Integrity - Plan 1

## Goal

The phase goal: the test suite runs on GitHub for every push and pull request, and the archive and decision-log fixes (#15, #40, #38, the drain race, and #17's second case) land under it, so no known path writes wrong bytes into the archive or loses a decision's data.

This plan puts the suite in CI. A new workflow runs `cargo test --workspace --all-features --locked` on all four release runners (#47), and the 27 test files gated as a whole on `testkit` are declared with `required-features`, so naming one without the feature is refused instead of passing with zero tests (#44).

## Must be true when done

- On every push and pull request, a workflow runs `cargo test --workspace --all-features --locked` on `ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-15-intel` and `macos-15`, and a failing test fails that runner's job.
- If a tool the shell tests need is missing on a runner, that job fails in a step before `cargo test` starts, and the step names the tool.
- `cargo test -p terminus --test mcp`, and the same command for each of the other 26 whole-file testkit test files, is refused by cargo with a message naming `testkit`.
- `cargo test --workspace --all-features` still builds and runs every test file under `crates/*/tests/`, and the four mixed files and `crates/terminus/tests/hook.rs` still run without the feature.
- `README.md` and `SECURITY.md` no longer say that no workflow runs the tests, and the README still gives `cargo test --workspace --all-features` as the local command.

## Context

- Locked: D-01 to D-07 in `.planning/phases/13/CONTEXT.md`. The workflow copies `release.yml`'s conventions. It never uses `--release` and never loosens a timing budget (D-04). Exactly 27 files get the declaration (D-05). The `--all-features` notes stay (D-06). The docs change in the same commit as the workflow, and branch protection is not touched (D-07).
- Out of scope: making the workflow a required status check, sentinel's `full.yml` (the owner does not want it), and any workflow feature no decision names, such as caching.
- This plan shares no file with plans 2 to 4. It runs first because the other three are meant to land with CI already in place.
- Prose in `README.md` and `SECURITY.md` follows `~/.claude/john-voice.md`. That is the user's global rule for anything published under his name, and the file is read in full before writing.

## Tasks

### Task 1: Declare the 27 whole-file testkit test targets with `required-features`

- **Files:** crates/terminus-core/Cargo.toml, crates/terminus/Cargo.toml
- **Action:** Add a `[[test]]` entry with `required-features = ["testkit"]` for each test file gated as a whole by a `#![cfg(feature = "testkit")]` line, and for no other file (D-05). `crates/terminus-core/Cargo.toml` gets the 24 files `backfill`, `capture`, `compaction`, `derive`, `feedback`, `fixtures`, `index`, `ingest`, `inject_brief`, `inject_compaction`, `inject_prompt`, `label`, `lineage`, `observe`, `parse`, `pass`, `recall_golden`, `recall`, `recover`, `redacted_recall`, `reindex`, `retention`, `snapshot` and `verify`. `crates/terminus/Cargo.toml` gets `datamove`, `mcp` and `recall_cli`. Each entry's name is the file stem and its path is that file under `tests/`, so the names `cargo test --test <name>` takes do not change. Leave auto-discovery on. The mixed files (`discover.rs`, `egress.rs`, `schema.rs` and `store_open.rs` in terminus-core) and `crates/terminus/tests/hook.rs`, which gates only its kill test, stay auto-discovered and get no declaration, because they hold tests that run without the feature. `hook.rs`'s header explains why its budget test must not be gated. Every `#![cfg(feature = "testkit")]` line stays where it is: the declaration goes beside it, not in place of it. Put a short comment above each manifest's block. It says the entries make cargo refuse a named run without the feature instead of reporting zero tests as a pass (#44), and that a package-wide `cargo test -p` still skips these targets silently, which is why `--all-features` stays the documented command (D-06). Leave crates/terminus/Cargo.toml's `[dev-dependencies]` line that turns on terminus-core's `testkit` exactly as it is.
- **Verify:** For each of the 24 terminus-core names, `cargo test -p terminus-core --test <name>` exits non-zero and its stderr names `testkit`. The same holds for `cargo test -p terminus --test datamove`, `--test mcp` and `--test recall_cli`. `cargo test -p terminus-core --test discover --no-run` and `cargo test -p terminus --test hook --no-run` exit 0. `grep -c 'required-features = \["testkit"\]'` prints 24 for crates/terminus-core/Cargo.toml and 3 for crates/terminus/Cargo.toml. `cargo test --workspace --all-features --no-run` lists one test executable for every file under `crates/terminus-core/tests/*.rs` (36) and `crates/terminus/tests/*.rs` (27). `cargo test --workspace --all-features` passes.

### Task 2: Run the whole suite on every push and pull request on the four release runners, and stop the docs saying no workflow does

- **Files:** .github/workflows/test.yml, README.md, SECURITY.md
- **Action:** Create a test workflow of its own, separate from `release.yml`, triggered on `push` and `pull_request` (D-01). It follows `release.yml`'s conventions:
  - top-level `permissions: contents: read`;
  - every action pinned by full commit SHA with its version in a trailing comment, reusing the checkout pin `release.yml` already carries;
  - `persist-credentials: false` on checkout;
  - `defaults: run: shell: bash`, with `set -euo pipefail` in each multi-line step;
  - `RUST_VERSION: "1.97.1"`, installed with rustup's minimal profile and made the default the way `release.yml`'s "Install the named Rust version" step does it;
  - `--locked`.

  The workflow reads no secret and sets no corpus variable, so `corpus.rs` skips as it does on any machine without the private corpus. The matrix runs on all four release runners, `ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-15-intel` and `macos-15` (D-02), with `fail-fast: false` as in `release.yml`, so one platform's failure does not hide another's. It builds for the host target only. The musl toolchain is for release builds and is not needed here.

  Before `cargo test`, one step installs what the runner lacks, and the next step checks every tool the shell tests name (D-03). If any tool is missing, the check fails and names it. The set is install_script.rs's `TOOLS` (`od`, `uname`, `mktemp`, `rm`, `curl`, `sha256sum`, `mkdir`, `chmod`, `mv`, `ln`, `readlink`, `grep`, `ssh-keygen`), plus sign_release.rs's `TOOLS` (`mktemp`, `rm`, `grep`, `sha256sum`), plus the programs those two files look up by name: `dash`, `shasum`, `openssl` and `cp`. Both files find tools by scanning `PATH` (their `which` helpers), so a tool counts only if it is on `PATH` under that exact name. For `openssl`, the check also confirms that `openssl s_server` offers `-WWW` and that `openssl req` accepts the `-addext` flag `Server::start` in install_script.rs passes. The macOS images ship LibreSSL as `/usr/bin/openssl`. If that one fails the check, install Homebrew's OpenSSL and put its `bin` directory first on `PATH` through `GITHUB_PATH`. On macOS, also install `dash`, and a `sha256sum` under that exact name, through Homebrew when either is absent. Key the install steps on `runner.os`. Do not change any test to tolerate a missing tool: both files fail rather than skip by design (their headers say so).

  Then run `cargo test --workspace --all-features --locked` on the debug build. Use no `--release`, and no variable or flag that loosens a timing budget (D-04). `hook.rs`'s p99 budget and the budgets in `backfill.rs`, `lock_race.rs` and `corpus.rs` keep their numbers.

  Change the docs in the same commit (D-07). README.md's "Current state" bullet saying the tests do not run in CI (#47) goes. The "How it is built" bullet stops saying no workflow runs the tests. It says the test workflow runs the whole suite on every push and pull request on the four platforms. It keeps "No status check is required", because this phase does not change branch protection, and it keeps the local command `cargo test --workspace --all-features` (D-06). In SECURITY.md, the "Every pull request" part of "How Terminus is checked" gains a bullet for the test workflow, in the style of the `fast.yml` bullet beside it. The bullet says what the workflow runs, where it runs, and that a failing test fails it. Read `~/.claude/john-voice.md` in full before writing either file.
- **Verify:** `cargo test -p terminus --test release_workflows` passes with the new file in `.github/workflows`, which shows it reads no secret and signs nothing. Grepping the new file shows all four runner names, `cargo test --workspace --all-features --locked`, `RUST_VERSION: "1.97.1"`, `persist-credentials: false`, `contents: read`, and both the `push` and `pull_request` triggers. It also shows every `uses:` value ending in `@` and 40 hex characters, and no `--release`. Copy the tool-check step's script into a scratch file and run it with bash twice. With the normal `PATH` it exits 0. With `PATH` set to a scratch directory of links to every listed tool except `dash`, it exits non-zero and prints `dash`. `grep -n -e "no workflow runs" -e "do not run in CI" README.md SECURITY.md` prints nothing, and README.md still contains `cargo test --workspace --all-features`. human-verify (needs GitHub Actions): after the branch is pushed, the workflow runs on all four runners for the push and for the pull request, the tool-check step runs before the `cargo test` step, and all four jobs pass. If a timing-budget test fails on a hosted runner, stop and bring the test name and its measured number to the owner. Do not loosen the budget or switch to `--release` (D-04).

## Notes

- Two CONTEXT assumptions can only be settled by a real run, and Task 2's human-verify is where that happens. One is what the macOS images lack (`dash`, an unprefixed `sha256sum`, an `openssl` with `s_server -WWW`). The other is whether a debug-build hook meets the 10 ms p99 on a hosted runner.
- Prior art: in `v0.1.1/phases/3/SUMMARY.md` (phase 3), every task's Verify ran zero of the tests it named, because the commands left out `--features testkit`. Task 1 turns that mistake into a refusal from cargo. Every Verify in plans 2 to 4 names `--features testkit` or `--all-features` for a gated file.
- AC7: the pull request carrying phase 13 must close #47 and #44, and #15, #40, #38 and #17 from plans 2 to 4, with closing keywords. The orchestrator owns git and the pull request.
- The owner reviews README.md and SECURITY.md himself. Open both in RustRover once they are written.
