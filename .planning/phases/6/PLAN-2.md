---
phase: 6
plan: 2
requirements: [INST-14]
files:
  - CHANGELOG.md
  - DESIGN-BRIEF.md
  - .planning/PROJECT.md
---

# Phase 6: Signed Install - Plan 2

## Goal

`install.sh` refuses a release it cannot verify against the pinned crenshawdev release key, and an update, which is a rerun of the same command, gets the same check. Plan 1 built that. This plan brings the CHANGELOG and the design brief in line with it, and corrects the two `PROJECT.md` lines that contradict the owner's 2026-10-14 ruling, using text the owner approves at plan review.

## Must be true when done

- **The CHANGELOG describes the signed install.** The `CHANGELOG.md` Unreleased section names the three downloads, the signature check against the pinned key under `terminus-release`, the refusals, and the need for `ssh-keygen` with its three package names. It no longer says "unsigned development artifacts only", "inside `verify_download`" or "the same two names", and it still says no release is published.
- **The design brief describes the signed install.** `DESIGN-BRIEF.md` §13 describes the same check as current state, including the principal, the namespace, where the allowed-signers line comes from, and that nothing overrides the key. It still says "No release is published yet."
- **`PROJECT.md` no longer contradicts the ruling.** Lines 21 and 23 hold the text in Task 3, or the owner's edit of it from plan review. `PROJECT.md` no longer says a release waits for the update feature, or that updates come with the signing Baley's #14 owns. No other line changes.

## Context

- Plan 2 of 2. It shares no file with plan 1. It runs after plan 1, as CONTEXT orders, because the docs describe what `install.sh` actually does. Read `install.sh` as plan 1 left it, and quote no message the script does not print.
- D-14 binds Tasks 1 and 2. D-15 binds Task 3. Both are in `.planning/phases/6/CONTEXT.md`. "No release is published yet" stays until phase 7.
- CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing this prose. Its "README, project docs" rules apply to the CHANGELOG, and its "Design docs" rule applies to the brief: describe the system as it stands now, never as a change log.
- Out of scope: the README (phase 8), the release and its signing script (phase 7), any `PROJECT.md` line other than 21 and 23, `REQUIREMENTS.md`, `ROADMAP.md` and `STATE.md`.

## Tasks

### Task 1: Describe the signed install in the CHANGELOG

- **Files:** CHANGELOG.md (anchor: `## [Unreleased]`, `### Added`, the `install.sh` entries)
- **Action:**
  - **The one-command entry.** It now says the script downloads `terminus-<os>-<arch>.manifest`, `terminus-<os>-<arch>.manifest.sig` and `terminus-<os>-<arch>`. Under a custom source, "the same two names" becomes the same three.
  - **The check, before the SHA-256 sentence.** Before it reads the manifest, the script verifies the signature with `ssh-keygen -Y verify` against the crenshawdev release key pinned in `install.sh`, under the namespace `terminus-release`. It downloads the binary only after that. The SHA-256 check in `verify_download` then runs as before.
  - **Replace the bullet that says "`install.sh` installs unsigned development artifacts only ...".** The new bullet covers:
    - The script refuses a missing signature, a signature by another key or under another namespace, and a manifest changed after signing.
    - It refuses a machine without `ssh-keygen` before it downloads anything, naming `openssh-client` (Debian, Ubuntu), `openssh-clients` (Fedora) and `openssh` (Arch).
    - The key is never fetched, and no argument, setting or environment variable changes it, so a source passed as the argument can only serve releases signed with that key.
    - Keep the sentence saying no release is published.
  - **The key itself.** Do not quote the public key line in the CHANGELOG. `install.sh` holds the one copy, and plan 1's source test guards it there.
  - **Leave everything else.** Every other Unreleased entry stays as it is, along with every released section below it.
