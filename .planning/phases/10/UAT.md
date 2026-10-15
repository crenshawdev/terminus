---
status: testing
phase: 10
fields_version: 1
started: 2026-10-15
updated: 2026-10-15
---

## Items

### 1. Pushing v0.1.0 leaves an eight-file draft with exact manifests
expected: Pushing the tag v0.1.0 on main leaves a draft release holding exactly the four binaries and a .manifest for each, and each manifest is exactly `version 0.1.0` and `sha256 <its binary's SHA-256>`, each line ending in a newline.
criterion: AC1
status: pass
first_pass: pass
source: model
evidence: Checked 2026-10-15 before the phase closed (phases/10/reports/plan-1.md, Task 2). Release run 38675944932, a push on v0.1.0 at 518c803, concluded success. `gh release view v0.1.0` printed isDraft true and exactly the eight files: terminus-{linux,macos}-{x86_64,aarch64} and a .manifest for each. Each manifest is 86 bytes, `version 0.1.0\n` then `sha256 <64 hex>\n`, and all four hashes matched `sha256sum` of their binaries.

### 2. Linux binaries are static and macOS binaries link only /usr/lib
expected: `file` reports both Linux binaries as statically linked, and `otool -L` on both macOS binaries lists only libraries under /usr/lib/.
criterion: AC2
status: pass
first_pass: pass
source: model
evidence: Checked 2026-10-15 (phases/10/reports/plan-1.md, Task 2; SUMMARY.md Goal check). Release run 38675944932's log: `file` printed `static-pie linked` for linux-x86_64 and `statically linked` for linux-aarch64; `otool -L` on both macOS binaries listed only `/usr/lib/libSystem.B.dylib`. The same holds for 0.1.1.

### 3. v0.1.1 is signed, published as latest and installs on all four runners
expected: After the signing script runs on the v0.1.1 draft, the release is published, not a prerelease, releases/latest points at it, it holds twelve files, and the check passes on all four runners with `~/.local/bin/terminus --version` printing 0.1.1.
criterion: AC4
status: pass
first_pass: pass
source: model
evidence: Checked 2026-10-15 (SUMMARY.md Goal check). `gh release view v0.1.1` shows it published, not a prerelease, with 12 files; releases/latest is v0.1.1; check run 38680343692 passed on all four runners. `gh release list` today prints `v0.1.1 Latest 2026-10-15T04:13:27Z`.

### 4. The docs say the release is published
expected: None of README.md, install.sh, DESIGN-BRIEF.md, CHANGELOG.md or .planning/PROJECT.md says no release is published; the CHANGELOG has 0.1.0 and 0.1.1 sections; PROJECT.md no longer calls origin self-hosted or the only remote; Cargo.toml and Cargo.lock say 0.1.1; git.create_tag is false.
criterion: AC7
status: pass
first_pass: pass
source: model
evidence: Checked 2026-10-15 (SUMMARY.md Goal check; commits 5a5b151, 7828e73, 2923e11). `grep -niE 'no release is published|first release will be|only remote|delivered is still open'` over the five files printed nothing; the CHANGELOG has [0.1.1] and [0.1.0] sections with compare links; Cargo.toml says 0.1.1; git.create_tag is false.

### 5. 0.1.1 installs from nothing on the owner's machine
expected: After `terminus uninstall --purge` and deleting the versions folder, the one-line install puts 0.1.1 in place: `terminus --version` prints 0.1.1, `claude mcp list` shows terminus connected, the backfill run shows 0 failed, and `terminus verify` reports no failure.
criterion: AC8
status: pass
first_pass: pass
source: model
evidence: Checked 2026-10-15 on the owner's machine (SUMMARY.md Goal check). ~/.local/bin/terminus links to versions/0.1.1/terminus and only 0.1.1 is in versions; `claude mcp list` shows terminus connected; the archive's first run is the backfill with 1999 files committed and 0 failed; `terminus verify` checked 2013 sessions with 0 failed.

## Summary

total: 5
passed: 5
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 0
