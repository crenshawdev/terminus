---
phase: 2
plan: 4
requirements: [INST-09]
files:
  - DESIGN-BRIEF.md
  - .planning/PROJECT.md
  - .planning/REQUIREMENTS.md
  - .planning/CAPTURE.md
  - CHANGELOG.md
---

# Phase 2: Linux and macOS Only, No npm - Plan 4

## Goal

The docs and planning files say what the code now does. Terminus supports Linux and macOS, Windows was dropped as it is across all of the owner's products, and there is no npm channel. The npm, Windows and icacls items still open in the capture list are closed as no longer applicable.

## Must be true when done

- `DESIGN-BRIEF.md` and `.planning/PROJECT.md` name Linux and macOS as the supported systems. Every remaining mention of Windows in them is about claude-mem or says Windows was dropped.
- npm is named only to say it was removed, in `DESIGN-BRIEF.md`, in `.planning/PROJECT.md` outside its shipped-cycle tables, and in `.planning/REQUIREMENTS.md` outside the INST-01 and INST-09 rows.
- `.planning/PROJECT.md` has a Key Decisions row saying Windows is dropped across all of the owner's products.
- `.planning/CAPTURE.md`'s npm items, its Windows item, its Publish milestone and its icacls proposal are marked closed as no longer applicable, and `capture-check` reports no annotations.
- `CHANGELOG.md`'s `[Unreleased]` has Removed entries for the npm package and for Windows support, and its rename entry names no `%LOCALAPPDATA%` path. The Verbatim 0.1.1 line about the Windows deferral is unchanged.
- `.planning/ROADMAP.md`, `.planning/ARCHIVE.md`, `.planning/phases/`, `.planning/reads.jsonl`, the PRIV-02 row, and the INST-01 and INST-09 rows are unchanged.

## Context

- Plan 4 of 4. It touches no file the code plans touch and needs none of their results. Its text describes the state they produce.
- Binding decisions: D-09 (which docs change and which stay), D-10 (CHANGELOG), D-11 (the icacls proposal is closed and the closure recorded in CAPTURE.md), and D-05 (case folding narrowed to macOS). D-11 also warns: find the ACL deferral by its content, never by searching for "D-15".
- Prose in all five files follows `~/.claude/john-voice.md`, the user's rule for doc prose. Read it before writing.
- Out of scope: designing the phase 4 installer (D-09 says to name it and not design it), the README (phase 5), and `windows-sys` in `Cargo.lock`.
- Plan 1 keeps `config.rs`'s `fold` doc citing `.planning/PROJECT.md` constraints for case folding. Task 2 keeps macOS case folding in the Compatibility constraint so that citation stays true.

## Tasks

### Task 1: The design brief describes Linux and macOS with no npm channel

- **Files:** DESIGN-BRIEF.md (anchor: §5 `Why not the alternatives`, §6 `Trigger model`, `Ingest run` and `Project identity`, §11 `Project exclusion`, §12 `Paths and configuration`, §13 `Install and packaging` with `Binary layout` and `Upgrade`, §15 `Testing`, §17 `Open items`)
- **Action:**
  - **Windows text (D-09, D-05).**
    - §6 `Trigger model`: the detached-spawn sentence ends at "new process group" and drops `DETACHED_PROCESS | CREATE_NO_WINDOW` on Windows.
    - §6 `Ingest run`, step 1: the try-lock is `flock` alone.
    - §6 `Project identity`: case-fold on macOS.
    - §11 `Project exclusion`: case-insensitive on macOS.
    - §12: the paths table loses its Windows column, so it is Purpose, Linux and macOS. The `**Windows specifics:**` line is replaced by a line naming the supported systems. It says Terminus supports Linux and macOS, that Windows was dropped as it is across all of the owner's products, and that a build for any other system fails at compile time with "Terminus supports Linux and macOS only."
    - §15: the Platform CI row is a Linux and macOS matrix.
    - §17: delete the `UserPromptSubmit` cost on Windows row. The question was the cost of a spawn on Windows, and no Windows build remains to measure.
  - **The Postgres row (AC7).** In §5 `Why not the alternatives`, the Postgres reason becomes "Requires a server." and drops "Windows is the larger user base".
  - **§13 (D-09).**
    - The opening says there is no npm channel, because the npm package was removed, and that a one-command installer arrives in phase 4. It does not describe how that installer works.
    - The esbuild paragraph goes, and with it the per-platform package names including `win32-x64`, `npx` and the postinstall point.
    - The secondary-channel line drops Scoop/WinGet and keeps GitHub releases, Homebrew and deb/rpm.
    - The Claude Code plugin paragraph stays.
  - **`Binary layout` and `Upgrade` (D-09, D-08).**
    - `Binary layout`: `terminus install` copies the running binary to the stable path. That copy is still canonical, with one binary, no version skew and no PATH change. Drop "out of the npm package" and the npm shim.
    - `Upgrade`: the new binary's `terminus install` re-copies it to the stable path, with no package manager named. "Hooks are never rewritten" and the rest of the paragraph stay.
  - **Stays (D-09).** The two claude-mem facts stay word for word: the "45 open Windows issues" bullet in §2, and "retires most of claude-mem's Windows issue class" in §6. Everything §13 says about `terminus install`'s steps, `--yes`, uninstall and doctor stays; phase 3 owns those.
