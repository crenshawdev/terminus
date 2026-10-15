---
status: testing
phase: 8
fields_version: 1
started: 2026-10-15
updated: 2026-10-15
---

## Items

### 1. Signing script refuses a bad draft
expected: `cargo test -p terminus --test sign_release` passes, and its tests show the script exits non-zero, uploads nothing and leaves the release a draft when a binary does not match its manifest, when the draft is missing any of the eight files, and when a signature does not verify against the key pinned in install.sh, with a stand-in gh and a throwaway key.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: `cargo test --offline --locked -p terminus --test sign_release`: 15 passed. a_draft_missing_a_file_is_refused, a_binary_that_does_not_match_its_manifest_is_refused and a_signature_the_pinned_key_does_not_verify_is_refused pass under sh and dash, each asserting non-zero exit, no upload, no edit and no -Y sign before the refusal (sign_release.rs:452-539); scripts/sign-release.sh:74-104 runs every check before the upload at :111.

### 2. Workflows hold no secret and no signing step
expected: No file under .github/workflows/ contains `secrets.`, `secrets[`, `-Y sign` or `PRIVATE KEY`; `cargo test -p terminus --test release_workflows` passes and fails when one is added; `gh api repos/crenshawdev/terminus/actions/secrets` reports 0.
criterion: AC5
status: pass
first_pass: pass
source: model
evidence: `grep -n -E "secrets\.|secrets\[|-Y sign|PRIVATE KEY" .github/workflows/*` prints nothing (exit 1); `gh api repos/crenshawdev/terminus/actions/secrets --jq .total_count` prints 0; `cargo test --locked -p terminus --test release_workflows`: 1 passed; with a probe file .github/workflows/zz-probe.yml holding `secrets.X` it fails naming zz-probe.yml:2, and passes again once the probe is removed.

### 3. musl build keeps the hooks under 10 ms
expected: Against the owner's real corpus store, the musl release build stays under the 10 ms p99 wall for every hook event in the corpus test. (human-verify: needs the musl toolchain, absent here; `sudo pacman -S musl`)
criterion: AC6
status: pass
first_pass: pass
reported: i take yours
reason: The owner accepted the coordinator's run: corpus test on the musl release build, 2 passed, slowest p99 SessionStart 5.39 ms against the 10 ms wall.

### 4. Signing script refuses a release that is not a draft
expected: Run against a published release, the script fails before downloading or signing anything: no upload and no edit call (the `a_published_release_is_refused` test in sign_release.rs).
status: pass
first_pass: pass
source: verifier
evidence: a_published_release_is_refused::sh and ::dash passed: the gh log is the isDraft view alone, with no download, ssh-keygen, upload or edit; scripts/sign-release.sh:62-65 runs before anything is downloaded.

### 5. Linux musl binary is static and uses mimalloc
expected: `cargo build --release --locked -p terminus --target x86_64-unknown-linux-musl` builds; `file` calls the binary statically linked; `nm` shows mi_free and mi_malloc_aligned; `--version` prints 0.1.0.
status: pass
first_pass: pass
source: model
evidence: At HEAD d09341c: `cargo build --release --locked -p terminus --target x86_64-unknown-linux-musl` finished; `file` prints "ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked"; `nm` prints `T mi_free` and `T mi_malloc_aligned`; `--version` prints 0.1.0.

### 6. Release workflow builds four platforms into one draft
expected: release.yml runs only on a pushed v<X.Y.Z> tag, builds the four terminus-<os>-<arch> targets natively with fail-fast off, checks --version against the tag, file/otool linkage, and only one job, after all four builds, holds contents: write and makes the draft with github.token; every uses: is pinned by a full 40-character SHA.
status: pass
first_pass: pass
source: verifier
evidence: release.yml:5-8 is the only trigger, :20-35 has four native runner/target pairs with fail-fast: false, :76-84 checks the version (run locally: exit 0 for v0.1.0, exit 1 for v0.1.1), :89-110 checks file/otool linkage, :138-141 is the only contents: write job and needs the build, :170-179 makes the draft with github.token, and all 3 uses: are pinned by 40-hex SHAs. The eight-file check and manifest step were run locally and behave.

### 7. Check workflow installs a published release on four runners
expected: release-check.yml runs only on release: published with contents: read, puts a stand-in claude on PATH, runs the one curl line from main under bash, and requires ~/.local/bin/terminus --version to print the release version on each of the four runners.
status: pass
first_pass: pass
source: verifier
evidence: release-check.yml:7-12 has the release: published trigger and contents: read, :20 the four runners, :26 bash, :30-37 the stand-in claude, :40 the public curl line from main, and :42-52 the version comparison through $HOME/.local/bin/terminus. Install's registration only needs claude mcp to exit 0 (install/mod.rs:282-317).

