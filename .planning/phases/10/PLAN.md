---
phase: 10
plan: 1
requirements: [REL-02]
files:
  - install.sh
  - README.md
  - DESIGN-BRIEF.md
  - CHANGELOG.md
  - .planning/PROJECT.md
---

# Phase 10: Publish 0.1.0 - Plan

## Goal

The phase goal: Terminus 0.1.0 is published as the latest GitHub release, built in CI by phase 8's workflow and signed and published by the owner on his own machine, and it installs on every platform.

This plan is the live release. Phases 1 to 9 land on `main`, `v0.1.0` is tagged there, and the workflow makes the draft. The owner signs and publishes it, the check installs it on all four runners, and the owner installs it fresh on his own machine. After that, the docs stop saying no release is published.

## Must be true when done

- **`main` holds the work.** `origin/main` has `install.sh`, `README.md`, both workflows, `scripts/sign-release.sh`, and the workspace version 0.1.0.
- **The tag makes the draft (AC1, AC2).** Pushing `v0.1.0` on `main` leaves a draft release holding exactly eight files: `terminus-linux-x86_64`, `terminus-linux-aarch64`, `terminus-macos-x86_64`, `terminus-macos-aarch64` and a `.manifest` for each.
  - Each manifest is exactly `version 0.1.0\n` then `sha256 <that binary's SHA-256>\n`.
  - The build logs show `file` calling both Linux binaries static, and `otool -L` listing only `/usr/lib/` libraries for both macOS binaries.
- **The release is published and installs (AC4, AC8).** After the owner runs the signing script:
  - the release is published, not a prerelease, and `releases/latest` points at it
  - it holds twelve files: the eight plus four `.manifest.sig`
  - the check passes on all four runners, and on each one `~/.local/bin/terminus --version` prints `0.1.0`
  - on the owner's machine, a fresh install after `terminus uninstall --purge` installs 0.1.0, registers the MCP server, and rebuilds the archive with `terminus verify` clean
- **The docs say it is published (AC7).**
  - None of `README.md`, `install.sh`, `DESIGN-BRIEF.md`, `CHANGELOG.md` or `.planning/PROJECT.md` says no release is published, or that the first release is still to come.
  - The CHANGELOG has a 0.1.0 section.
  - `PROJECT.md` no longer calls `origin` self-hosted or the only remote.

## Context

- **Moved from phase 8, where it was plan 4.** The owner moved the live release out of phase 8 on 2026-10-14 so the README (phase 9) is on `main` before anyone can install. It builds on phase 8's three plans and phase 9's docs: Task 1 lands them on `main`. It starts only after phases 8 and 9 are executed and verified.
- D-02, D-03, D-10, D-13, D-16, D-17, D-19 and D-21 in `.planning/phases/8/CONTEXT.md` bind this plan. D-02's order changed with the move: the README lands before the release, not after it.
- **Who does what.** Tasks 1 to 3 are the owner's steps: landing, pushing a tag, and typing the key passphrase. They are human-verify. Task 4 is the executor's, and it runs only after Task 3's checks pass.
- **A published release cannot be taken back.** Before Task 3, everything is reversible. A failed build leaves no draft (D-10), and a tag or a draft can be deleted. After Task 3 it is not.
- CLAUDE.md requires reading `~/.claude/john-voice.md` in full before Task 4 writes doc prose or a commit message.

## Tasks

### Task 1: Land phases 1 to 9 on `main`

- **Files:** none. This is the owner's step, and no repository file changes.
- **Action:**
  - **Land (D-02).** With phases 8 and 9 executed and verified, the owner lands the integration branch on `main` with `/cad-land`, through a PR on github.com/crenshawdev/terminus.
    - Phase 8's plan 1 set `git.create_tag` to `false`, so `/cad-land` cuts no bare `0.1.0` tag (D-03).
    - The work goes to `origin`, not `forgejo`.
    - The PR description says `Fixes #13`, so issue #13 closes on merge. Commit 65ef2d1 fixed it.
  - **If the branch is reaped.** `/cad-land`'s cleanup may remove the integration branch. If it does, Task 4's edits go on a new branch from `main`.
- **Verify:** human-verify. The owner runs these, and each must hold:
  - After `git fetch origin`, `git ls-tree --name-only origin/main install.sh README.md .github/workflows/release.yml .github/workflows/release-check.yml scripts/sign-release.sh` lists all five paths.
  - `git show origin/main:Cargo.toml | grep '^version'` prints `version = "0.1.0"`.
  - `git ls-remote --tags origin 'v[0-9]*'` prints nothing, because no release tag exists yet. The pattern starts with a digit because `v*` would also match the `verbatim-v0.1.x` tags.

### Task 2: Tag `v0.1.0` on `main` and check the draft