- **Verify:**
  - `grep -n -e 'unsigned development' -e 'inside .verify_download' -e 'same two names' CHANGELOG.md` prints nothing.
  - The Unreleased section contains `manifest.sig`, `ssh-keygen`, `terminus-release`, `openssh-client`, `openssh-clients`, `openssh` and a sentence saying no release is published.
  - `git diff CHANGELOG.md` changes lines only between `## [Unreleased]` and the next `## [` heading.

### Task 2: Describe the signed install in DESIGN-BRIEF §13

- **Files:** DESIGN-BRIEF.md (anchor: `## 13. Install and packaging`, its first paragraph, and the `install.sh` bullet under `### Binary layout` that begins "`install.sh` downloads")
- **Action:**
  - **Current state, in detail.** State the following as how the system works now:
    - The downloads, in order: the manifest, its `.manifest.sig`, and the binary only after the signature verifies.
    - The check: the script writes the allowed-signers line `crenshawdev-release <pinned key>` from its own literal into its temporary folder, then runs `ssh-keygen -Y verify -I crenshawdev-release -n terminus-release` with the manifest on stdin, before it reads the manifest. Nothing is read from HOME or `~/.ssh`.
    - Output: `ssh-keygen`'s output is not shown, and the script prints its own reason.
    - The SHA-256 check in `verify_download` follows as before.
    - `ssh-keygen` is required. The script checks for it before any download and names `openssh-client` (Debian, Ubuntu), `openssh-clients` (Fedora) and `openssh` (Arch).
    - No override: every source, including the `https://` argument, must serve a signature by the pinned key, and there is no unsigned mode. The next bullet already gives the local-build path, so do not repeat it.
  - **Keep "No release is published yet."** in the first paragraph.
  - **The key itself.** Do not quote the public key line, for the same reason as Task 1.
  - **Leave the rest of §13.** `### Upgrade` and the other subsections stay as they are.
- **Verify:**
  - Between `## 13.` and `## 14.`, `DESIGN-BRIEF.md` contains `manifest.sig`, `ssh-keygen -Y verify`, `crenshawdev-release`, `terminus-release`, `openssh-client`, `openssh-clients`, `openssh` and `No release is published yet.`
  - `git diff DESIGN-BRIEF.md` changes lines only inside §13.

### Task 3: Correct `PROJECT.md` lines 21 and 23 with the approved text

- **Files:** .planning/PROJECT.md (anchor: the two paragraphs under `**v0.0.1, opened 2026-10-12.**`)
- **Action:**
  - **The owner approves this text at plan review, before it is written (D-15).** If he edits it there, his version is the one written, word for word.
  - **Line 21.** Replace it whole with this paragraph:

    > Eight phases. Four bring Terminus in line with Baley's ADR 0038 and 0039: no model calls and so no API keys and no network use, Linux and macOS only with the npm package removed, an install that asks nothing and registers the MCP server through `claude mcp`, and a one-command installer script. The fifth points that script at GitHub releases by default, and the sixth has it refuse a release whose signature does not verify against the crenshawdev release key pinned in the script. The seventh builds each release in CI, and John signs and publishes it from his own machine. The eighth writes the README the public repository does not have yet. An update is a rerun of the install command, so it gets the same check.

  - **Line 23.** Replace its first sentence, "No release is published until the update feature is built.", with the sentence below, and remove its last sentence, "v0.0.1 is merged but not released.", which is false (the owner approved both at plan review, 2026-10-14). The sentences between them stay exactly as they are.

    > Phase 7 publishes the first release, signed, without waiting for Baley.

  - **Nothing else changes.** Change no other line, including the `*Last updated:*` footer.
- **Verify:**
  - `sed -n '21p;23p' .planning/PROJECT.md` prints the approved text.
  - `git diff -U0 .planning/PROJECT.md` shows exactly two changed lines, 21 and 23.
  - `grep -c -e 'until the update feature is built' -e '(#14)' -e 'merged but not released' .planning/PROJECT.md` prints `0`.
