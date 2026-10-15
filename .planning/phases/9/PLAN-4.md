---
phase: 9
plan: 4
requirements: [DOC-01]
files:
  - README.md
  - .planning/phases/9/CLAIMS.md
  - .planning/phases/10/PLAN.md
---

# Phase 9: README and User Docs - Plan 4

## Goal

The phase goal: the public repository gets a README for someone who has never heard of Terminus, opening on the reader's problem and leading with the security model and the signed release, with user docs beside it for installing, every command, every setting and how a release is built, signed and checked.

This plan finishes the README from Installing and updating down to License, takes it through its review stop, and writes the list of claims that phase 10's landing PR carries.

## Must be true when done

- **The install section is the one line (AC3).** `## Installing and updating` gives `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`, says it checks the release's signature and that rerunning it updates Terminus, gives `cargo build --release && target/release/terminus install` for a local build, says Linux and macOS only, and links `docs/install.md`.
- **Every command and every setting is listed (AC4).** `## Commands` has one line for every command in `main.rs`'s `USAGE`, and `## Settings` has every `terminus.toml` key with its default, each linking its docs page.
- **The README is whole, in the brief's order.** `grep '^## ' README.md` prints Trust, What it does, What it is designed to refuse, Current state, How it works, Hosts and platforms, Installing and updating, Commands, Settings, How it is designed and built, Issues and contributions, Sponsoring and License, and nothing else. The License section reads `[MIT](LICENSE)`.
- **The README names nothing the code lacks (AC7, human-verify).** `/cad-docs-verify README.md` reports no stale claim, and the owner has reviewed `README.md` in RustRover (D-16).
- **The claims reach the landing PR (AC7).** `.planning/phases/9/CLAIMS.md` lists each claim in the README's `## Trust` and `## Current state` beside the file or issue that backs it, and phase 10's Task 1 says the landing PR description carries that list.

## Context

- Plan 4 of 4. It runs after plan 3, because both write `README.md`, and after plans 1 and 2, whose files the README links.
- D-01, D-05, D-09, D-12, D-15, D-16, D-18, D-19 and D-21 in `.planning/phases/9/CONTEXT.md` bind this plan, along with the brief's text for Issues and contributions and for Sponsoring.
- The writing rules in plan 3's Context hold here unchanged: read `~/.claude/john-voice.md` in full first, read every fact from the code, name no other tool, no counts that go stale, nothing unbuilt as a feature, and keep procedures in `docs/`. The Status sentence stays the only one about release status or macOS never having run (D-07).
- After each README task, open `README.md` in RustRover for the owner with `mcp__rustrover__open_file_in_editor` (memory `open-docs-in-rustrover`). Commits are signed, stay on `cadence/v0.0.1`, and no PR is opened (D-15).
- Out of scope: any Lineage section (D-12), linking `docs/design/` (phase 11), a community link, which no decision names, and any edit to phase 10's plan beyond the one line in Task 3.

## Tasks

### Task 1: Write installing, commands and settings

- **Files:** README.md
- **Action:**
  - **`## Installing and updating`, after `## Hosts and platforms` (D-21).** Short: the one line `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`; that it checks the release's signature against the pinned key before it installs anything; that rerunning the same line is how Terminus updates; the local build `cargo build --release && target/release/terminus install`; and Linux and macOS only. Link `docs/install.md` for the rest, including uninstalling. The `sh -s -- https://<source>` form stays out of the README, in the user docs only.
  - **`## Commands`, after `## Installing and updating` (D-05, D-18).** One line for each command in `USAGE` in `crates/terminus/src/main.rs`, saying what it does: `ingest`, `ingest <path.jsonl>`, `backfill`, `hook <event>`, `search <words>`, `show <id>...`, `sessions`, `verify`, `reindex`, `status`, `compact`, `usage`, `export <dir>`, `stats`, `observations`, `observations regenerate`, `replay`, `retention --dry-run`, `doctor`, `mcp`, `install`, `uninstall` and `data move <path>`. `hook <event>` and `mcp` are marked as run by Claude Code, not typed. `backfill --work` does not appear. Say that data commands accept `--json`, and link `docs/commands.md` for every option.
  - **`## Settings`, after `## Commands` (D-05, D-19).** Each `terminus.toml` key with its default: `roots`, `exclude`, `[injection] brief_chars` and `prompt_chars`, `[retention] action` and `age_days`, `[retention.project."<path>"]`, `[snapshot] enabled`, `interval_hours` and `keep`, `[capture] mode` and `[privacy] redact_recall`, with the defaults D-19 gives and `config.rs` holds. One line on where the file is read from, and a link to `docs/configuration.md` for each key's effect and the environment variables.