- **Verify:**
  - `git grep -n -i windows -- DESIGN-BRIEF.md` prints the §2 claude-mem issue count, the §6 claude-mem issue-class sentence, and the new §12 supported-systems line, and nothing else.
  - `git grep -n -i -E 'npm|npx' -- DESIGN-BRIEF.md` prints only lines that say the npm package was removed or there is no npm channel.
  - `git grep -n -E 'LockFileEx|CREATE_NO_WINDOW|DETACHED_PROCESS|APPDATA|Scoop|WinGet|win32|\.exe' -- DESIGN-BRIEF.md` prints nothing.
  - §12's table header reads `| Purpose | Linux | macOS |`.

### Task 2: The project and requirements files state the platform and distribution as they now are

- **Files:** .planning/PROJECT.md (anchor: `## Context` open items, `## Constraints` Compatibility and Distribution, `## Key Decisions`), .planning/REQUIREMENTS.md (anchor: the v2 `### Distribution` Secondary channels bullet)
- **Action:**
  - **`PROJECT.md` open items (D-09).** Delete the "`UserPromptSubmit` cost on Windows" bullet. Task 1 deletes the same item from the brief, and the list says its dispositions are settled there.
  - **`PROJECT.md` Compatibility (D-09, D-05).** It says Linux and macOS only, that Windows was dropped as it is across all of the owner's products, that a build for any other system fails at compile time, and that path comparison is case-folded on macOS. The Windows list goes: `LockFileEx`, `DETACHED_PROCESS | CREATE_NO_WINDOW`, rename-then-replace for a running exe, `\\?\` long paths and UNC paths.
  - **`PROJECT.md` Distribution (D-09).** It says there is no npm channel because the package was removed, and that a one-command installer arrives in phase 4. "`origin` (self-hosted) is the only remote" stays.
  - **`PROJECT.md` Key Decisions (D-09).**
    - The row "Hooks call the absolute binary path, no shell; spawn fully detached" keeps its decision. Its rationale names the failure classes it retires (PATH probing, version resolution, inherited handles) without calling them a Windows issue class; crediting claude-mem is fine.
    - The npm row ("npm primary: thin package, per-platform `optionalDependencies` ...") becomes the decision that survives without npm: a canonical stable binary path and hooks that are never rewritten. Its rationale is one binary, no version skew and no version-check hook, and it may say the npm package was removed.
    - Add one row. Decision: Linux and macOS only, with Windows dropped across all of the owner's products. Rationale: the owner is removing Windows from every product, so it is removed outright rather than deferred, and a build for any other system fails at compile time. Outcome: `- Pending`, following the table's own rule that outcomes are pending until shipped and validated.
  - **`PROJECT.md` stays.** The paragraph listing this cycle's five phases already says the npm package is removed. The shipped-cycle tables (both Manifest rows) and the claude-mem incumbent paragraph stay as they are.
  - **`REQUIREMENTS.md` (D-09).** The v2 Distribution "Secondary channels" bullet drops Scoop/WinGet and "npm is the primary channel and ships first". It says there is no npm channel because the package was removed, and keeps GitHub releases, Homebrew and deb/rpm. Nothing else in the file changes. The PRIV-02 row stays as written (D-11), and so do the INST-01 and INST-09 rows.
- **Verify:**
  - `git grep -n -i windows -- .planning/PROJECT.md` prints the claude-mem incumbent paragraph, the Compatibility constraint and the new decision row, and nothing else.
  - `git grep -n -i npm -- .planning/PROJECT.md` prints the five-phases paragraph, the two Manifest rows in the shipped-cycle tables, and lines that say the npm package was removed.
  - `git grep -n -i -E 'npm|scoop|winget' -- .planning/REQUIREMENTS.md` prints the INST-09 line, the INST-01 traceability row and the Distribution bullet saying npm was removed.
  - `git diff -U0 -- .planning/REQUIREMENTS.md` changes only the Distribution bullet.
  - `git diff --stat -- .planning/ROADMAP.md .planning/ARCHIVE.md .planning/phases .planning/reads.jsonl` prints nothing.

### Task 3: The capture list closes its npm, Windows and icacls items

