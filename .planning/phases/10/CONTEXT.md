# Phase 10: Publish 0.1.0 - Context

Gathered: 2026-10-14 (moved from phase 8)
Feeds: /cad-execute 10

## Scope boundary

In: The live release that was phase 8 plan 4, now `.planning/phases/10/PLAN.md`. Phases 1 to 9 land on `main`, `v0.1.0` is tagged on `main`, the release workflow makes the draft, the owner signs and publishes it with `scripts/sign-release.sh`, the check workflow installs it on all four runners, the owner removes his local install and installs the release from nothing, and the docs stop saying no release is published. Any failure the build or the check finds is fixed here through a gaps plan.
Out: Writing the README and user docs (phase 9, before this). The how-it-works design docs (phase 11, after this). Changing the release machinery beyond a fix the live run forces.

Plan shape: one plan, moved whole from phase 8 plan 4.

## Decisions

The decisions that bind this phase are in `.planning/phases/8/CONTEXT.md`: D-02, D-03, D-10, D-13, D-16, D-17, D-19 and D-21. One changed when the release moved here: D-02 now has the README (phase 9) land on `main` before the tag, not after the release.

## Acceptance criteria

AC1, AC2, AC4 and AC7 keep the numbers they had in phase 8, so the plan and the phase 8 record still name them alike. AC7 now names the README as well. AC8 is new.

AC4, AC7 and AC8 moved from 0.1.0 to 0.1.1 on 2026-10-15. 0.1.0 was published, and the check failed on both aarch64 runners because the aarch64 binaries do not carry the install marker. A published release cannot be taken back, so the fix ships as 0.1.1. AC1 and AC2 were met by 0.1.0 and stay as written.

- [ ] AC1: Pushing the tag `v0.1.0` on `main` leaves a draft release holding exactly eight files: `terminus-linux-x86_64`, `terminus-linux-aarch64`, `terminus-macos-x86_64`, `terminus-macos-aarch64` and a `.manifest` for each. Each manifest is exactly `version 0.1.0` and `sha256 <its binary's SHA-256>`, each line ending in a newline. (human-verify: needs GitHub Actions and the pushed tag)
- [ ] AC2: `file` reports both Linux binaries as statically linked, and `otool -L` on both macOS binaries lists only libraries under `/usr/lib/`. (human-verify: needs the macOS runners for `otool`)
- [ ] AC4: After the owner runs the signing script on the real `v0.1.1` draft, the release is published, not a prerelease, `releases/latest` points at it, and it holds twelve files: the eight plus four `.manifest.sig`. The check then passes on all four runners: each runs the one-line command from `main`, and `~/.local/bin/terminus --version` prints `0.1.1`. (human-verify: needs the owner's key passphrase and GitHub Actions)
- [ ] AC7: After publishing, none of `README.md`, `install.sh`, `DESIGN-BRIEF.md`, `CHANGELOG.md` or `.planning/PROJECT.md` says no release is published. The CHANGELOG has a 0.1.0 section and a 0.1.1 section, PROJECT.md no longer calls `origin` self-hosted or the only remote, `Cargo.toml` and `Cargo.lock` say 0.1.1, and `git.create_tag` is false in `.planning/config.json`.
- [ ] AC8: On the owner's machine, after `terminus uninstall --purge` and deleting `~/.local/lib/crenshawdev/terminus/versions`, the one-line install installs 0.1.1: `~/.local/bin/terminus --version` prints `0.1.1`, `claude mcp list` shows `terminus` connected, and once the backfill has run, `terminus status` shows that run with 0 failed and `terminus verify` reports no failure. (human-verify: needs the published release and the owner's machine)
