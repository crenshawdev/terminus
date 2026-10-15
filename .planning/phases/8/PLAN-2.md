---
phase: 8
plan: 2
requirements: [REL-01]
files:
  - .github/workflows/release.yml
  - .github/workflows/release-check.yml
  - crates/terminus/tests/release_workflows.rs
---

# Phase 8: Release Builds - Plan 2

## Goal

The phase goal: a tagged version produces a GitHub release that Phase 6's `install.sh` accepts. CI builds `terminus-<os>-<arch>` for Linux and macOS on x86_64 and aarch64, writes each manifest and attaches both to a draft release. CI never holds the key.

This plan writes the two workflows. The release workflow builds the four platforms when a `v<X.Y.Z>` tag is pushed, and it makes one draft when all four pass. The check workflow installs the published release with the one-line command on the same four runners. A test keeps every key and signing step out of both.

## Must be true when done

- **A version tag builds four native binaries.** `.github/workflows/release.yml` runs only on a pushed tag matching `v[0-9]+.[0-9]+.[0-9]+`. It builds with `cargo build --release --locked -p terminus --target <triple>` and no features, under a named Rust version:
  - `x86_64-unknown-linux-musl` on `ubuntu-24.04`
  - `aarch64-unknown-linux-musl` on `ubuntu-24.04-arm`
  - `x86_64-apple-darwin` on `macos-15-intel`
  - `aarch64-apple-darwin` on `macos-15`
- **A wrong build makes no draft.** A build job fails when the binary's `--version` differs from the tag's version, when `file` does not call a Linux binary static, or when `otool -L` lists a library outside `/usr/lib/`. In any of these cases no draft is made.
- **One job makes one draft of eight files.** It runs after all four builds pass, it is the only job with `contents: write`, and it uses `github.token`. Each manifest is exactly `version X.Y.Z\n` then `sha256 <64 lowercase hex>\n`.
- **Publishing starts the check.** `.github/workflows/release-check.yml` runs on `release: published`, on the same four runners. It puts a stand-in `claude` on `PATH`, runs `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`, and requires `~/.local/bin/terminus --version` to print the release's version.
- **No workflow touches a key (AC5).** No workflow file contains `secrets.`, `secrets[`, `-Y sign` or `PRIVATE KEY`, a test fails when one does, and the repository has no Actions secrets.
- **Every action is pinned.** Every `uses:` names a full 40-character commit SHA with its version in a comment.

## Context

- Plan 2 of 4. It shares no file with plans 1 and 3. Plan 4 pushes the tag and publishes the release that first run these workflows.
- D-03, D-05 to D-11, D-16 to D-18 and D-20 in `.planning/phases/8/CONTEXT.md` bind this plan.
- **The model is Baley's `test.yml`** (`/code/baley/.github/workflows/test.yml`):
  - top-level `permissions: contents: read`
  - `uses:` pinned by SHA with a `# vX.Y.Z` comment
  - the toolchain step in its shape, with `set -euo pipefail` and `rustc --version`
  - a why-comment above each step that needs one
- CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing those comments or commit messages.
- **Runners, checked first as CONTEXT asked (2026-10-14).** `data/reusables/actions/supported-github-runners.md` in github/docs lists these as free for public repositories: `ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-15-intel` (Intel) and `macos-15` (arm64). The four images' readmes in actions/runner-images list rustup and gh on each, and openssh-client 9.6p1 on both Ubuntu images.
- **Pins, resolved 2026-10-14 with `gh api`:**
  - `actions/checkout` v7.0.1 = `3d3c42e5aac5ba805825da76410c181273ba90b1`, the same pin Baley uses
  - `actions/upload-artifact` v7.0.2 = `cf430e030ddbb5b0abf93d22962f4752f3646cd9`
  - `actions/download-artifact` v8.0.2 = `9000827ccba6bdab643e8b6fd33ac0654aef8333`
