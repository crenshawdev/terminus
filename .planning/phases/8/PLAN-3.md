---
phase: 8
plan: 3
requirements: [REL-01]
files:
  - scripts/sign-release.sh
  - crates/terminus/tests/sign_release.rs
---

# Phase 8: Release Builds - Plan 3

## Goal

The phase goal: a tagged version produces a GitHub release that Phase 6's `install.sh` accepts. A script on the owner's machine downloads the four manifests, signs each with `ssh-keygen -Y sign -n terminus-release`, uploads the `.sig` files and publishes the release.

This plan writes that script and its tests. Before signing, it rechecks each binary against its manifest. Before uploading anything, it verifies each signature exactly as `install.sh` will. Only when all four pass does it upload the signatures and publish the release as latest.

## Must be true when done

- **A good draft is signed and published.** `sh scripts/sign-release.sh v0.1.0` does these steps in order:
  - downloads the draft's eight files
  - checks each binary's SHA-256 against its manifest
  - signs each manifest with `ssh-keygen -Y sign -f ~/.ssh/crenshawdev-release -n terminus-release`, manifest on stdin and signature on stdout
  - verifies each signature with `install.sh`'s exact command, against the key it reads from `install.sh`'s `release_key=` line
  - uploads the four `.manifest.sig` files
  - publishes the release as latest, not as a prerelease
- **A bad draft is refused (AC3).** When any of the eight files is missing, when a binary does not match its manifest, or when a signature does not verify against the key pinned in `install.sh`:
  - the script exits non-zero
  - it uploads nothing and the release stays a draft
  - in the first two cases nothing is signed
  - Tests show each case with a stand-in `gh` and a throwaway key, under `/bin/sh` and dash.
- **The script holds no copy of the key (D-12).** The pinned base64 does not appear in `scripts/sign-release.sh`.
- **No test can reach the real release or the real key.** Every test runs with a cleared environment, a temporary HOME, and a `PATH` that holds no real `gh`.
- **The gates hold.** `sh -n` and `dash -n` pass on the script, `cargo test --workspace --all-features` passes, and clippy is clean.

## Context

- Plan 3 of 4. It shares no file with plans 1 and 2. Plan 4 runs this script on the real draft.
- D-12 to D-16 in `.planning/phases/8/CONTEXT.md` bind this plan. D-13 means one passphrase prompt per manifest and no agent.
- **The script follows `install.sh`'s style:** `#!/bin/sh`, `set -eu`, `LC_ALL=C`, a `fail` that prints `terminus: <reason>` on stderr, a `mktemp -d` folder removed by `trap ... 0`, and `digest`'s `sha256sum`-or-`shasum -a 256` choice.
- **The tests follow the harness in `crates/terminus/tests/install_script.rs`:**
  - `TOOLS` and `Fixture::tools_folder`, for a `PATH` built from links to named system tools
  - `Shell` and `both_shells!`, for `::sh` and `::dash`
  - `fixture`'s throwaway Ed25519 key and its `PINNED_KEY` swap into a copy of `install.sh`
  - `Fixture::sign`, for signing from stdin to stdout
  - A new test file cannot import from that one, so copy the small parts it needs.
- **gh finds a draft by its tag (checked 2026-10-14).** In gh 2.102.0, `release download`, `upload` and `edit` all call `shared.FetchRelease`, and that falls back to a draft lookup by pending tag name (cli/cli v2.102.0, `pkg/cmd/release/shared/fetch.go`). So the script names the release by its tag, which settles that CONTEXT flagged assumption.
- **This repo has two remotes,** `origin` and `forgejo`, so every `gh` call names `--repo crenshawdev/terminus`, and gh never has to choose one.
- CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing the script's comments, the test doc comments or commit messages.
- Out of scope: the workflows, Baley's signing, an `ssh-agent` (D-13), and a key or key path given by argument or environment.

## Tasks

### Task 1: Sign, verify and publish a good draft

