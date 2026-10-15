---
phase: 8
status: complete
completed: 2026-10-15
---

# Phase 8: Release Builds - Summary

The release machinery: static musl Linux builds with mimalloc, a tag-triggered GitHub Actions workflow that builds four platforms into one draft, a check workflow that installs a published release on four runners, and `scripts/sign-release.sh`, which signs and publishes a draft on the owner's machine. Nothing has run on GitHub yet; the live release is phase 10.

## What shipped

- mimalloc as the global allocator on musl targets only - `Cargo.toml` (`[workspace.dependencies]`), `crates/terminus/Cargo.toml`, `crates/terminus/src/main.rs`
- Workspace version 0.1.0 - `Cargo.toml`, `Cargo.lock`
- `git.create_tag` off, so `/cad-land` cuts no bare tag - `.planning/config.json`
- Release workflow: four native builds on a `v<X.Y.Z>` tag, then one draft of eight files - `.github/workflows/release.yml`
- Check workflow: installs a published release on the four runners with the one-line command - `.github/workflows/release-check.yml`
- A test that fails when a workflow reads a secret or signs anything - `crates/terminus/tests/release_workflows.rs`
- The signing script: refuses a release that is not a draft, rechecks every binary against its manifest, signs and verifies each manifest against the key pinned in `install.sh`, then uploads and publishes - `scripts/sign-release.sh`, tested by `crates/terminus/tests/sign_release.rs`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 3b321c3 | use mimalloc as the global allocator on musl builds |
| 1 | 2 | 3131497 | set the workspace version to 0.1.0 for the first release |
| 1 | 3 | 23251ac | stop cadence cutting a bare tag when it lands |
| 2 | 1 | 4e74878 | build the four platforms when a version tag is pushed |
| 2 | 2 | dadb8a2 | make one draft after all four builds pass |
| 2 | 3 | 2117553 | install each published release on the four runners |
| 2 | 4 | 638739d | fail when a workflow reads a secret or signs anything |
| 3 | 1 | 6c94e66 | sign each manifest, verify it as install.sh does, then publish |
| 3 | 2 | 8f7234a | prove a missing file or a mismatched binary is refused before signing |
| 3 | 3 | add6bc6 | prove a signature the pinned key does not verify is never uploaded |
| 3 | risk fix | 8af6f7c | refuse to sign or publish a release that is no longer a draft |

The suite (`cargo test --workspace --all-features`) passed after each plan: 842, 843, 856, then 858 after the fix, 0 failed.

Plan 4, the live release, moved to phase 10 on 2026-10-14 (ea25a57) so the README lands first. Its acceptance criteria AC1, AC2, AC4 and AC7 moved with it.

## Deviations

- [deviation] Plan 3: on a graphical desktop, `ssh-keygen -Y sign` reading stdin tries ssh-askpass instead of the terminal and exits 255 without prompting. The script sets `SSH_ASKPASS_REQUIRE=never` on the sign call, and the success test asserts it (6c94e66). D-13 in CONTEXT.md is annotated with the correction.
- [deviation] Plan 3: the verify-always-succeeds mutation failed the pinned-key test at its exit-status check before the upload log; the test was reordered so the upload-log check, which the Verify names, fails first (add6bc6).

## Open items

- Neither workflow has run on GitHub, and no macOS binary has been built. Both are first exercised by the live release in phase 10.
- The signing script's draft check has a narrow race: a release published from elsewhere while the script is signing would still be uploaded to and marked latest. Only an account with write access can publish, and GitHub offers no upload-only-if-draft call. Rated low in the plan 3 risk review's second round.
- The plan 2 risk review rated low that the workflow test matches fixed strings, so `toJSON(secrets)` or a non-ssh signing tool would not trip it. The repository holds no Actions secrets, so there is nothing to read.
- Plan 3 noted two behaviours the plan did not name: the script refuses a manifest that is not exactly the two lines `install.sh` accepts, and it signs and verifies one manifest at a time, so a wrong key fails after one passphrase prompt instead of four. Every verify still runs before any upload.

## Goal check

The commits deliver the phase goal as it now stands, the machinery without the live run. The Linux build is static and uses mimalloc: the coordinator's musl build printed `static-pie linked` and both `mi_free` and `mi_malloc_aligned` in `nm`, and the corpus test passed with every hook event under 10 ms p99, the slowest SessionStart at 4.69 ms (reports/plan-1.md). The release workflow's build and draft jobs are 4e74878 and dadb8a2, the check workflow is 2117553, and 638739d's test passes with no secret or signing string in either file and `gh api .../actions/secrets` at 0 (reports/plan-2.md). The signing script publishes only after every binary matches its manifest and every signature verifies against the pinned key, and refuses a non-draft, with 15 `sign_release` tests passing under sh and dash (reports/plan-3.md). What is not shown, and cannot be until phase 10, is any of it running on GitHub: the workflows were checked by reading, grep and a local dry run of the eight-file check, the macOS builds and `otool -L` have never run, and the signing script has only met a stand-in `gh`.
