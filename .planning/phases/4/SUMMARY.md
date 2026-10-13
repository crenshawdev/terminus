---
phase: 4
status: complete
completed: 2026-10-13
---

# Phase 4: One-Command Installer - Summary

`sh install.sh https://<source>` downloads the manifest and binary for the machine, checks the SHA-256 in `verify_download`, stores the binary at `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, moves the link at `~/.local/bin/terminus` onto it and runs `terminus install`; `terminus install` itself now stores the running build as a version behind the same link, so a local build is `cargo build --release && target/release/terminus install`.

## What shipped

- `terminus install` stores the running build as a version and points the stable path at it with a symbolic link, replacing the copy over the stable path - `crates/terminus/src/cmd/install/binary.rs:197` (`place`), `:239` (`store`), `:275` (`link`), `:102` (`versions_dir`)
- Install run from the link's own target leaves the link and the version file alone, also when the link points into a version folder named by a manifest - `crates/terminus/src/cmd/install/binary.rs:218` (`linked_to`)
- A rebuild at the same version is renamed over its file, and the same bytes are left alone (D-03)
- Uninstall removes the link, keeps every version and prints the versions folder's path - `crates/terminus/src/cmd/uninstall.rs:77,91`
- `install.sh` in Baley's shape: https-only source, Linux and Darwin on x86_64 and aarch64, an exact two-line manifest, `verify_download` before anything is staged, ownership of the stable path by the marker, refusal when `TERMINUS_BIN_DIR` is set, and `"$stable" install` as its last line - `install.sh`
- A unit test that fails when `install.sh`'s copy of the marker differs from `binary::MARKER` - `crates/terminus/src/cmd/install/binary.rs:402`
- An end-to-end harness that runs `install.sh` against `openssl s_server -WWW` with a throwaway certificate: a full install, no `claude`, the `shasum` fallback, seven refusals that change nothing, and successive installs that keep every version - `crates/terminus/tests/install_script.rs` (14 tests)
- DESIGN-BRIEF.md §10, §12 and §13, PROJECT.md's Privacy and Distribution lines, and CHANGELOG.md `[Unreleased]` describe the link, the versions folder and `install.sh`'s download as the one network use

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | e09ce89 | Store this build as a version and link the stable path to it |
| 1 | 2 | 8960444 | Leave the link and its file alone when install runs from them |
| 1 | 3 | db883c4 | Remove the link on uninstall and keep the versions folder |
| 2 | 1 | 7a48983 | Add `install.sh` in Baley's shape, owning the stable path by the marker |
| 2 | 2 | 29e5b4e | Run `install.sh` end to end against a local HTTPS server |
| 2 | 3 | ab5e816 | Prove every `install.sh` refusal leaves the machine untouched |
| 2 | 4 | b786f5a | Prove successive installs keep every version and its inode |
| 3 | 1 | 6c13f2f | Describe the stable path as a link into the versions folder |
| 3 | 2 | 9ee9524 | Say the binary opens no network connection and `install.sh`'s download is the one network use |
| 3 | 3 | d3c95f9 | Record `install.sh` and the link into the versions folder in the changelog |

## Deviations

- [deviation] Plan 2 task 3: with `verify_download` commented out, the executor expected only the digest refusal test to fail; `shasum_computes_the_digest_when_sha256sum_is_missing` failed too, because it also asserts that a wrong digest is refused. A prediction miss; nothing changed. ab5e816

## Open items

- `openssl s_server -WWW` answers a missing file with `200 ok` and an error text, so `curl -f` cannot fail on a missing file in the harness. Every test serves both files, the module doc says so, and `install.sh`'s "download failed" refusals (`install.sh:162-165`) are exercised by no test.
- PROJECT.md's Key Decisions table still carries rows from before phase 1 and this phase: "Observations opt-in; parser does mechanical, LLM only judgment", "Redact at egress, never at ingest; ... local providers send nothing over the wire", "Shared credentials at ~/.config/jcrenshaw/credentials.toml" and "Canonical stable binary path". Plan 3 corrected only the Privacy and Distribution lines it was scoped to.

## Goal check

The ten commits plausibly deliver the goal. `install.sh` maps `uname -s` and `uname -m` to the machine's folder and downloads its manifest and binary over https only (`install.sh:140-165`), checks the SHA-256 in `verify_download` (`install.sh:71-75`) before anything is staged (`:191`, staging at `:195-210`), stores the binary under `versions/<version>/terminus` and moves a temporary link over `~/.local/bin/terminus` (`:135-137`, `:212-219`), and ends with `"$stable" install` (`:232`). Its header says it installs unsigned development artifacts only and that the signature check goes inside `verify_download` later (`install.sh:2-4`). It is qualified against a local HTTPS server with unsigned artifacts (29e5b4e, ab5e816, b786f5a), and the full suite passed at 784 tests after plan 2 (plan 2 report, task 4) and again after plan 3. The local-build question CONTEXT settled is delivered as D-02 said: `place` stores the running build as a version (e09ce89). Two things are not proven. The download-failure refusals are untested because of the harness quirk above. Everything was measured on Linux only; CONTEXT's assumption that `current_exe()` through a link returns the target on macOS (`.planning/phases/4/CONTEXT.md:45`) was not checked. Plan 2's `risk_surface` review raised five findings: three were refuted against D-07, D-08 and D-04 and the code, two were confirmed at low, and the user declined all five (`.planning/phases/4/ADJUDICATION-risk_surface-plan-2.json`, `.planning/DECLINED.md`).