### 8. Version 0.1.0 and no bare tag on land
expected: Cargo.toml and Cargo.lock say 0.1.0 for terminus and terminus-core, and `jq .git.create_tag .planning/config.json` prints false.
status: pass
first_pass: pass
source: verifier
evidence: Cargo.toml:6 0.1.0; Cargo.lock:442 and :454 0.1.0 for terminus and terminus-core; `jq .git.create_tag .planning/config.json` prints false, and HEAD's copy says the same.

### 9. Tag makes a draft of eight files (moved)
expected: Moved to phase 10 with the live release on 2026-10-14; verified there.
criterion: AC1
status: skipped
source: model
reason: Moved to phase 10 with the live release by the owner on 2026-10-14 (ea25a57); verified in phase 10.

### 10. Release binaries' linkage on CI (moved)
expected: Moved to phase 10 with the live release on 2026-10-14; verified there.
criterion: AC2
status: skipped
source: model
reason: Moved to phase 10 with the live release by the owner on 2026-10-14 (ea25a57); verified in phase 10.

### 11. Published release installs on four runners (moved)
expected: Moved to phase 10 with the live release on 2026-10-14; verified there.
criterion: AC4
status: skipped
source: model
reason: Moved to phase 10 with the live release by the owner on 2026-10-14 (ea25a57); verified in phase 10.

### 12. Docs say the release is published (moved)
expected: Moved to phase 10 with the live release on 2026-10-14; verified there.
criterion: AC7
status: skipped
source: model
reason: Moved to phase 10 with the live release by the owner on 2026-10-14 (ea25a57); verified in phase 10.

### 13. Item 2, last clause: run `gh api repos/crenshawdev/terminus/actions/secrets --jq .total_count`
expected: It prints 0. The rest of item 2 is verified: no forbidden string under .github/workflows, and release_workflows passes.
origin: verifier
why_human: Not a limit of the model: it needs a GitHub API call, and this verification may not use the network. The owner's gh is logged in on this machine, so the walk can run it as is.
status: pass
first_pass: pass
source: model
evidence: `gh api repos/crenshawdev/terminus/actions/secrets --jq .total_count` prints 0.

### 14. Item 5: run `cargo build --release --locked -p terminus --target x86_64-unknown-linux-musl`, then `file`, `nm | grep -E ' (mi_free|mi_malloc_aligned)$'` and `--version` on target/x86_64-unknown-linux-musl/release/terminus
expected: The build finishes, file says static-pie linked or statically linked, nm lists both symbols, and --version prints 0.1.0.
origin: verifier
why_human: Not a limit of the model: no probe here ran it. The musl binary on disk predates the 0.1.0 version commit and prints 0.1.1, and a fresh thin-LTO, codegen-units=1 release build takes minutes, far past this verification's 10-second limit per check. The musl target and /usr/bin/musl-gcc are both installed now, so the walk can run it.
status: pass
first_pass: pass
source: model
evidence: Same run as item 5: build finished, static-pie linked, mi_free and mi_malloc_aligned present, `--version` prints 0.1.0.

### 15. Item 3 (AC6): run `TERMINUS_TEST_CORPUS="$HOME/.claude" cargo test --release --locked -p terminus --target x86_64-unknown-linux-musl --test corpus -- --nocapture`
expected: The corpus test passes, and every hook event's p99 is under the 10.0 ms wall.
origin: verifier
why_human: Not a limit of the model: no probe here ran it. It needs a musl release build of the corpus test, which takes minutes, past this verification's 10-second limit, and it reads the owner's real ~/.claude corpus. The test writes only to temp folders. The item's note that musl is absent here is out of date: the musl target and musl-gcc are installed, so the walk can run it.
status: pass
first_pass: pass
source: model
evidence: `TERMINUS_TEST_CORPUS="$HOME/.claude" cargo test --release --locked -p terminus --target x86_64-unknown-linux-musl --test corpus -- --nocapture`: ok, 2 passed. p50/p99 over 100 runs, wall 10.0 ms: SessionStart 3.81/5.39 ms, UserPromptSubmit 0.36/0.69 ms, SessionEnd 0.31/0.63 ms, PostCompact 0.32/0.84 ms.

## Summary

total: 15
passed: 11
failed: 0
pending: 0
skipped: 4
blocked: 0
reworked: 0