- **Files:** none. This is the owner's step, and no repository file changes.
- **Action:**
  - **Tag (D-02, D-03).**
    - Run `git fetch origin`.
    - Run `git tag -a v0.1.0 -m v0.1.0 origin/main`. The tag is annotated, as the existing `verbatim-v0.1.0` and `verbatim-v0.1.1` tags are.
    - Run `git push origin v0.1.0`. Push it only to `origin`.
  - **The run.** The release workflow builds on the four runners and makes one draft once all four pass.
  - **If a build fails (D-19).** No draft exists, so nothing is published, and moving the tag breaks nothing.
    1. Delete the tag on `origin` and locally: `git push origin :refs/tags/v0.1.0`, then `git tag -d v0.1.0`.
    2. Fix it through a gaps plan for this phase (`/cad-plan 10 --gaps`), which declares the files the fix needs.
    3. Land the fix on `main`, and repeat this task.
- **Verify:** human-verify (AC1, AC2). The owner checks:
  - `gh release view v0.1.0 --repo crenshawdev/terminus --json isDraft,isPrerelease,assets --jq '[.isDraft, .isPrerelease, ([.assets[].name] | sort)]'` shows `true`, `false` and exactly the eight names.
  - Download the files into an empty folder with `gh release download v0.1.0 --repo crenshawdev/terminus --dir <that folder>`. In it, each `.manifest` is two lines, `version 0.1.0` and `sha256 <hex>`, each ending in a newline (`od -c` shows it). Each `<hex>` equals `sha256sum` of its binary.
  - The run's logs show `file` saying `statically linked` or `static-pie linked` for both Linux binaries, and `otool -L` listing only `/usr/lib/` paths for both macOS binaries.
  - On this machine, `file terminus-linux-x86_64` says it is static, and running it with `--version` prints `0.1.0`.

### Task 3: Sign, publish and check the release

- **Files:** none. This is the owner's step, and no repository file changes.
- **Action:**
  - **Sign and publish (D-13, D-16, D-17).** From a checkout of `main`, the owner runs `sh scripts/sign-release.sh v0.1.0` and types the key passphrase once per manifest, four times.
    - The script publishes the release as latest, not as a prerelease.
    - Publishing with the owner's own `gh` login fires `release: published`, which starts the check.
  - **If the check fails on a platform.** First find out whether the failure is in `install.sh` or in the binary.
    - A failure in `install.sh` is fixed on `main` through a gaps plan, and then the check is re-run. The check fetches `install.sh` from `main`.
    - A failure in the binary itself cannot be fixed in 0.1.0, because a published release cannot be taken back. The fix needs a new release, 0.1.1. That is the owner's decision, made through a gaps plan.
  - **Fresh install on this machine.** Once the check passes, the owner removes the local 0.1.1 install entirely and installs the release from nothing, as a new user would:
    1. Run `terminus uninstall --purge` and answer yes once. This deletes the archive too. Nothing is lost: `cleanupPeriodDays` is 3650, so every transcript the archive holds is still on disk, and the backfill rebuilds it.
    2. Run `rm -rf ~/.local/lib/crenshawdev/terminus/versions`. Uninstall always keeps that folder.
    3. Run `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`.
    - This is the only install against the real Claude Code. The four runners use a stand-in `claude`.
    - While the backfill runs, recall in open sessions returns partial results.
- **Verify:** human-verify (AC4, AC8). The owner checks:
  - `gh release view v0.1.0 --repo crenshawdev/terminus --json isDraft,isPrerelease,assets --jq '[.isDraft, .isPrerelease, (.assets | length)]'` prints `false`, `false`, `12`. The four new names are the `.manifest.sig` files.
  - `gh api repos/crenshawdev/terminus/releases/latest --jq .tag_name` prints `v0.1.0`.
  - The newest run of `release-check.yml`, shown by `gh run list --repo crenshawdev/terminus --workflow release-check.yml --limit 1`, succeeded. Each of its four jobs printed `0.1.0` from `~/.local/bin/terminus --version`.
  - After the fresh install on this machine, `~/.local/bin/terminus --version` prints `0.1.0`, and `readlink ~/.local/bin/terminus` ends in `versions/0.1.0/terminus`.
  - `claude mcp list` shows `terminus` as connected.
  - Once the backfill's run shows in `terminus status`, that run reports `0 failed`, and `terminus verify` reports no failure.

### Task 4: Say the release is published

