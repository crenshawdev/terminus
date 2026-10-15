---
phase: 9
plan: 3
requirements: [DOC-01]
files:
  - README.md
---

# Phase 9: README and User Docs - Plan 3

## Goal

The phase goal: the public repository gets a README for someone who has never heard of Terminus, opening on the reader's problem and leading with the security model and the signed release, with user docs beside it for installing, every command, every setting and how a release is built, signed and checked.

This plan writes the README from its title down to Hosts and platforms: the reader's problem and what Terminus does about it, where it stands, the security model, what it does and refuses, its known gaps, and how it works.

## Must be true when done

- **The README opens on the reader's problem (AC1).** `README.md` starts with `# Terminus`. The lines between the title and the Status line say that a new session or a compaction loses earlier work and that Claude Code deletes transcripts after `cleanupPeriodDays`, then what Terminus does about it, and they name no component, table or file.
- **The Status line says where it stands, once (AC1).** One `> **Status: ...**` line in Baley's form, holding the one sentence that says no release is published yet and the macOS builds have never run. No other README sentence says either.
- **The security model is prominent (AC2).** `## Trust` holds the brief's paragraph word for word, then "What leaves your machine" naming `install.sh`'s downloads and the archived text Terminus hands Claude Code, with `[privacy] redact_recall` and the injection budgets, then the paragraph on the signed release linking `docs/release.md`. `## What it is designed to refuse` names no daemon, no telemetry, no model call, no API key and no network connection from the binary, then the refusals with their triggers.
- **Capabilities and gaps are stated as they are on `main`.** `## What it does` lists only capabilities that work today, and `## Current state` lists exactly #47, #5, #1 and #26, each with its issue link.
- **The mechanism and the platforms are described (D-10).** `## How it works` covers the components, where the data is stored and in what form, and what happens on a crash or a context loss. `## Hosts and platforms` says Claude Code only, Linux and macOS on x86_64 and aarch64, and what each needs.
- **The sections stand in the brief's order so far.** `grep '^## ' README.md` prints Trust, What it does, What it is designed to refuse, Current state, How it works, and Hosts and platforms, in that order.

## Context

- Plan 3 of 4. Plans 3 and 4 both write `README.md`, so plan 4 runs after this one. This plan links `TRUST.md` from plan 1 and the docs pages from plan 2, so it runs after both.
- D-03, D-04, D-06, D-07, D-08, D-10 and D-11 in `.planning/phases/9/CONTEXT.md` bind this plan, and the brief at `.planning/phases/9/BRIEF.md` gives the section order and the writing rules. Where the brief and CONTEXT differ, CONTEXT wins: the brief's "facts from me" were checked and corrected at D-03.
- **The code is the source.** Every claim is true of the code on this branch, read from the file that implements it. `DESIGN-BRIEF.md` is a map only and is stale in places (D-18). Nothing from DESIGN-BRIEF §2's comparison with another tool is used (D-04).
- **Writing rules from the brief and CLAUDE.md.** Read `~/.claude/john-voice.md` in full before writing, and follow its README rules. Show, never claim: no word describing Terminus as serious, honest or the like. Name no other tool. No images, badges, emoji, superlatives or comparison tables. Nothing unbuilt reads as a feature. State limits, failures and refusals with the condition that triggers them. No counts that go stale: name the three recall tools rather than counting them. Keep it short, and move any procedure longer than a paragraph to a page under `docs/` and link it. Nothing about being open source or free beyond plan 4's Sponsoring line. Name every setting by its key and every command by its name.
- **Release status lives in one sentence (D-07).** Phase 10's Task 4 finds the Status sentence by "no release is published" and rewrites it, and nothing else in the README. So that sentence is the only one saying whether a release exists or that macOS has never run.
- After each task, open `README.md` in RustRover for the owner with `mcp__rustrover__open_file_in_editor` (memory `open-docs-in-rustrover`). The README's review stop is at the end of plan 4, once the file is whole. Commits are signed, stay on `cadence/v0.0.1`, and no PR is opened (D-15).
- Out of scope: the sections from Installing and updating down to License, which are plan 4's; any Lineage section (D-12); changing code to match the README.

## Tasks

### Task 1: Open the README on the reader's problem

- **Files:** README.md
- **Action:**
  - **Create `README.md`** with `# Terminus` as its first line.
  - **The opening (D-04).** The lines under the title start on the reader's problem: a new session or a compaction loses earlier work, and Claude Code deletes transcripts after `cleanupPeriodDays`, 30 days by default (`CLAUDE_DEFAULT_CLEANUP_DAYS` in `crates/terminus/src/cmd/install/mod.rs`). Then they say what Terminus does about it, in terms anyone can follow: it keeps every session and lets you and Claude find earlier work again. The sentence saying what Terminus does and for whom carries no adjectives (brief, item 1). These lines name no component, table or file: no hooks, MCP, database, index, blocks, `terminus.toml` or `.jsonl`. The mechanism comes later.
  - **The Status line (D-07).** Under the opening, one blockquote line in Baley's form, `> **Status: ...**` (`/code/baley/README.md`). It says plainly what works and what does not. Exactly one of its sentences says that no release is published yet and that the macOS builds have never run, and it uses the words "no release is published" so phase 10's Task 4 finds it. It may point to `#current-state` the way Baley's does.
  - **Avoid** any second sentence anywhere in the README about release status, "first release will be", or macOS never having run. A second one would survive phase 10 and go false.
