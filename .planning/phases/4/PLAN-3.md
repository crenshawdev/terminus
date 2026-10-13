---
phase: 4
plan: 3
requirements: [INST-12]
files:
  - DESIGN-BRIEF.md
  - .planning/PROJECT.md
  - CHANGELOG.md
---

# Phase 4: One-Command Installer - Plan 3

## Goal

The docs describe what plans 1 and 2 built. The stable path is a link into a versions folder. `install.sh` downloads and installs a version, and its download is the one network use; the binary opens no network connection.

## Must be true when done

- **DESIGN-BRIEF's binary layout describes the link.** Its §13 "Binary layout" and "Upgrade" describe:
  - the link from `~/.local/bin/terminus` into `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`
  - how `install.sh` and `terminus install` each put a version there
  - that old versions stay
  - that uninstall removes the link and leaves the versions

  No sentence says install copies the binary to the stable path or that a copy is canonical. `grep -n "re-copies\|That copy is canonical\|copies the running binary" DESIGN-BRIEF.md` prints nothing.
- **DESIGN-BRIEF §12's paths table matches §13.** It has a row for the versions folder, and its row for `~/.local/bin/terminus` no longer calls it the canonical binary.
- **No doc says the installer is still to come.** "arrives in phase 4" appears in neither DESIGN-BRIEF §13 nor PROJECT.md's Distribution constraint.
- **DESIGN-BRIEF §10 qualifies "no network".** It says the Terminus binary opens no network connection and that `install.sh`'s download is the one network use.
- **PROJECT.md's privacy line is corrected.** It no longer says "except the configured model provider", "redaction at egress only", or "credentials `0600`-enforced". Its Privacy constraint says the binary opens no network connection and that `install.sh`'s download is the one network use.
- **The changelog records the installer.** `CHANGELOG.md` `[Unreleased]` records:
  - `install.sh`
  - the link and the versions folder
  - `terminus install` storing itself as a version, and the local-build command
  - uninstall leaving the versions
- **Nothing in the new prose is made up.** Every path, command and name it uses exists in `install.sh`, `binary.rs` or `uninstall.rs` as plans 1 and 2 left them.

## Context

- This is plan 3 of 3 and runs after plans 1 and 2.
- Decisions that bind this plan: D-13 (the network statement) and D-14 (the docs), with D-01, D-02, D-03, D-04 and D-11 as the facts the prose states.
- This prose goes out under John's name. Per `~/.claude/CLAUDE.md`, read `~/.claude/john-voice.md` in full before writing, from the file every time, and follow its rules for project docs and for changelog entries.
- Before writing, read `install.sh`, `crates/terminus/src/cmd/install/binary.rs` and `crates/terminus/src/cmd/uninstall.rs` as they are now. Describe what they do, not what this plan says they do.
- Out of scope: the README (phase 5).

## Tasks

### Task 1: DESIGN-BRIEF's binary layout describes the link and the versions folder

- **Files:** DESIGN-BRIEF.md (anchor: §13 "### Binary layout", "### Upgrade")
- **Action:**
  - **Rewrite "Binary layout" (D-14).** It replaces "copies the running binary" and "That copy is canonical" with these points:
    - `~/.local/bin/terminus` is a symbolic link to `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`.
    - The hooks and the MCP registration name the link, so they are written once.
    - The versions folder sits under HOME and outside the data directory. The data directory moves with `TERMINUS_DATA_DIR` and `terminus data move`, and `--purge` deletes it, which would delete the file every hook runs (D-01).
    - `install.sh` stages the version and moves the link.
    - `terminus install` run from anywhere else stores itself under its own version and points the link at it, so a local build is `cargo build --release && target/release/terminus install` (D-02). Run from the link's own target, it changes neither.
    - A version folder holding different bytes gets the new file renamed over it (D-03).
    - Old version folders are never deleted, so a running session keeps its file (D-04).
    - Uninstall removes the link and leaves the versions folder (D-11).
    - "No PATH modification" stays.
  - **"Upgrade" (D-14).** Replace "re-copies it to the stable path" with the link moving to the new version's file. The rest of the paragraph stays.
  - **§12's paths table (added by the owner, 2026-10-13).** The `~/.local/bin/terminus` row stops calling it "Binary (canonical)" and says it is the stable link. Add a row for `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, with the same macOS column convention the other rows use.
  - **"A one-command installer arrives in phase 4" in §13 (added by the owner, 2026-10-13).** Replace it with a sentence naming `install.sh` as the one-command installer, run as `sh install.sh https://<source>`, and saying no release is published yet.