- **Verify:**
  - `sed -n '/^## Installing and updating$/,/^## /p' README.md` contains `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`, `cargo build --release && target/release/terminus install`, `signature`, `Linux`, `macOS` and `docs/install.md`, and `grep -c 'sh -s --' README.md` prints 0.
  - For each of `ingest`, `backfill`, `hook`, `search`, `show`, `sessions`, `verify`, `reindex`, `status`, `compact`, `usage`, `export`, `stats`, `observations regenerate`, `replay`, `retention`, `doctor`, `mcp`, `install`, `uninstall` and `data move`, `sed -n '/^## Commands$/,/^## /p' README.md | grep -qF <command>` succeeds, and the same section contains `docs/commands.md` and no `--work`.
  - For each of `roots`, `exclude`, `brief_chars`, `prompt_chars`, `action`, `age_days`, `[retention.project.`, `enabled`, `interval_hours`, `keep`, `mode` and `redact_recall`, `sed -n '/^## Settings$/,/^## /p' README.md | grep -qF -- <key>` succeeds, and the section matches `6,?000`, `4,?000`, `24`, `full`, `false` and `docs/configuration.md`.
  - Every relative link in `README.md` resolves: each `](target)` that is not `http` or `https`, with any `#anchor` removed, names a path that exists.

### Task 2: Close the README and take it through its review stop