- **No local workflow linter.** This machine has no actionlint, PyYAML, ruby or yq, so the checks below read the files and grep them. The first real run is plan 4's human-verify.
- Out of scope: running the test suite in CI (D-20), a build cache, Windows, and any key, secret or signing step.

## Tasks

### Task 1: Build the four platforms when a version tag is pushed

- **Files:** .github/workflows/release.yml
- **Action:**
  - **Trigger (D-03).** `on: push: tags:` holds the one pattern `v[0-9]+.[0-9]+.[0-9]+`, and there is no other trigger. In GitHub's filter syntax `+` means one or more of the preceding character, and the docs' own example is `v[12].[0-9]+.[0-9]+`.
  - **Permissions (D-09).** The top level sets `permissions: contents: read`.
  - **Matrix (D-06).** One build job has a matrix of four entries. Each entry names its runner, its Rust target and its asset name, exactly as listed in "Must be true when done". The asset names are `terminus-linux-x86_64`, `terminus-linux-aarch64`, `terminus-macos-x86_64` and `terminus-macos-aarch64`.
    - Every platform builds natively on its own runner, with no cross-compiling tool.
    - Set `fail-fast: false`, so one push shows every platform's result. This is the planner's call: D-19's macOS failures then all show up in one run.
  - **Checkout.** Use the pinned checkout with `persist-credentials: false`, because no step uses git.
  - **Rust (D-09).** Name the version `1.97.1` once in the workflow. That is the compiler this machine's `rustc --version` reports, and the one the suite passes on.
    - Install it in Baley's step shape: `rustup toolchain install <version> --profile minimal --target <triple>`, then `rustup default <version>`, then print `rustc --version`.
    - Do not add a `rust-toolchain.toml`. It would pin the toolchain for every local `cargo` command in this repo, and D-09 asks only that the version be named.
  - **musl C toolchain (Linux only).** Install `musl-tools` with apt. For the build step, set `CC_x86_64_unknown_linux_musl` or `CC_aarch64_unknown_linux_musl` to `musl-gcc`.
    - Why: `cc` 1.4.2, the version in `Cargo.lock`, maps `aarch64-unknown-linux-musl` to `aarch64-linux-musl-gcc` (`prefix_for_target`), and Ubuntu's `musl-tools` installs only `musl-gcc`.
    - The C code in zstd-sys, libsqlite3-sys and libmimalloc-sys must compile against musl's headers.
  - **Build (D-08).** The build is `cargo build --release --locked -p terminus --target <triple>` and nothing else:
    - no `--features`, `--all-features` or `--no-default-features`
    - no cache action, so a release binary is built from source on a clean runner (planner's call)
  - **Version check (D-07).** Run the built binary with `--version`. Its output must equal the tag name without its leading `v`, read from the runner's `GITHUB_REF_NAME` variable, never from a `${{ }}` expression inside the script. On a mismatch, fail with an `::error::` line that names both versions.
  - **Linkage check (AC2, D-01).** Print the output of `file` (Linux) or `otool -L` (macOS) to the log either way.
    - On Linux, fail unless `file` says `statically linked` or `static-pie linked`. Rust's musl targets build a static PIE by default.
    - On macOS, fail when any library line from `otool -L` is outside `/usr/lib/`. Apple does not support fully static binaries.
  - **Manifest (D-05, D-07).** Copy the binary to its asset name. Write `<asset>.manifest` as exactly `version <v>\n` then `sha256 <hex>\n`.
    - `<v>` is the `--version` output.
    - `<hex>` is only the first field of `sha256sum` on Linux or `shasum -a 256` on macOS, never the file name those tools print after it.
  - **Artifact.** Upload the binary and its manifest as one artifact named for the asset, with `if-no-files-found: error`.
  - **Shell.** Every `run:` either starts with `set -euo pipefail`, or the job's default shell is `bash`. GitHub runs an explicit `shell: bash` as `bash -eo pipefail`.
- **Verify:**
  - `grep -n 'uses:' .github/workflows/release.yml | grep -vE '@[0-9a-f]{40} # v'` prints nothing.
  - `grep -nE -- '--(all-|no-default-)?features|cargo test|nextest|rust-cache' .github/workflows/release.yml` prints nothing.
  - `grep -c '1\.97\.1' .github/workflows/release.yml` prints `1`.
  - Reading the file shows all of these:
    - The only trigger is the tag push with `v[0-9]+.[0-9]+.[0-9]+`, and top-level `permissions` is `contents: read`.
    - The four matrix entries are exactly as listed in "Must be true when done", with `fail-fast: false`.
    - Each build job has a version check, a linkage check and a manifest step, after the build and before the upload.
  - human-verify: the first real run is plan 4 Task 2 (AC1, AC2).

### Task 2: Make one draft after all four builds pass

- **Files:** .github/workflows/release.yml
- **Action:**
  - **The job (D-09, D-10).** Add a second job that `needs` the build job, runs on `ubuntu-24.04`, and is the only job with `permissions: contents: write`. `needs` waits for every matrix entry, so a failed platform leaves no draft.
  - **Gather the files.** Download every build artifact into one folder, using the pinned download-artifact with a pattern for the four asset names and the artifacts merged into that one folder.
  - **Check the eight (AC1).** Before creating anything, require the folder to hold exactly the four `terminus-<os>-<arch>` files and their four `.manifest` files, and nothing else. Fail with a line naming what is missing or extra.
  - **Create the draft (D-11, D-16).** Use the image's `gh`, with `GH_TOKEN` set to `${{ github.token }}`.
    - Run `gh release create` on the tag with `--draft`, `--verify-tag` and `--repo "$GITHUB_REPOSITORY"`, attaching the eight files.
    - Never pass `--prerelease`.
    - The title is the tag, and the body is left empty. That is the planner's call: `CHANGELOG.md` is the release record, and no decision sets release text.
    - No checkout, and no third-party release action.
- **Verify:**
  - `grep -c 'contents: write' .github/workflows/release.yml` prints `1`, and reading shows it sits under the draft job.
  - `grep -n 'secrets' .github/workflows/release.yml` prints nothing, and `grep -n 'github.token' .github/workflows/release.yml` finds the draft job's `GH_TOKEN`.
  - Reading shows the draft job `needs` the build job, the create line carries `--draft` and `--verify-tag`, and `--prerelease` appears nowhere.
  - The Task 1 pin check still prints nothing.
  - human-verify: plan 4 Task 2.

### Task 3: Install the published release on the four runners

- **Files:** .github/workflows/release-check.yml
- **Action:**
  - **Trigger (D-17).** `on: release: types: [published]`, and nothing else. It fires because the owner's own `gh` login publishes the release. A release published with a workflow's `GITHUB_TOKEN` would start no workflow.
  - **Permissions and checkout.** The top level sets `permissions: contents: read`, and no job raises it. There is no checkout, because the check tests the public command, which fetches `install.sh` from `main` (D-02).
  - **Runners (D-06).** Use the same four runner labels as the release workflow, with `fail-fast: false`.
  - **Shell.** Set the default shell to `bash` explicitly. Then pipefail makes a failed `curl` inside `curl ... | sh` fail the step, which settles that CONTEXT flagged assumption. A `run:` with no `shell:` runs as `bash -e {0}`, without pipefail.
  - **Stand-in `claude` (D-18).** Before the install step, write a `claude` that only exits 0 into a folder under `$RUNNER_TEMP`, make it executable, and add the folder to `$GITHUB_PATH`.
    - Why: `terminus install` exits non-zero when it cannot register the MCP server.
    - A probe on 2026-10-14 ran this machine's release build of `terminus install` with an empty HOME and a `claude` that only exits 0. It exited 0.
  - **Install.** Run exactly the command `CHANGELOG.md` documents: `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`.
  - **Version.** Run `"$HOME/.local/bin/terminus" --version` by its full path. Its output must equal the release's tag without its leading `v`.
    - Pass the tag from `github.event.release.tag_name` through `env:`, never by `${{ }}` inside the script.
    - On a mismatch, fail with an `::error::` line.
- **Verify:**
  - `grep -n 'uses:' .github/workflows/release-check.yml | grep -vE '@[0-9a-f]{40} # v'` prints nothing.
  - `grep -cF 'curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh' .github/workflows/release-check.yml` prints `1`.
  - `diff <(grep -oE '(ubuntu|macos)-[0-9][0-9.a-z-]*' .github/workflows/release.yml | sort -u) <(grep -oE '(ubuntu|macos)-[0-9][0-9.a-z-]*' .github/workflows/release-check.yml | sort -u)` prints nothing.
  - Reading shows the only trigger is `release` `published`, `contents: read` is the only permission, the default shell is `bash`, the stand-in step comes before the install step, and the version step runs `~/.local/bin/terminus` by full path.
  - human-verify: plan 4 Task 3 (AC4).

### Task 4: Fail a test when a workflow names a secret or a signing step

- **Files:** crates/terminus/tests/release_workflows.rs
- **Action:**
  - **The test (D-11, AC5).** Write a plain `#[test]`, modelled on `install_sh_pins_the_release_key_once` in `crates/terminus/tests/install_script.rs` and `install_sh_holds_the_same_marker` in `crates/terminus/src/cmd/install/binary.rs`.
    - It reads every file in `.github/workflows/` at the repo root, reached from `CARGO_MANIFEST_DIR` as `../../.github/workflows`.
    - It fails when any of them contains `secrets.`, `secrets[`, `-Y sign` or `PRIVATE KEY`, naming the file and the line. Both `secrets.X` and `secrets['X']` read a secret, so the test matches both forms.
  - **No silent pass.** It also fails when the folder holds no `.yml` file, so a moved or renamed folder cannot pass by reading nothing.
  - **The doc comment** says what the test guards and why: CI never holds the key, and the draft is made with `github.token`.
  - **Where it lives.** The test goes in its own file rather than `install_script.rs`, because it is about the workflows, not `install.sh`. It needs no new dependency.
- **Verify:**
  - `cargo test -p terminus --test release_workflows` passes.
  - Four temporary edits each make it fail, with a message naming `release.yml` and the line: a comment line holding `secrets.X`, one holding `secrets['X']`, one holding `-Y sign`, and one holding `PRIVATE KEY` added to `release.yml`. Temporarily renaming `.github/workflows` also makes it fail. Restored, it passes.
  - `gh api repos/crenshawdev/terminus/actions/secrets --jq .total_count` prints `0`. This is a read, and gh is logged in as crenshawdev.
  - `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Notes

- **Flagged assumptions settled in this plan.**
  - The Intel macOS and arm64 Linux runners exist and are free for public repositories (see Context).
  - The check sets `bash` explicitly, so a failed `curl` in the pipe fails the step.
  - The musl C compiler is named through `CC_<target>`, so the aarch64 build does not depend on cc's cross-prefix guess.
- **Still open until the first real run.** Plan 4 surfaces each of these, and D-19 makes a failure this phase's fix:
  - whether zstd-sys, libsqlite3-sys and mimalloc build for musl on the arm64 runner
  - whether macOS's `ssh-keygen -Y verify` behaves as OpenSSH 10.6p1 does
  - whether the arm64 macOS binary runs with only the linker's ad-hoc signature
- **Recalled prior art.** The asset and manifest names come from Phase 5 D-01 (`.planning/phases/5/CONTEXT.md`, phase 5). The check's one-line command is the one Phase 5's UAT ran piped into `sh` (`.planning/phases/5/UAT.md`, phase 5).
