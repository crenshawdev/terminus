---
phase: 10
status: complete
completed: 2026-10-15
---

# Phase 10: Publish 0.1.0 - Summary

Terminus 0.1.0 and 0.1.1 are published on github.com/crenshawdev/terminus, each signed on the owner's machine. The 0.1.0 check failed on both aarch64 runners, so the fix shipped as 0.1.1, which is the latest release and installs on all four platforms.

## What shipped

- Phases 1 to 9 landed on `main` through PR #48.
- `v0.1.0`, published 2026-10-15 03:30 UTC, holds twelve files. Its check (run 38676626386) passed on `ubuntu-24.04` and `macos-15-intel` and failed on `ubuntu-24.04-arm` and `macos-15`, because the aarch64 binaries did not carry the install marker.
- The fix landed through PR #49: the marker is read through a volatile load so every build carries it, the release workflow fails a build whose binary does not carry the marker, and the version is 0.1.1.
- `v0.1.1`, published 2026-10-15 04:13 UTC, holds twelve files and is `releases/latest`. Its check (run 38680343692) passed on all four runners.
- PR #50 made the docs say the release is published. The CHANGELOG has a 0.1.0 and a 0.1.1 section.
- On the owner's machine, 0.1.1 was installed from nothing. The archive was created at 04:15:35 UTC, the same second as the `versions/0.1.1` folder, and only 0.1.1 is in `versions`.

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 3 | 50efd02 | Read the marker through a volatile load so every build carries it |
| 1 | 3 | 85a501f | Name this build's file, not the link, in the refusal's command |
| 1 | 3 | c40021a | Let install.sh replace a link into its own versions folder |
| 1 | 3 | 513fb63 | Fail a build whose binary does not carry the marker |
| 1 | 3 | 2923e11 | Bump the version to 0.1.1 |
| 1 | 3 | 1402bf1 | Move AC4, AC7 and AC8 to 0.1.1 |
| 1 | 4 | 5a5b151 | Say the release is published, and record 0.1.0 and 0.1.1 in the changelog |
| 1 | 4 | 7828e73 | Say in PROJECT.md how Terminus is delivered and where origin is |

Tasks 1 and 2 were the owner's steps and changed no file.

## Deviations

- [deviation] The 0.1.0 check failed on both aarch64 runners. A published release cannot be taken back, so the fix shipped as 0.1.1, and AC4, AC7 and AC8 in `CONTEXT.md` now name 0.1.1 (1402bf1). The ROADMAP's success criteria 3 and 4 still name 0.1.0.
- [deviation] The plan sends a failure through a gaps plan (`/cad-plan 10 --gaps`). The fix landed directly through PR #49, and phase 10 has no gaps plan.
- [deviation] Task 4 said the `origin` lines in `PROJECT.md` and `DESIGN-BRIEF.md` would name `forgejo` as the self-hosted verbatim.git. They say instead that `origin` is github.com/crenshawdev/terminus, where releases are published, and the brief says publishing elsewhere is handled outside this project.

## Open items

None.

## Goal check

The phase goal is met with 0.1.1. AC1 and AC2 were met by 0.1.0: the draft held eight files, and the build logs show `static-pie linked` for the Linux binaries and only `/usr/lib/libSystem.B.dylib` from `otool -L` for the macOS binaries, as they do for 0.1.1. AC4: `gh release view v0.1.1` shows it published, not a prerelease, with 12 files, `releases/latest` is `v0.1.1`, and run 38680343692 passed on all four runners. AC7: `grep -niE 'no release is published|first release will be|only remote|delivered is still open'` over the five files prints nothing, the CHANGELOG has `[0.1.1]` and `[0.1.0]` sections with their compare links, `Cargo.toml` says 0.1.1, and `git.create_tag` is false. AC8: `~/.local/bin/terminus` links to `versions/0.1.1/terminus`, `claude mcp list` shows `terminus` connected, the archive's first run is the backfill with 1999 files committed and 0 failed, and `terminus verify` checked 2013 sessions with 0 failed.
