---
phase: 9
plan: 1
requirements: [DOC-01]
files:
  - LICENSE
  - Cargo.toml
  - DESIGN-BRIEF.md
  - .planning/PROJECT.md
  - CHANGELOG.md
  - TRUST.md
  - SECURITY.md
  - .planning/phases/9/BRIEF.md
---

# Phase 9: README and User Docs - Plan 1

## Goal

The phase goal: the public repository gets a README for someone who has never heard of Terminus, opening on the reader's problem and leading with the security model and the signed release, with user docs beside it for installing, every command, every setting and how a release is built, signed and checked.

This plan puts in place the files the README links to and names that are not docs pages. The license becomes MIT everywhere it is stated, `TRUST.md` is the shared text, and `SECURITY.md` gives security reports a private route.

## Must be true when done

- **The license is MIT (AC6).** `LICENSE` is byte-identical to `/code/baley/LICENSE`, `cargo metadata` reports `MIT` for both crates, and no line of `Cargo.toml`, `DESIGN-BRIEF.md` or `.planning/PROJECT.md` names Apache-2.0.
- **The CHANGELOG records the switch.** The `### Changed` list under `## [Unreleased]` has one entry saying the license is now MIT and was Apache-2.0.
- **`TRUST.md` is the shared copy (AC6).** It is byte-identical to the fenced block in `.planning/phases/9/BRIEF.md` and to `/code/baley/TRUST.md`.
- **`SECURITY.md` exists in Baley's form (AC6, human-verify).** It sends reports to the repository's Security tab, its scope is Terminus, and it says nothing about whether a release is published. `gh api repos/crenshawdev/terminus/private-vulnerability-reporting` returns `"enabled":true` once the owner has turned it on.
- **Each file has had its review stop (D-16).** The owner has reviewed `LICENSE`, `Cargo.toml`, `TRUST.md` and `SECURITY.md` in RustRover.

## Context

- Plan 1 of 4. It shares no file with plan 2, and the two can run in either order. Plans 3 and 4 write the README, which links `LICENSE`, `TRUST.md` and `SECURITY.md`, so they run after this plan.
- D-01, D-02, D-13, D-14 and D-16 in `.planning/phases/9/CONTEXT.md` bind this plan. D-01 overrides the brief's "Do not change LICENSE", because the owner moved the switch into this phase on 2026-10-14.
- CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing any prose John puts his name on, and that includes `SECURITY.md`, the CHANGELOG entry, the corrected lines in `DESIGN-BRIEF.md` and `PROJECT.md`, and every commit message.
- Every file a task writes is opened for the owner in RustRover with `mcp__rustrover__open_file_in_editor` (memory `open-docs-in-rustrover`). The commits are signed and stay on `cadence/v0.0.1`, and no PR is opened (D-15).
- Out of scope: the README, the docs pages and anything under `docs/`. `TRUST.md` is never adapted to Terminus.

## Tasks

### Task 1: Switch the license to MIT

- **Files:** LICENSE, Cargo.toml (anchor: the license key under workspace.package), DESIGN-BRIEF.md (anchor: the section 16 bullet starting Apache-2.0), .planning/PROJECT.md (anchors: the Licensing bullet under Constraints and the Key Decisions row starting Apache-2.0), CHANGELOG.md (anchor: the Changed list under Unreleased)
- **Action:**
  - **`LICENSE` (D-01).** Replace the Apache-2.0 text with the MIT text in Baley's form, reading "Copyright (c) 2026 John Crenshaw". Copy the bytes of `/code/baley/LICENSE` rather than retyping them, because the check is byte identity.
  - **`Cargo.toml` (D-01).** `license = "Apache-2.0"` under `[workspace.package]` becomes `license = "MIT"`. Both crates inherit it through `license.workspace = true`, so neither crate manifest changes. No dependency changes, so `Cargo.lock` does not change either.
  - **`DESIGN-BRIEF.md` (D-13).** The §16 bullet's lead becomes **MIT.** and its sentence "Apache over MIT for the explicit patent grant." goes, because MIT carries no patent grant. The rest of the bullet stays: source must be public, AGPL is a negative signal, and the dependency licenses. "rayon MIT/Apache" stays, because it names rayon's license and not Terminus's.
  - **`.planning/PROJECT.md` (D-13).** The `**Licensing**` constraint says MIT where it says Apache-2.0 and keeps the rest of its line. The Key Decisions row says MIT where it says Apache-2.0, and its rationale drops "explicit patent grant" and keeps its other two reasons.
  - **`CHANGELOG.md` (D-13).** Add one bullet to the `### Changed` list under `## [Unreleased]`: the license is now MIT, it was Apache-2.0, `LICENSE` holds the MIT text, and the workspace `Cargo.toml` says `license = "MIT"`. Match the tense and form of the bullets already there.
  - **No new reasons.** No source gives a reason for MIT over Apache-2.0 beyond the owner's move of 2026-10-14. Write none into `DESIGN-BRIEF.md`, `PROJECT.md` or the CHANGELOG, because a reason nobody stated is an invented motive.
  - **Review stop (D-16).** Open `LICENSE` and `Cargo.toml` in RustRover for the owner.
