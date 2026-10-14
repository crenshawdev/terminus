---
phase: 6
status: complete
completed: 2026-10-14
---

# Phase 6: Signed Install - Summary

`install.sh` now verifies `terminus-<os>-<arch>.manifest.sig` with `ssh-keygen -Y verify` against the pinned crenshawdev release key under the namespace `terminus-release` before it reads the manifest, and refuses a release it cannot verify.

## What shipped

- The pinned key, held once as a literal in `release_key` - `install.sh:21`
- The signature download after the manifest, and the `ssh-keygen -Y verify` check before the binary is requested and before the manifest is read - `install.sh:190-202`
- The refusal of a machine without `ssh-keygen`, naming openssh-client, openssh-clients and openssh, before any download - `install.sh:176-177`
- A header comment describing the check, the pinned key and the `ssh-keygen` requirement - `install.sh:5-7`
- Tests for a missing signature, a signature from another key (with `$HOME/.ssh/allowed_signers` listing that key), the `baley-release` namespace, a manifest changed after signing, a missing `ssh-keygen`, and the key held once - `crates/terminus/tests/install_script.rs`
- The signed install in the CHANGELOG's Unreleased section - `CHANGELOG.md`
- A `### Release signature` subsection and the three-download order in §13 - `DESIGN-BRIEF.md`
- Lines 21 and 23 corrected to the text the owner approved at plan review - `.planning/PROJECT.md`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 3f20639 | Pin the key, download the signature, verify it before the binary download and the manifest read, rewrite the header; the test harness signs each served manifest |
| 1 | 2 | 3817a49 | Refuse a machine without `ssh-keygen` before any download |
| 1 | 3 | 25fbe9f | Prove a missing or bad signature is refused before the binary is requested |
| 1 | 4 | 504f952 | Prove the pinned key appears in `install.sh` exactly once, as a literal |
| 2 | 1 | 8ef12ab | Describe the signed install in the CHANGELOG |
| 2 | 2 | b7eab6b | Describe the signature check in DESIGN-BRIEF §13 as it stands |
| 2 | 3 | e89a517 | Correct PROJECT.md lines 21 and 23 with the approved text |

## Deviations

- [deviation] Plan 1, task 4: a one-character change to the pinned key was expected to fail only the new test and the success-path tests, and it failed 47, because most refusal tests install 1.0.0 first and fail in that setup. The 8 that passed are 4 tests with no install step, run under both shells. The task's Verify still held as written, so nothing was changed (504f952).

## Open items

None.

## Goal check

The commits deliver the goal. `install.sh:21` pins the key's public line as a literal, `install.sh:190-193` downloads the manifest and then its signature, and `install.sh:196-199` writes the allowed-signers file from that pinned key alone and runs `ssh-keygen -Y verify -I crenshawdev-release -n terminus-release` against it, failing on any mismatch, before the binary download at `install.sh:201` and the manifest read that follows it. The SHA-256 check is unchanged (`install.sh:80-91`), and an update runs the same script, so it gets the same check. Plan 1's report records 55 install-script tests passing, with each shell test run under both `/bin/sh` and dash and a refusal test for each bad-signature case, and plan 2's report records `cargo test --workspace --all-features` at 827 passed, 0 failed. `sh -n install.sh` and `dash -n install.sh` both exit 0 here. One limit is worth knowing: the tests that install sign with a throwaway key and run a copy of the script with that key swapped in, so the pinned crenshawdev key has not yet verified a real signature. That first happens when phase 7 publishes a signed release, which is that phase's goal.