- **Files:** .planning/CAPTURE.md (anchor: the phase 4 todos about npm and AC3's Windows half, `### Publish milestone (discussed 2026-08-23, not yet a roadmap milestone)`, `**Proposed amendment to Phase 3 (v0.1.1, awaiting John's yes)**`)
- **Action:**
  - **How closure is marked (the planner's call, per CONTEXT's fourth flagged assumption).**
    - A closed bullet's checkbox is `[x]`, and it carries one appended sentence: "Closed <date>: no longer applicable, because phase 2 removed the npm package" or "... removed Windows support". `<date>` is the day the task runs.
    - The capture grammar indexes a checked bullet with a `[closed]` prefix, so recall stops presenting it as live work. Do not use the annotation shapes `KEPT <date>` or `recorded not fixed`, which `capture-check` reports as issues.
    - Keep every closed bullet on one line, because a continuation line is not a bullet.
  - **Todos, found by content (D-09).**
    - Close the six npm items: "The npm shim does not forward signals to the child"; "npm's os/cpu selection of an optionalDependency"; "Four of the five npm platform packages carry no binary"; "npm name checked 2026-08-23"; "Declined an engines.node field on the thin npm package"; and "shellcheck is not installed on this machine, so npm/pack-local.sh's static analysis".
    - Close the Windows item: "AC3's Windows half - no console window, no handle inherited from the hook" (recalled: CAPTURE.md and v0.1.0 phase 4 SUMMARY).
    - The phase 4 items between them about `ingest::backfill` and `fault::pass_fails_after` and about `cargo fmt --check` stay open and unchanged. They are not npm items, although they sit inside the range CONTEXT cites.
  - **The Publish milestone (D-09).**
    - Directly under its `###` heading, add one checked bullet. It says the milestone is closed as no longer applicable because phase 2 removed the npm package and Windows support and a one-command installer arrives in phase 4, and that the text below stays as the record of what was decided.
    - Every column-0 bullet in the section, from **Decided** through **Open, needs a decision before anything publishes**, becomes `- [x] `, which includes the `win32-x64` bullet. The prose and the continuation lines stay as written.
  - **The icacls proposal (D-11).** Directly under the bold "Proposed amendment to Phase 3" line, add one checked bullet. It says the proposal is closed as no longer applicable because Windows support was removed, so there is no Windows ACL to check and the Windows ACL deferral it would have replaced is closed too. Refer to that deferral by its content, not by "D-15" alone. The proposal's paragraphs stay below it as the record.
- **Verify:**
  - `node /home/john/.claude/plugins/cache/cadence/cadence/3.9.0/cadence-core/bin/planning.mjs capture-check` prints `"annotations":[]`.
  - `grep -n '^- ' .planning/CAPTURE.md | grep -i -E 'npm|windows|win32|icacls' | grep -v ':- \[x\]'` prints only the phase 8 `data move` item about the `LOCK` file (see Notes).
  - `perl -ne 'print if /^### Publish milestone/../^## Notes/' .planning/CAPTURE.md | grep -c -E '^- (\[ \]|[^[])'` prints `0`, so every bullet in the section is checked.
  - `git diff -U0 -- .planning/CAPTURE.md` touches only the seven todo lines and lines inside the Publish milestone section.

### Task 4: The changelog records both removals

- **Files:** CHANGELOG.md (anchor: `## [Unreleased]`, its `### Changed` rename entry and its `### Removed` list)
- **Action:**
  - **The rename entry (D-10).** Its data-directory parenthetical drops ", Windows `%LOCALAPPDATA%\terminus`" and keeps the macOS path. No Windows build of that path ever shipped.
  - **`### Removed` (D-10).** Add two entries after the existing ones.
    - The npm package: the `npm/` directory, meaning the thin `terminus` package, its five platform packages and `pack-local.sh`, is gone, and Terminus has no npm channel.
    - Windows support: Terminus builds on Linux and macOS only, and a build for any other system fails at compile time with "Terminus supports Linux and macOS only." That includes other Unix systems, which used to build against a generic data-directory fallback. Doctor and install print only `chmod` and `rm -f` fixes.
    - State what changed and nothing about future channels.
  - **Stays.** The Verbatim 0.1.1 entry ending "Unix only; `doctor` reports the mode and states the Windows deferral." is release history (D-10).
- **Verify:**
  - `git grep -n -i windows -- CHANGELOG.md` prints the new Windows entry and the Verbatim 0.1.1 line, and nothing else.
  - `git grep -n -i -E 'npm|LOCALAPPDATA' -- CHANGELOG.md` prints only the new npm entry.
  - `git diff -U0 -- CHANGELOG.md` changes only lines between `## [Unreleased]` and the next `## [` heading.

## Notes

- **The Postgres row** in `DESIGN-BRIEF.md` is not in D-09's list. It is changed because AC7 allows no other Windows mention in the brief.
- **Deliberately left for the human:**
  - `DESIGN-BRIEF.md` §6 says "No daemon, no systemd/launchd/Task Scheduler". Task Scheduler is Windows's scheduler, but the sentence names no Windows and stays true.
  - `CAPTURE.md`'s phase 8 item says `data move` skips the `LOCK` file because "copying a locked file is a read Windows refuses". D-02 keeps that behaviour and the item is about `data move`, so it is not closed. Closing or rewording it is the human's call.
- **The Windows open item** is deleted from both the brief and PROJECT.md rather than marked. That is this plan's reading of "brought in line": the item is a measurement on a system that no longer builds.
