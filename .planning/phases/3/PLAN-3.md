---
phase: 3
plan: 3
requirements: [INST-10, INST-11]
files:
  - DESIGN-BRIEF.md
  - docs/json-shapes.md
  - CHANGELOG.md
---

# Phase 3: Install Asks Nothing - Plan 3

## Goal

The design brief, the JSON shapes doc and the changelog describe install and uninstall as they now run. Install asks nothing, places the MCP entry through `claude mcp`, and ends with a receipt. Uninstall removes the entry with `claude mcp remove`.

## Must be true when done

- In `DESIGN-BRIEF.md` and `docs/json-shapes.md`, install has no diff, no confirmation, no `--yes`, no data-directory question and no offer to raise `cleanupPeriodDays`.
- The `terminus install` section of the design brief lists the steps install now takes, including `claude mcp add-json --scope user` and the receipt.
- `docs/json-shapes.md` still says `install` and `uninstall` have no `--json` (D-24), and the reason it gives is true of both commands today.
- The `[Unreleased]` section of `CHANGELOG.md` records the change. It names no command, flag or path that the code does not have.

## Context

- This is plan 3 of 3. It runs after plans 1 and 2. Take install's, uninstall's and doctor's behaviour from the code (`crates/terminus/src/cmd/install/mod.rs`, `uninstall.rs` and `doctor.rs`), not from the earlier plans' wording.
- D-19 decides which passages change:
  - `DESIGN-BRIEF.md`: the §5 bullet about the install-time offer, and the `terminus install` section of §13.
  - `docs/json-shapes.md`: the D-24 reason at the end of the `doctor` section.
  - `CHANGELOG.md`: new entries under `[Unreleased]`.
- The owner's `CLAUDE.md` says to read `~/.claude/john-voice.md` in full before writing any doc prose he puts his name on. Its "Design docs and diagrams" rule says a design doc describes the system as it stands now, never as a log of changes.
- Out of scope:
  - the README, which is phase 5's work
  - the brief's uninstall paragraph and its doctor check list, which D-19 does not name and which are both still true
  - everything §13 says about the binary layout, upgrade and plugins

## Tasks

### Task 1: The design brief and the JSON shapes doc describe install as it now runs

- **Files:** DESIGN-BRIEF.md (anchor: §5's "Upstream transcripts are a second recovery path" bullet, and §13's `### \`terminus install\`` section), docs/json-shapes.md (anchor: the D-24 paragraph that closes the `doctor` section)
- **Action:**
  - Read `~/.claude/john-voice.md` in full before writing.
  - **§5 bullet.** The install-time offer goes. In its place, say that the install receipt shows the current `cleanupPeriodDays`, says that transcripts older than it are already gone, and gives the line that raises it. Terminus never changes the setting itself.
  - **§13, `terminus install`.** Rewrite the steps to match the code as it stands. Keep the canonicalized config roots step. The data-directory question goes, because the code never asked it (D-19). The steps are:
    - **Hooks.** The four hook entries are merged into `settings.json` with a one-time backup, atomically and idempotently, without a question. A file not in the two-space format is rewritten in full (D-12).
    - **MCP entry.** The entry is placed with `claude mcp add-json --scope user terminus '<JSON>'`: stdio, the stable path, and `args` of `["mcp"]`.
      - No `claude` runs when the entry is already correct, and an entry in the wrong shape is removed and then added again.
      - `claude` is found on `PATH` and runs with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`.
      - Terminus never writes `~/.claude.json`.
      - When `claude` is missing or fails, the install is incomplete: it prints the exact command and exits non-zero.
    - **Backfill.** The backfill is estimated and started detached, as the current step 6 says.
    - **Receipt.** The receipt lists every change, the backfill estimate and `terminus status`, the `cleanupPeriodDays` lines, the auto-compact recommendation, and how to undo the install.

    Replace the `--yes` line with a statement that install asks nothing.
  - **`docs/json-shapes.md`.** Keep "`install` and `uninstall` deliberately have no `--json` (D-24)". Replace "They show a diff and ask a question" with a reason that is true now. Read that reason off the code: install prints a receipt for a person, and uninstall asks before `--purge` deletes.
- **Verify:**
  - `git grep -n -i -e 'offer to raise' -e '\*\*offer\*\*' -e 'confirm once' -e 'show the exact' -e 'ask for the data directory' -e 'show a diff' -e 'accepts defaults' -- DESIGN-BRIEF.md docs/json-shapes.md` prints nothing.
  - `git grep -n -e '--yes' -- DESIGN-BRIEF.md docs/json-shapes.md` prints nothing.
  - `git grep -n 'claude mcp add-json --scope user' -- DESIGN-BRIEF.md` prints a line inside §13.
  - `git grep -n 'no `--json` (D-24)' -- docs/json-shapes.md` still prints the paragraph.
  - Every command, flag, key and path the rewritten passages name exists in `crates/terminus/src`. Check each one with `git grep`.

### Task 2: The changelog records the change

- **Files:** CHANGELOG.md (anchor: `## [Unreleased]`)
- **Action:**
  - Read `~/.claude/john-voice.md` in full before writing. Follow the file's own Keep a Changelog layout and the style of its existing entries.
  - **Under `### Changed`:**
    - `terminus install` asks nothing, applies its defaults and ends with a receipt. The receipt lists every change, the backfill estimate, where to watch it, and the current `cleanupPeriodDays` with the line that raises it.
    - Install places the MCP entry with `claude mcp add-json --scope user` and never writes `~/.claude.json`. It runs no `claude` when the entry is already correct.
    - A missing or failing `claude`, or a `~/.claude.json` that does not parse, leaves the install incomplete, not aborted. The binary and the hooks are still placed, the exact command is printed, and the exit status is non-zero.
    - Every `claude` that Terminus starts gets `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, so wiring Terminus up opens no network connection.
    - Uninstall removes the entry with `claude mcp remove --scope user terminus`. It leaves an existing `.claude.json.terminus-backup` in place and prints its path, without ever restoring it.
    - When `claude` is not on `PATH`, doctor's MCP fix is the `claude mcp add-json` command.
    - A `settings.json` that is not in the two-space format is rewritten in full without a question, and its backup holds the original.
  - **Under `### Removed`:**
    - the install diff and its confirmation
    - `install --yes`, which now exits 2 as an unknown flag
    - the offer to raise `cleanupPeriodDays`

    `uninstall --purge --yes` is unchanged. Say so where the removed `--yes` is named, so a reader does not take it for both.
- **Verify:**
  - `sed -n '/^## \[Unreleased\]/,/^## \[Verbatim/p' CHANGELOG.md` shows entries naming `claude mcp add-json --scope user`, `claude mcp remove --scope user terminus`, `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`, `.claude.json.terminus-backup`, `cleanupPeriodDays`, the receipt, the non-zero exit of an incomplete install, and `--yes` as removed from install only.
  - Every command, flag and path those entries name exists in `crates/terminus/src`. Check each one with `git grep`.