- **Verify:**
  - `head -n 1 README.md` prints `# Terminus`, and `grep -c '^> \*\*Status:' README.md` prints 1.
  - `awk 'NR>1 && /^> \*\*Status:/{exit} NR>1' README.md | grep -c 'cleanupPeriodDays'` prints at least 1, and the same lines piped to `grep -niE 'hook|mcp|sqlite|database|ingest|index|fts|block|table|\.toml|\.jsonl|\.json|zstd|stdio|server'` print nothing.
  - `grep -niE 'no release is published|first release will be|never (been )?run' README.md` prints only the Status line.

### Task 2: Write the security model

- **Files:** README.md
- **Action:**
  - **`## Trust`, directly after the Status line (D-06), in this order:**
    - The brief's paragraph, unchanged: the line of `.planning/phases/9/BRIEF.md` starting "Terminus collects nothing about you", copied whole, with its `[TRUST.md](TRUST.md)` link.
    - **What leaves your machine.**, as a bold lead like Baley's, built from the code: every destination, what is sent to it, and what turns it on. `install.sh`'s `curl` downloads: the script itself from `raw.githubusercontent.com`, then the manifest, its signature and the binary from the GitHub releases on crenshawdev/terminus, or from the `https://` source the user passes. The archived text Terminus hands Claude Code through its hooks and through the `recall_search`, `recall_context` and `recall_get` tools, which Claude Code sends to its model. `[privacy] redact_recall` filters that text, and `[injection] brief_chars` and `prompt_chars` cap its size. Read when each hook hands text over from `crates/terminus/src/cmd/hook.rs` and `crates/terminus-core/src/inject/`.
    - It never says "Nothing." as the answer, because something leaves (D-06). It says nothing either way about whether the `claude mcp` commands install runs open a connection, because CONTEXT left that question flagged.
    - Then a short paragraph: every release is signed on the owner's machine and never in CI; `install.sh` checks the signature against the pinned key before it reads the manifest or downloads the binary; and the Terminus binary opens no network connection. It links `docs/release.md`. Back each from `.github/workflows/release.yml`, `install.sh` and `crates/terminus-core/tests/no_network.rs`.
  - **`## What it is designed to refuse`, after `## Trust` (D-11).** Leave room between them for plan 3's Task 3, which puts `## What it does` there. Two lists:
    - The flat refusals: no daemon, explained with the condition (the ingest and backfill a hook spawns run detached and exit, and the MCP server is a stdio child Claude Code starts for each session); no telemetry; no model call; no API key, with a `[provider]` table failing the config load; no network connection from the binary; and no encryption at rest, stated as a limit. Do not repeat DESIGN-BRIEF's "no long-lived process", because the MCP server lives as long as the session.
    - The refusals with their triggers: `install.sh` refuses a missing or bad signature, a missing `ssh-keygen`, `TERMINUS_BIN_DIR` set, an unsupported OS or architecture, and a file at `~/.local/bin/terminus` that is not a Terminus build; a build for any other OS fails at compile time; a store in another archive format is refused with the clean-install steps; excluded projects are never opened; retention deletes a session only after Claude Code has removed its transcript; and hooks fail open. Check each against its code before writing it: `install.sh`, `crates/terminus-core/src/lib.rs`, `crates/terminus-core/src/error.rs`, `crates/terminus-core/src/config.rs`, `crates/terminus/src/cmd/spawn.rs` and `crates/terminus-core/src/retention/`.
- **Verify:**
  - `grep -qxF "$(grep '^Terminus collects nothing about you' .planning/phases/9/BRIEF.md)" README.md` succeeds.
  - `sed -n '/^## Trust$/,/^## /p' README.md` contains each of `What leaves your machine`, `raw.githubusercontent.com`, `recall_search`, `recall_context`, `recall_get`, `redact_recall`, `brief_chars`, `prompt_chars`, `CI` and `docs/release.md`, and piped to `grep -nE '(^|\. |\*\* )Nothing\.'` prints nothing.
  - `sed -n '/^## What it is designed to refuse$/,/^## /p' README.md` matches, case-insensitively, each of `daemon`, `telemetry`, `model`, `API key`, `network connection`, `encryption`, `signature`, `ssh-keygen`, `TERMINUS_BIN_DIR`, `compile`, `archive format`, `exclu`, `retention` and `fail open`.
  - `grep -ci 'long-lived' README.md` prints 0.
  - Every relative link in `README.md` resolves: each `](target)` that is not `http` or `https`, with any `#anchor` removed, names a path that exists.

### Task 3: Write what it does and its known gaps