- **Verify:** `grep -n "re-copies\|That copy is canonical\|copies the running binary\|Binary (canonical)\|arrives in phase 4" DESIGN-BRIEF.md` prints nothing. `grep -n "crenshawdev/terminus/versions" DESIGN-BRIEF.md` prints a line in §12 and a line in §13. `grep -n "crenshawdev" crates/terminus/src/cmd/install/binary.rs install.sh` shows the same path spelled the same way.

### Task 2: The network statement names install.sh's download as the one network use

- **Files:** .planning/PROJECT.md (anchor: "## Constraints", the **Privacy** line), DESIGN-BRIEF.md (anchor: "## 10. Privacy and the data boundary")
- **Action:**
  - **PROJECT.md (D-13).** In the Privacy constraint, replace "no network connections except the configured model provider" with a statement that the Terminus binary opens no network connection and that `install.sh`'s `curl` download is the one network use.
  - **The rest of PROJECT.md's Privacy line (added by the owner, 2026-10-13).** Drop "redaction at egress only" and "credentials `0600`-enforced and redacted at the boundary". Both have been stale since phase 1 removed the model calls and stopped reading `credentials.toml`. Add no new claim in their place unless the code backs it.
  - **PROJECT.md's Distribution constraint (added by the owner, 2026-10-13).** Replace "a one-command installer arrives in phase 4" with `install.sh` as the one-command installer, and say no release is published yet. The rest of the line stays.
  - **DESIGN-BRIEF §10 (D-13).** The contract quote's "opens no network connection" and the closing "No network connections." are about the binary. Make §10 say so, and say that `install.sh`'s download is the one network use. Keep the quote's other sentences as they are.
- **Verify:** `grep -n "configured model provider\|redaction at egress\|0600\|arrives in phase 4" .planning/PROJECT.md` prints nothing. `grep -n "install.sh" .planning/PROJECT.md DESIGN-BRIEF.md` shows the network statement in both files and the installer in PROJECT.md's Distribution line.

### Task 3: The changelog records install.sh and the link

- **Files:** CHANGELOG.md (anchor: "## [Unreleased]")
- **Action:** Under `[Unreleased]`, follow Keep a Changelog's section order, with an Added section before the existing Changed section. Record the following.
  - **Added.** `install.sh`, run as `sh install.sh https://<source>`:
    - what it downloads for which systems
    - that `verify_download` checks the SHA-256 before anything is placed
    - that it installs unsigned development artifacts only, and no release is published
    - that it refuses `TERMINUS_BIN_DIR` and a stable path holding a file that is not a Terminus build
    - that its download is the one network use, and the binary opens none
  - **Changed.**
    - `~/.local/bin/terminus` is now a link into `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`.
    - `terminus install` stores itself as a version, so a local build is `cargo build --release && target/release/terminus install`.
    - A rebuild at the same version replaces its file.
    - Old versions are kept, and nothing prunes them.
    - An existing plain-file install becomes the link on the next install.
    - `terminus uninstall` removes the link, leaves the versions folder, and prints its path.
- **Verify:** `perl -ne 'print if /^## \[Unreleased\]/../^## \[Verbatim/' CHANGELOG.md` shows entries naming `install.sh`, `crenshawdev/terminus/versions`, `target/release/terminus install`, and uninstall leaving the versions folder.

## Notes

- The planner found three stale spots outside D-13 and D-14: DESIGN-BRIEF §12's paths table, the "arrives in phase 4" lines in DESIGN-BRIEF §13 and PROJECT.md's Distribution constraint, and the rest of PROJECT.md's Privacy line. On 2026-10-13 the owner added all three to tasks 1 and 2. The §12 change also answers the plan review's finding on §12.
