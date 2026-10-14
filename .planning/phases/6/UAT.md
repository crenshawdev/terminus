---
status: testing
phase: 6
fields_version: 1
started: 2026-10-14
updated: 2026-10-14
---

## Items

### 1. Pinned key appears once; downloads run manifest, signature, binary
expected: install.sh contains the line `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q` exactly once, and a no-argument run requests the default source's terminus-<os>-<arch>.manifest, terminus-<os>-<arch>.manifest.sig and terminus-<os>-<arch>, in that order.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: install.sh:21 holds the line once (grep -c -F prints 1), and it matches ~/.ssh/crenshawdev-release.pub. install_sh_pins_the_release_key_once passes. no_source_downloads_from_the_github_releases and a_piped_script_with_no_source_downloads_from_the_github_releases, ::sh and ::dash, assert the curl log is exactly the manifest, manifest.sig and binary URLs, and they pass.

### 2. Bad or missing signature is refused before the binary is requested
expected: For a missing signature, a signature from another key, a signature under the namespace baley-release, and a manifest changed after signing, the run exits non-zero with a reason on stderr, never requests the binary, and leaves the stable path and the versions folder unchanged. A missing signature reports `signature download failed: <url>`.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: install.sh:192-199 runs before the binary curl at :201 and the manifest read at :204. a_missing_signature_is_refused, a_release_signed_by_another_key_is_refused, a_signature_under_another_namespace_is_refused and a_manifest_changed_after_signing_is_refused pass as ::sh and ::dash. Each asserts non-zero exit, the curl log stops at the .sig, the stable path and versions tree are unchanged, no claude call, empty stdout, and only 'terminus: ' lines on stderr. The missing case reports 'signature download failed: <url>'.

### 3. No ssh-keygen is refused before any download
expected: With no ssh-keygen on PATH, the run exits non-zero before any download, the message names openssh-client, openssh-clients and openssh, and nothing under HOME changes.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: install.sh:176-177 runs before mktemp (:179) and any curl (:190). no_ssh_keygen_is_refused_before_any_download::{sh,dash} asserts an empty curl log, an empty HOME and the three package names as whole words, and passes.

### 4. A correctly signed release installs under /bin/sh and dash
expected: A correctly signed release installs as it did in Phase 5 under both /bin/sh and dash, with no ssh-keygen output in what the run prints. `sh -n install.sh` and `dash -n install.sh` pass, and `cargo test --workspace --all-features` passes.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: sh -n and dash -n exit 0. cargo test --workspace --all-features exits 0 with 827 passed and 0 failed; install_script has 55 passed. one_run_installs_and_wires_claude_code::{sh,dash} installs a signed release and asserts no ssh-keygen 'Good' line in the output.

### 5. Unmodified install.sh ignores the user's allowed_signers
expected: The unmodified install.sh refuses a release signed by a throwaway key, even when that key is listed in $HOME/.ssh/allowed_signers.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: a_release_signed_by_another_key_is_refused::{sh,dash} runs the shipped install.sh with $HOME/.ssh/allowed_signers listing the throwaway key and is refused with the bad-signature line. A probe shows that same allowed_signers line verifies the signature when passed as -f (exit 0), so the test would catch a script that read it.

### 6. Docs describe the signed install
expected: The CHANGELOG.md Unreleased section, DESIGN-BRIEF.md §13 and the install.sh header describe the signature check, the pinned key and the need for ssh-keygen; none still says "inside verify_download" or "unsigned development artifacts only"; they still say no release is published yet. PROJECT.md no longer says a release waits for the update feature, or that updates come with Baley's #14 signing.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: CHANGELOG.md:20-44, DESIGN-BRIEF.md:381,395,404-412 and install.sh:2,5-7 describe the check, the pinned key and ssh-keygen, and each still says no release is published yet. The stale phrases grep to 0. PROJECT.md:21,23 hold the approved text, and the stale phrases grep to nothing.

### 7. DESIGN-BRIEF §13 install.sh bullet mixes verb forms
expected: behavior wrong - a grammar slip introduced in this phase's rewrite of the bullet
origin: verifier
status: pass
first_pass: fail
source: model
evidence: After 05942ce: grep -c 'and moves a temporary link' DESIGN-BRIEF.md -> 0; grep -c 'and move a temporary link over the stable path' DESIGN-BRIEF.md -> 1. DESIGN-BRIEF.md:395 now reads 'Only then does it download ..., check ..., stage it ..., and move a temporary link over the stable path with `mv -f`.'
reported: behavior wrong - a grammar slip introduced in this phase's rewrite of the bullet
severity: cosmetic
cause: Commit b7eab6b rewrote the bullet from 'downloads ..., checks ..., stages ..., and moves' into 'Only then does it download ..., check ..., stage ...', and changed three of the four verbs to the base form. The fourth, 'moves', kept its old form.
fix: 05942ce, retest

## Summary

total: 7
passed: 7
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 1