- **Files:** README.md
- **Action:**
  - **`## What it does`, between `## Trust` and `## What it is designed to refuse` (brief, item 4).** Bullets, each one capability that works on this branch, with the command or setting that reaches it. Find each one in the code before writing it, from `crates/terminus/src/main.rs`'s `USAGE` and the modules under `crates/terminus-core/src/`. Block rows are built (phase 7) and are described as built, not planned (D-03). The model add-on is gone, so nothing about observations calls a model.
  - **Avoid** anything unbuilt, anything planned, and any number that goes stale.
  - **`## Current state`, after `## What it is designed to refuse` (D-08).** Exactly four bullets, the gaps that touch a README claim, each with its issue linked in the form Baley uses (`[#47](https://github.com/crenshawdev/terminus/issues/47)`), and no counts: the tests do not run in CI (#47); `redact_recall` misses bare `sk-` style tokens (#5); `redact_recall` does a lossy UTF-8 round trip (#1); and the MCP tool results are undocumented (#26). Read each issue with `gh issue view` before writing its line, so the line says what the issue says. It does not list every open issue.
- **Verify:**
  - `sed -n '/^## What it does$/,/^## /p' README.md | grep -c '^- '` prints at least 1, and the same section piped to `grep -niE '\bwill\b|planned|coming|soon|future'` prints nothing.
  - `sed -n '/^## Current state$/,/^## /p' README.md | grep -c '^- '` prints 4, and the section contains `issues/47`, `issues/5)`, `issues/1)` and `issues/26`.
  - For each of 47, 5, 1 and 26, `gh issue view <n> --repo crenshawdev/terminus --json state --jq .state` prints `OPEN`.
  - `grep '^## ' README.md` prints Trust, What it does, What it is designed to refuse and Current state, in that order.

### Task 4: Write how it works and where it runs

- **Files:** README.md
- **Action:**
  - **`## How it works`, after `## Current state` (brief, item 7).** The mechanism, for an engineer. The components: one Rust binary; the hooks Claude Code runs on `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`, and what each does; the ingest each hook starts detached; the archive; the derived tables `terminus reindex` rebuilds; and the MCP server Claude Code starts over stdio for each session. Where the data is stored and in what form: the SQLite database in the data directory (link `docs/configuration.md` for where that is), each session stored as numbered blocks, one row each with its bytes and hash, and the session checksum over the block hashes, read from `crates/terminus-core/src/blob/block.rs` and `crates/terminus-core/src/store/`. What happens on a crash and on a context loss, read from `crates/terminus-core/src/ingest/`, `crates/terminus-core/src/recover.rs` and `crates/terminus-core/src/inject/`. Link `DESIGN-BRIEF.md` as the design doc.
  - **Avoid** linking `docs/design/`, which phase 11 writes and does not exist yet, and any claim the code does not back, wherever DESIGN-BRIEF says otherwise.
  - **`## Hosts and platforms`, after `## How it works` (D-10).** Claude Code only, with no other tool named. Linux and macOS on x86_64 and aarch64. Needed: `curl`, `ssh-keygen`, `sha256sum` or `shasum`, and `claude` on `PATH`, with the condition under which a missing `claude` leaves the MCP registration incomplete and install exits 1, as `register` in `crates/terminus/src/cmd/install/mod.rs` decides it. `git` is optional; without it, project identity degrades (`crates/terminus-core/src/project.rs`). No minimum Claude Code version, because `doctor` calls its 2.1.227 floor unproven. Nothing here says macOS has never run, because the Status sentence carries that (D-07).
- **Verify:**
  - `sed -n '/^## How it works$/,/^## /p' README.md` contains each of `SessionStart`, `UserPromptSubmit`, `SessionEnd`, `PostCompact`, `SQLite`, `block`, `hash`, `crash`, `DESIGN-BRIEF.md` and `docs/configuration.md`, and `grep -n 'docs/design' README.md` prints nothing.
  - `sed -n '/^## Hosts and platforms$/,/^## /p' README.md` contains each of `Claude Code`, `Linux`, `macOS`, `x86_64`, `aarch64`, `curl`, `ssh-keygen`, `sha256sum`, `shasum`, `claude` and `git`, and piped to `grep -niE 'never (been )?run|2\.1\.'` prints nothing.
  - `grep -niE 'claude-mem|codex|cursor|copilot|windsurf' README.md` prints nothing.
  - `grep '^## ' README.md` prints Trust, What it does, What it is designed to refuse, Current state, How it works, and Hosts and platforms, in that order.
  - Every relative link in `README.md` resolves, checked as in Task 2.

## Notes

- **Why the README spans plans 3 and 4.** It needs seven tasks against a ceiling of four per plan. The two plans share `README.md`, so they run in sequence, as phase 1's plans that shared files did.
- **The macOS clause after phase 10.** CONTEXT flags that phase 10's Task 4 has to drop the Status sentence's macOS clause when it rewrites that sentence. This plan keeps the clause inside that one sentence so the rewrite reaches it. Changing phase 10's Task 4 is not in this phase's scope, which allows one line in phase 10's Task 1 only.