- **Files:** scripts/sign-release.sh, crates/terminus/tests/sign_release.rs
- **Action:**
  - **Usage.** The script runs as `sh scripts/sign-release.sh v<X.Y.Z>`. Its one argument is the tag, and it refuses anything that is not `v` followed by major.minor.patch.
  - **Key (D-12).** It reads `install.sh` from the repo root, found relative to the script's own folder as `<script folder>/../install.sh`. It takes the value of the single `release_key='...'` line, and fails unless there is exactly one such line. The key text appears nowhere in the script.
  - **Work folder.** The script works in a `mktemp -d` folder that its exit trap removes.
  - **Download (D-15).** It fetches the release's files into that folder with `gh release download <tag> --repo crenshawdev/terminus --dir <folder>`.
  - **Recheck (D-15).** For each of the four platforms, the binary's SHA-256 must equal the value on its manifest's `sha256 ` line.
  - **Sign (D-12, D-13).** For each manifest, run `ssh-keygen -Y sign -f "$HOME/.ssh/crenshawdev-release" -n terminus-release` with the manifest on stdin and stdout into `<manifest>.sig`.
    - Never use the form that names the file. Once a `.sig` exists, that form stops at "Overwrite (y/n)?".
    - ssh-keygen asks for the passphrase on the terminal, once per manifest.
  - **Verify (D-14).** Write `crenshawdev-release <key>` into an `allowed_signers` file in the work folder. For all four manifests, run `ssh-keygen -Y verify -f <that file> -I crenshawdev-release -n terminus-release -s <sig>` with the manifest on stdin. Every check runs before anything is uploaded.
  - **Upload and publish (D-14, D-16).** Only after all four verify:
    - Run `gh release upload <tag> <the four .manifest.sig files> --repo crenshawdev/terminus --clobber`. `--clobber` lets a rerun after a failed publish replace its own earlier signatures.
    - Then run `gh release edit <tag> --repo crenshawdev/terminus --draft=false --prerelease=false --latest`.
  - **Failures.** Every failure prints the script's own reason on stderr and exits non-zero.
  - **Harness: layout.** Each test gets a fixture with a temporary root holding:
    - **HOME,** with `.ssh/crenshawdev-release` set to a throwaway Ed25519 key that has no passphrase, made the way `fixture` in `install_script.rs` makes one.
    - **The script and `install.sh`,** as `<root>/scripts/sign-release.sh` beside `<root>/install.sh`. The `install.sh` copy is the shipped file with `PINNED_KEY` swapped for the throwaway key's public line. A test can ask for the unmodified file instead.
    - **A draft folder,** with four binaries of distinct arbitrary bytes and their four correct manifests at version 0.1.0.
  - **Harness: stand-in `gh`.**
    - It answers `release download` from the draft folder.
    - It copies `release upload`'s files into an uploaded folder.
    - It logs every call with its arguments.
  - **Harness: `ssh-keygen` wrapper.** In the tools folder, `ssh-keygen` is a small wrapper that logs each call's arguments and then runs the system one. Task 2 relies on that log.
  - **Harness: environment.** The environment is cleared first, and HOME is the fixture's. `PATH` holds the stand-in's folder, then the tools folder, and nothing else. The real `gh` is logged in as crenshawdev, so no test may ever reach it.
  - **Harness: shells.** Each test runs as `::sh` and `::dash`. A machine without dash or ssh-keygen fails these tests rather than skipping them.
  - **The success test** asserts all of these:
    - The script exits 0.
    - The `ssh-keygen` log holds four `-Y sign` calls, each with `-f <HOME>/.ssh/crenshawdev-release -n terminus-release` and no file argument, and four `-Y verify` calls, each with `-I crenshawdev-release -n terminus-release`.
    - The stand-in logged one upload naming exactly the four `.manifest.sig` files, and after it one edit carrying `--draft=false` and `--latest`.
    - Every `gh` call carried `--repo crenshawdev/terminus`.
    - Each uploaded signature verifies against the throwaway key with `install.sh`'s command.
- **Verify:**
  - `sh -n scripts/sign-release.sh` and `dash -n scripts/sign-release.sh` exit 0.
  - `cargo test -p terminus --test sign_release` passes, and the test runs as `::sh` and `::dash`.
  - `grep -c 'AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q' scripts/sign-release.sh` prints `0`.
  - Every flag the script passes to `gh release download`, `gh release upload` and `gh release edit` appears in that subcommand's `--help` output from the installed gh 2.102.0.
  - Temporarily changing the script's signing namespace to `baley-release` makes the success test fail. Restored, it passes.

### Task 2: Refuse an incomplete draft or a binary that does not match its manifest

- **Files:** crates/terminus/tests/sign_release.rs, scripts/sign-release.sh
- **Action:**
  - **Missing file (AC3).** Write one test that loops over the eight names. For each name, it removes that one file from the draft and runs the script. Each run must:
    - exit non-zero, with stderr naming the missing file
    - log no `ssh-keygen -Y sign` call
    - log no `gh release upload` and no `gh release edit`
  - **Binary mismatch (AC3, D-15).** Append bytes to one binary and leave its manifest unchanged. The run must exit non-zero, with stderr naming that binary and the SHA-256 mismatch, and with no sign, upload or edit call logged.
  - **Order.** Change the script only where these tests show it is needed. Its order is download, then the eight-file check, then the recheck, then signing. A refused draft never asks the owner for a passphrase.
- **Verify:**
  - `cargo test -p terminus --test sign_release` passes, and each new test runs as `::sh` and `::dash`.
  - Temporarily moving the recheck after the signing loop makes the mismatch test fail on the sign log. Restored, it passes.

### Task 3: Refuse a signature that does not verify against the pinned key

- **Files:** crates/terminus/tests/sign_release.rs, scripts/sign-release.sh
- **Action:**
  - **The test (AC3, D-14).** Run the script beside the unmodified `install.sh`, which pins the real crenshawdev key, with the throwaway key at `$HOME/.ssh/crenshawdev-release`. The signatures are made, but none verifies against the pinned key. The run must:
    - exit non-zero, with stderr saying a signature does not verify against the key pinned in `install.sh`
    - log no `gh release upload` and no `gh release edit`
  - **The script.** Change it only where this test shows it is needed.
- **Verify:**
  - `cargo test -p terminus --test sign_release` passes, as `::sh` and `::dash`.
  - Temporarily making the script's verify step always succeed makes this test fail on the upload log. Restored, it passes.
  - The phase gate holds:
    - `sh -n scripts/sign-release.sh` and `dash -n scripts/sign-release.sh` exit 0.
    - `cargo test --workspace --all-features` passes.
    - `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **Where the script lives.** `scripts/sign-release.sh` is a new folder. It keeps a maintainer's script apart from `install.sh`, which users fetch. That is the planner's call; CONTEXT says only that the script is in this repo.
- **Recalled prior art.** Signing from stdin to stdout and the throwaway-key swap come from Phase 6 D-02 and D-13 (`.planning/phases/6/CONTEXT.md`, phase 6). The phase 6 UAT showed that the unmodified `install.sh` refuses a release signed by a throwaway key (`.planning/phases/6/UAT.md`, phase 6), and Task 3 relies on that.