- **Verify:**
  - `cmp LICENSE /code/baley/LICENSE` exits 0.
  - `grep -n '^license' Cargo.toml` prints only `license = "MIT"`. `cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | "\(.name) \(.license)"'` prints `terminus MIT` and `terminus-core MIT`, and `--locked` passing shows `Cargo.lock` needs no change.
  - `grep -n 'Apache-2.0' Cargo.toml DESIGN-BRIEF.md .planning/PROJECT.md LICENSE` prints nothing, and `grep -n 'patent' DESIGN-BRIEF.md .planning/PROJECT.md` prints nothing.
  - `sed -n '/^## \[Unreleased\]/,/^## \[Verbatim/p' CHANGELOG.md | sed -n '/^### Changed/,/^### /p' | grep -n 'MIT'` finds the new bullet, and it names Apache-2.0 as the old license.
  - `git diff --stat` lists only `LICENSE`, `Cargo.toml`, `DESIGN-BRIEF.md`, `.planning/PROJECT.md` and `CHANGELOG.md`.
  - human-verify: the owner reviews `LICENSE` and `Cargo.toml` in RustRover and accepts both.

### Task 2: Add `TRUST.md` as the shared text

- **Files:** TRUST.md
- **Action:**
  - **Byte for byte (D-02).** `TRUST.md` at the root is the content of the fenced `markdown` block in `.planning/phases/9/BRIEF.md`, without the fence lines, and nothing else. Extract it from the brief mechanically rather than retyping it. It is shared across every crenshawdev repository, so it is never adapted to Terminus: no Terminus name, no added section, no edited sentence.
  - **Review stop (D-16).** Open `TRUST.md` in RustRover for the owner.
- **Verify:**
  - `cmp TRUST.md <(sed -n '/^```markdown$/,/^```$/p' .planning/phases/9/BRIEF.md | sed '1d;$d')` exits 0.
  - `cmp TRUST.md /code/baley/TRUST.md` exits 0.
  - human-verify: the owner reviews `TRUST.md` in RustRover and accepts it.

### Task 3: Add `SECURITY.md` for private reports

- **Files:** SECURITY.md
- **Action:**
  - **Baley's form, scoped to Terminus (D-14).** Follow `/code/baley/SECURITY.md`: a `# Security policy` title, a `## Reporting a vulnerability` section with Baley's text, which sends reports to the repository's **Security** tab and **Report a vulnerability** and asks for no public issue, and a `## Scope` section about Terminus.
  - **The scope says nothing about releases.** Baley's scope opens by saying it has no released version. Terminus's must not say whether a release is published, because phase 10's Task 4 corrects only the README, `install.sh`, `DESIGN-BRIEF.md`, the CHANGELOG and `PROJECT.md`, and would leave a sentence here false. Say that reports about the `main` branch and any published release are in scope, which is true before phase 10 and after it.
  - **Of most interest.** List only properties the Terminus docs claim, so a report against one of them is a report of a broken promise. The list draws from these: `install.sh` installing a release not signed by the pinned key, or downloading the binary before the signature verifies; the Terminus binary opening a network connection; Terminus reading an excluded project; archived text reaching Claude Code through `[privacy] redact_recall` with a secret it should have replaced; and tampering with the archive that `terminus verify` does not detect. Check each against `install.sh`, `crates/terminus-core/tests/no_network.rs`, `crates/terminus-core/src/config.rs` and `crates/terminus/src/cmd/verify.rs` before writing it, and leave out any the code does not claim.
  - **Private reporting (D-14, human step).** The Security tab's report form exists only once the owner turns on private vulnerability reporting for crenshawdev/terminus. That is a repository setting the owner changes on GitHub; the executor does not change it.
  - **Review stop (D-16).** Open `SECURITY.md` in RustRover for the owner.
- **Verify:**
  - `grep -n '^# Security policy$\|^## Reporting a vulnerability$\|^## Scope$' SECURITY.md` finds all three headings, and `grep -n 'Report a vulnerability' SECURITY.md` finds the reporting line.
  - `grep -niE 'no release|released version|not (yet )?published|first release' SECURITY.md` prints nothing.
  - `grep -n -i 'baley' SECURITY.md` prints nothing.
  - human-verify: the owner turns on private vulnerability reporting for crenshawdev/terminus, and `gh api repos/crenshawdev/terminus/private-vulnerability-reporting` returns `{"enabled":true}`.
  - human-verify: the owner reviews `SECURITY.md` in RustRover and accepts it.

## Notes

- **Why this is plan 1 of 4.** CONTEXT's plan shape asks for the license and side files, the docs pages and the README to be reviewed in turn. The README needs seven tasks against a ceiling of four per plan, so it spans plans 3 and 4, which share `README.md` and run in sequence.
- **Why the license switch is one task.** D-13 says the Apache statements are corrected "in the same change as D-01", so `LICENSE`, `Cargo.toml` and the three documents that state the license change in one commit.