- **Files:** install.sh (anchor: the header's first comment line), README.md (anchor: any sentence saying no release is published or that the first release is still to come), DESIGN-BRIEF.md (anchors: the paragraph starting "**There is no npm channel.**", the bullet "`origin` is the self-hosted repo and the only remote.", and the bullet starting "crates.io `terminus` is taken."), CHANGELOG.md (anchors: the intro's "its first release will be 0.1.0", `## [Unreleased]`, the `### Added` bullet ending "No release is published yet.", the `### Changed` heading, and the `[Unreleased]:` link reference), .planning/PROJECT.md (anchors: the paragraph ending "the first release will be 0.1.0", the `**Distribution**` bullet, and the bullet starting "crates.io name `terminus` is taken")
- **Action:**
  - **When (D-21).** Run this task only after Task 3's checks pass.
  - **`install.sh`.** The first comment line drops "No release is published yet." and reads `# Install Terminus.` Nothing else in `install.sh` changes.
  - **`README.md`.** Phase 9 wrote it before the release. If any sentence says no release is published or that the first release is still to come, that sentence changes to say 0.1.0 is published. Nothing else in the README changes.
  - **`DESIGN-BRIEF.md`.**
    - Drop "No release is published yet." from the no-npm paragraph.
    - The `origin` bullet becomes true. `origin` is github.com/crenshawdev/terminus, where releases are published, and `forgejo` is the self-hosted verbatim.git. D-21 cites this bullet as evidence, and Notes explains why it is in this task.
    - The crates.io bullet drops "how Terminus is delivered is still open" and says Terminus is delivered by `install.sh` from the GitHub releases.
  - **`CHANGELOG.md`.**
    - The Unreleased section becomes `## [0.1.0] - <date>`. The date is the day the release was published, from `gh release view v0.1.0 --repo crenshawdev/terminus --json publishedAt`.
    - Its content is unchanged, except that "No release is published yet." leaves its bullet.
    - An empty `## [Unreleased]` heading stays above it, as Keep a Changelog, the format the file declares, keeps one.
    - The link references: `[Unreleased]` compares `v0.1.0...HEAD`, and a new `[0.1.0]` compares `verbatim-v0.1.1...v0.1.0`, in the compare form the file already uses.
    - The intro's "its first release will be 0.1.0" changes tense, because the release is no longer to come.
    - The 0.1.0 section's `### Changed` gains one bullet: the Linux binaries are static musl builds with mimalloc as the allocator, and the macOS binaries link only the system libraries every Mac has. This is the first release where that is true.
  - **`.planning/PROJECT.md`.**
    - The `**Distribution**` bullet drops "no release is published yet". Its last clause becomes: `origin` is github.com/crenshawdev/terminus and `forgejo` is the self-hosted verbatim.git (D-21).
    - The paragraph ending "the first release will be 0.1.0" changes tense, for the same reason.
    - The crates.io bullet drops "how Terminus is delivered is still open" and says Terminus is delivered by `install.sh` from the GitHub releases, matching the brief.
  - **Nothing else.** Change no other line in these five files.
  - **How these edits reach `main`.** Task 1 has already landed, so this task's commit is not on `main` yet. It reaches `main` with the next `/cad-land`, the one that lands this phase after `/cad-verify 10`, ahead of phase 11's design docs. Until then, the public `install.sh` header still says no release is published. That is a comment line, and the install does not read it.
- **Verify:**
  - `grep -niE 'no release is published|first release will be|only remote|delivered is still open' README.md install.sh DESIGN-BRIEF.md CHANGELOG.md .planning/PROJECT.md` prints nothing.
  - `grep -n -i 'musl' CHANGELOG.md` finds the new bullet inside the `[0.1.0]` section.
  - `grep -n '^## \[' CHANGELOG.md` lists `[Unreleased]` first and `[0.1.0] - <date>` second. `grep -nE '^\[Unreleased\]: .*v0\.1\.0\.\.\.HEAD$|^\[0\.1\.0\]: .*verbatim-v0\.1\.1\.\.\.v0\.1\.0$' CHANGELOG.md` finds both lines.
  - `grep -n 'forgejo' .planning/PROJECT.md DESIGN-BRIEF.md` finds the corrected line in each.
  - The rest of AC7, set by phase 8's plan 1: `grep -n '^version = "0.1.0"' Cargo.toml` finds the line, and `jq '.git.create_tag' .planning/config.json` prints `false`.
  - `sh -n install.sh` and `dash -n install.sh` exit 0, and `cargo test -p terminus --test install_script` passes.
  - `git diff --stat` lists only files from this task's five.

## Notes

- **Why the `DESIGN-BRIEF.md` `origin` bullet is in Task 4.** D-21 names only `PROJECT.md`'s remote line, but its evidence cites `DESIGN-BRIEF.md:382,501`. Line 501, now 502, is the brief's copy of the same false claim. Task 4 corrects both. The owner accepted this on 2026-10-14.
- **Why "first release will be 0.1.0" is in Task 4.** AC7 says none of the files may say no release is published. The CHANGELOG intro and `PROJECT.md` both say the first release "will be" 0.1.0, which says exactly that. So both change tense. The owner accepted this on 2026-10-14.
- **Added at the owner's acceptance, 2026-10-14.** The CHANGELOG's musl and mimalloc bullet, and the "how Terminus is delivered is still open" lines in `PROJECT.md:97` and `DESIGN-BRIEF.md:503`, which are false once the release exists.
- **Why Task 3 ends with a fresh install.** The owner chose it on 2026-10-14: remove the local install entirely and install the release from nothing, purging the archive too, since a rescan of the transcripts rebuilds it. Installing over the local 0.1.1 build would test only an upgrade. A purge is safe only while Claude Code keeps every transcript; once its retention is lowered, the archive is the only copy of older sessions.
- **Recalled prior art.** The Phase 6 UAT asked that the CHANGELOG, `DESIGN-BRIEF.md` and the `install.sh` header keep saying no release is published until one is (`.planning/phases/6/UAT.md`, phase 6). Task 4 is the change that UAT item was waiting for.