- **Files:** README.md
- **Action:**
  - **`## How it is designed and built`, after `## Settings` (D-09).** What is true today: `main` is protected and changes reach it through a pull request; no status check is required and no workflow runs the tests (#47), so the tests run locally with `cargo test --workspace --all-features`; design decisions live in `DESIGN-BRIEF.md` and `.planning/`; and the issues are at github.com/crenshawdev/terminus/issues. Baley's list of required checks is not copied, and `docs/design/` is not linked until phase 11 writes it.
  - **`## Issues and contributions`** is the brief's text, word for word: "Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue." `SECURITY.md` exists from plan 1 (D-14).
  - **`## Sponsoring`** is the brief's text, word for word: "Terminus is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it."
  - **`## License`** reads `[MIT](LICENSE)` (D-01).
  - **No Lineage section (D-12).** The history shows only the rename from Verbatim, which the CHANGELOG records.
  - **Review stop (D-16).** With the README whole, open it in RustRover and wait for the owner. Then ask the owner to run `/cad-docs-verify README.md`. A finding in the README is fixed here. A finding in a file another plan wrote is reported to the owner and not edited, since this plan does not lease it.
- **Verify:**
  - `grep '^## ' README.md` prints exactly these lines, in this order and with nothing else: `## Trust`, `## What it does`, `## What it is designed to refuse`, `## Current state`, `## How it works`, `## Hosts and platforms`, `## Installing and updating`, `## Commands`, `## Settings`, `## How it is designed and built`, `## Issues and contributions`, `## Sponsoring`, `## License`.
  - `grep -qxF 'Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue.' README.md` succeeds, and so does `grep -qxF 'Terminus is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it.' README.md`.
  - `grep -qx '\[MIT\](LICENSE)' README.md` succeeds.
  - `sed -n '/^## How it is designed and built$/,/^## /p' README.md` contains `cargo test --workspace --all-features`, `issues/47`, `DESIGN-BRIEF.md` and `.planning/`, and `grep -n 'docs/design\|cargo-deny\|clippy' README.md` prints nothing.
  - The brief's bans hold: `grep -c '!\[' README.md` prints 0; `grep -ci 'shields.io\|badge' README.md` prints 0; `grep -P '[\x{1F300}-\x{1FAFF}\x{2600}-\x{27BF}]' README.md` prints nothing; `grep -ci 'open source' README.md` prints 1; `grep -ciw 'free' README.md` prints 0.
  - `grep -niE 'no release is published|first release will be|never (been )?run' README.md` prints only the Status line, and `grep -niE 'claude-mem|codex|cursor|copilot|windsurf' README.md` prints nothing.
  - Every relative link in `README.md` resolves, checked as in Task 1.
  - human-verify (AC7): the owner runs `/cad-docs-verify README.md`, and it reports no stale claim.
  - human-verify (D-16): the owner reviews `README.md` in RustRover and accepts it.

### Task 3: Write the claim list and point phase 10's landing PR at it

- **Files:** .planning/phases/9/CLAIMS.md, .planning/phases/10/PLAN.md (anchor: the Land bullet of Task 1)
- **Action:**
  - **The list (D-15).** `.planning/phases/9/CLAIMS.md` takes each claim the accepted README makes in `## Trust` and in `## Current state`, one entry per claim, beside the file or issue that backs it. A file is named by its path and the symbol, test or line text that shows the claim, never a bare line number, because line numbers move before phase 10 lands. An issue is named by its number and link. The location is chosen here: the phase directory keeps the list with the decision that asked for it, and phase 10 reads it from there.
  - **Written to be pasted.** The list goes into the landing PR's description, which John sends as himself, so it follows the PR description rules in `~/.claude/john-voice.md`.
  - **Phase 10's Task 1 (D-15).** Add one sub-bullet under Task 1's **Land (D-02).** bullet in `.planning/phases/10/PLAN.md`, saying the PR description carries the list in `.planning/phases/9/CLAIMS.md`, each claim in the README's Trust and Current state sections beside the file or issue that backs it (phase 9, D-15). Change no other line of phase 10's plan.
- **Verify:**
  - Every repository file the list names as backing exists: each one passes `test -e` from the repository root. Paths that are part of a claim's own wording, such as `~/.local/bin/terminus`, and URLs are not backing files and are not tested.
  - For each issue number named in `.planning/phases/9/CLAIMS.md`, `gh issue view <n> --repo crenshawdev/terminus --json state --jq .state` prints `OPEN`, and the numbers include 47, 5, 1 and 26.
  - `git diff --numstat .planning/phases/10/PLAN.md` prints `1	0	.planning/phases/10/PLAN.md`, and `sed -n '/^### Task 1:/,/^### Task 2:/p' .planning/phases/10/PLAN.md | grep -n 'CLAIMS.md'` finds the new line.
  - human-verify: the owner reads `.planning/phases/9/CLAIMS.md` beside the README and finds an entry for every claim in `## Trust` and `## Current state`.

## Notes

- **Why the claim list is written last.** It has to match the README the owner accepted, so it follows Task 2's review stop.
- **Why `/cad-docs-verify` runs over `README.md` here.** Plan 2 ran it over `docs/`, where its findings could be fixed under that plan's lease. Together the two runs cover what AC7 names.
- **Recalled prior art.** Phase 6's UAT asked that the CHANGELOG, `DESIGN-BRIEF.md` and the `install.sh` header keep saying no release is published until one is (`.planning/phases/6/UAT.md`, phase 6). The README follows the same rule in its one Status sentence, which phase 10's Task 4 rewrites.
