---
phase: 9
plan: 2
requirements: [DOC-02, DOC-03]
files:
  - docs/release.md
  - docs/configuration.md
  - docs/commands.md
  - docs/install.md
  - docs/json-shapes.md
---

# Phase 9: README and User Docs - Plan 2

## Goal

The phase goal: the public repository gets a README for someone who has never heard of Terminus, opening on the reader's problem and leading with the security model and the signed release, with user docs beside it for installing, every command, every setting and how a release is built, signed and checked.

This plan writes the user docs the README links to: a release page, a configuration page, a commands page and an install page under `docs/`, each the full detail the README gives in a line.

## Must be true when done

- **The release page shows the pinned key and the by-hand check works (AC5).** `docs/release.md` holds `install.sh`'s `release_key` exactly. Its `ssh-keygen -Y verify` command, run with a throwaway key's allowed-signers line against a manifest that key signed under `-n terminus-release`, exits 0, and exits non-zero once one byte of the manifest changes. The page lists what the signature protects and what it does not, and it is not titled "Trust".
- **Every setting is documented in full (AC4).** `docs/configuration.md` gives every `terminus.toml` key with its default and its effect, where the file is read from, and the environment variables and Claude Code settings Terminus reads.
- **Every command is documented in full (AC4).** `docs/commands.md` has every command in `main.rs`'s `USAGE`, each with every option its parser accepts, except `backfill --work`.
- **Installing, updating and uninstalling are documented (AC3).** `docs/install.md` gives the one-line install, the `sh -s -- https://<source>` form, a local build, updating by rerunning the line, and uninstalling, naming what `terminus uninstall` and `--purge` leave behind.
- **The docs name nothing the code lacks (AC7, human-verify).** `/cad-docs-verify docs/` reports no stale claim in the four pages, and every relative link in them resolves.
- **Each page has had its review stop (D-16).** The owner has reviewed each of the four pages in RustRover.

## Context

- Plan 2 of 4. It shares no file with plan 1, and the two can run in either order. Plans 3 and 4 write the README, which links these pages, so they run after this plan.
- D-02, D-10, D-16 to D-20 and D-22 to D-24 in `.planning/phases/9/CONTEXT.md` bind this plan, and D-21 binds its install page.
- **The code is the source.** Read every fact from the file that implements it: `install.sh`, `scripts/sign-release.sh`, `.github/workflows/release.yml`, `crates/terminus/src/main.rs`, each command's file under `crates/terminus/src/cmd/`, `crates/terminus-core/src/config.rs` and `crates/terminus-core/src/store/open.rs`. `DESIGN-BRIEF.md` is a map only; its §14 is wrong about the CLI (D-18), so no command or option comes from it. A line number in CONTEXT's evidence may have moved, so find the code by its symbol.
- **No release status.** No page says whether a release is published, and none says the macOS builds have never run. Phase 10's Task 4 corrects only the README, `install.sh`, `DESIGN-BRIEF.md`, the CHANGELOG and `PROJECT.md`, so a sentence like that here would go stale once 0.1.0 is out. The README's Status line is the one place that says it (D-07).
- **Writing.** CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing doc prose or a commit message, and its README and project docs rules apply. Name no other tool. State each limit, failure and refusal with the condition that triggers it. Link `docs/json-shapes.md` for `--json` output and the exit codes rather than repeat them (D-17).
- **Order.** The pages are written so that each links only pages that already exist: the release page, then configuration, then commands, then install. Each page is opened for the owner in RustRover with `mcp__rustrover__open_file_in_editor` when written (memory `open-docs-in-rustrover`). Commits are signed, stay on `cadence/v0.0.1`, and no PR is opened (D-15).
- Out of scope: the README, `docs/json-shapes.md` and `docs/design/`, and changing any code to match the docs.

## Tasks

### Task 1: Write the release page

- **Files:** docs/release.md
- **Action:**
  - **A separate page, not "Trust" (D-02).** The page sits under `docs/` so `/cad-docs-verify` scans it by default, and its title does not contain the word Trust, so it is never confused with the shared `TRUST.md`. The file name is `docs/release.md`, chosen under D-17's rule that names carry no phase numbers.
  - **How a release is built in CI (DOC-03).** From `.github/workflows/release.yml`: a pushed `v<X.Y.Z>` tag builds `terminus-<os>-<arch>` for Linux and macOS on x86_64 and aarch64, writes each manifest, and makes one draft only when all four builds pass. The build jobs have `contents: read`, the draft job uses only the workflow's own token, and no workflow names a key or a signing secret. The manifest is exactly two lines, `version X.Y.Z` and `sha256 <64 lowercase hex>`.
  - **How it is signed on the owner's machine.** From `scripts/sign-release.sh`: it refuses a release that is not a draft, checks each binary against its manifest before signing anything, signs each manifest with the `crenshawdev-release` key under the namespace `terminus-release`, verifies each signature the way `install.sh` does, and uploads the `.sig` files and publishes the release only when all four pass.
  - **What `install.sh` checks before it installs anything.** Every check in the order the script makes it: the source argument, `TERMINUS_BIN_DIR`, `HOME`, what sits at `~/.local/bin/terminus`, the OS and architecture, `ssh-keygen` being present, the manifest and signature downloads, the signature against the pinned key, then the binary download, the manifest's two lines, and the binary's SHA-256. Give each refusal with the condition that triggers it. The key is never fetched, and no argument, setting or environment variable changes it.
  - **The pinned key.** Show the key exactly as `install.sh`'s `release_key` holds it, in full, with its `crenshawdev-release` identity and the `terminus-release` namespace.
  - **What the binary never does (DOC-03).** It opens no network connection, calls no model, and reads no API key; a `[provider]` table in `terminus.toml` fails the load. `install.sh`'s `curl` downloads are the one network use. Back each with its code: `crates/terminus-core/tests/no_network.rs`, `crates/terminus-core/src/observe/egress.rs` and `crates/terminus-core/src/config.rs`.
  - **What the signature protects and what it does not (D-23).** It protects each manifest, and through the manifest's SHA-256 the binary, signed by `crenshawdev-release` under `terminus-release`; CI holds no key; only a draft is signed; and the signer checks each binary against its manifest first. It does not protect `install.sh` itself, which is fetched unsigned over HTTPS from `main`; an older, validly signed release served from a source the user passes, since there is no version floor; key loss or rotation, since there is no revocation and a new key means a new `install.sh`; or proof that the binary was built from the tagged source. Give both lists the same prominence.
  - **Checking a release by hand (D-24).** Repeat `install.sh`'s own command. The reader downloads `terminus-<os>-<arch>`, its `.manifest` and its `.manifest.sig` from the release, writes an allowed-signers file holding the one line `crenshawdev-release <key>` with the full key, and runs `ssh-keygen -Y verify -f <file> -I crenshawdev-release -n terminus-release -s terminus-<os>-<arch>.manifest.sig < terminus-<os>-<arch>.manifest`. Then they compare the binary's SHA-256 from `sha256sum`, or `shasum -a 256` on macOS, with the hex on the manifest's second line. Say what a pass prints and that any other result means the file is not the one the key signed.
  - **No OpenSSH minimum.** State no OpenSSH version, because which version first has `-Y verify` is an open question CONTEXT left flagged.
  - **Review stop (D-16).** Open `docs/release.md` in RustRover for the owner.
- **Verify:**
  - `grep -cF "$(sed -n "s/^release_key='\(.*\)'\$/\1/p" install.sh)" docs/release.md` prints at least 1.
  - `head -n 1 docs/release.md | grep -ci trust` prints 0.
  - The by-hand check, in a new empty directory made with `mktemp -d`: `ssh-keygen -t ed25519 -N '' -f key` makes a throwaway key; a file `terminus-linux-x86_64.manifest` holds `version 0.1.0` and `sha256 ` followed by 64 `a` characters, each line ending in a newline; `ssh-keygen -Y sign -f key -n terminus-release < terminus-linux-x86_64.manifest > terminus-linux-x86_64.manifest.sig` signs it; and an allowed-signers file holds `crenshawdev-release ` followed by the contents of `key.pub`. The `ssh-keygen -Y verify` command copied from `docs/release.md`, with only `<file>`, `<os>` and `<arch>` filled in, exits 0. After one byte of the manifest changes, the same command exits non-zero.
  - `grep -n 'sha256sum' docs/release.md` and `grep -n 'shasum -a 256' docs/release.md` each find a line.
  - `grep -niE 'contents: read|revocation|version floor|tagged source' docs/release.md` finds the protected and unprotected items, and `grep -niE 'no release|not (yet )?published|first release|never (been )?run' docs/release.md` prints nothing.
  - Every relative link in the page resolves: each `](target)` that is not `http` or `https`, with any `#anchor` removed, names a path that exists relative to `docs/`.
  - human-verify: the owner reviews `docs/release.md` in RustRover and accepts it.

### Task 2: Write the configuration page

- **Files:** docs/configuration.md
- **Action:**
  - **Every `terminus.toml` key with its default and its effect (D-19).** From `crates/terminus-core/src/config.rs` (`FileConfig` and the tables under it, the `DEFAULT_` constants, `RetentionAction`, `CaptureMode`): `roots`, default `$CLAUDE_CONFIG_DIR`, otherwise `~/.claude`; `exclude`, default none; `[injection] brief_chars` 6000 and `prompt_chars` 4000, each capped at 10000; `[retention] action` `keep`, with an unknown value meaning `keep`, and `age_days` 0, which is off, with a negative value clamped to 0; `[retention.project."<path>"]` with the same two keys, overriding the global rule for that project; `[snapshot] enabled` `true`, `interval_hours` 24, with 0 falling back to 24, and `keep` 3, with a minimum of 1; `[capture] mode` `full`, also `lean` or `minimal`, with an unknown value meaning `full`; `[privacy] redact_recall` `false`. Read each value's other accepted values and each effect from the code that applies it, including `crates/terminus-core/src/inject/brief.rs` and `crates/terminus-core/src/inject/prompt.rs` for the budgets, and write the effect as what the reader sees change.
  - **The file itself (D-19).** It is read from `$TERMINUS_CONFIG_DIR`, otherwise `$XDG_CONFIG_HOME/terminus`, otherwise `~/.config/terminus`, the same on Linux and macOS. Terminus never writes it, ignores unknown keys, and refuses a `[provider]` table, failing the load with the error `config.rs` gives.
  - **Paths outside `terminus.toml` (D-20).** The data directory, in the order `crates/terminus-core/src/store/open.rs`'s `data_dir` resolves it: `TERMINUS_DATA_DIR`, then the location `terminus data move` records, then the platform data directory (`$XDG_DATA_HOME/terminus` or `~/.local/share/terminus` on Linux, `~/Library/Application Support/terminus` on macOS). `TERMINUS_CONFIG_DIR` and `CLAUDE_CONFIG_DIR`. The two Claude Code settings Terminus reports and never changes, `cleanupPeriodDays` and `autoCompactEnabled`, as `crates/terminus/src/cmd/doctor.rs` reports them. `TERMINUS_BIN_DIR` appears only as something `install.sh` refuses, and no other `TERMINUS_` variable appears, because the rest are test hooks or internal hand-offs.
  - **Links.** Link `docs/release.md` where the page mentions `install.sh`'s refusal.
  - **Review stop (D-16).** Open `docs/configuration.md` in RustRover for the owner.
- **Verify:**
  - For each of `roots`, `exclude`, `[injection]`, `brief_chars`, `prompt_chars`, `[retention]`, `action`, `age_days`, `[retention.project.`, `[snapshot]`, `enabled`, `interval_hours`, `keep`, `[capture]`, `mode`, `full`, `lean`, `minimal`, `[privacy]` and `redact_recall`, `grep -qF -- <term> docs/configuration.md` succeeds.
  - `grep -nE '6,?000' docs/configuration.md`, `grep -nE '4,?000' docs/configuration.md` and `grep -nE '10,?000' docs/configuration.md` each find a line.
  - `grep -oE 'TERMINUS_[A-Z_]+' docs/configuration.md | sort -u` prints exactly `TERMINUS_BIN_DIR`, `TERMINUS_CONFIG_DIR` and `TERMINUS_DATA_DIR`.
  - `grep -n 'XDG_CONFIG_HOME\|XDG_DATA_HOME\|Application Support\|CLAUDE_CONFIG_DIR\|cleanupPeriodDays\|autoCompactEnabled\|\[provider\]\|data move' docs/configuration.md` finds each term.
  - Every relative link in the page resolves, checked as in Task 1, and `grep -niE 'no release|not (yet )?published|first release|never (been )?run' docs/configuration.md` prints nothing.
  - human-verify: the owner reviews `docs/configuration.md` in RustRover and accepts it.

### Task 3: Write the commands page

- **Files:** docs/commands.md
- **Action:**
  - **Commands from the code (D-18).** The list is `USAGE` in `crates/terminus/src/main.rs`, and each command's options are what its `parse` function, or `json_flag` in `crates/terminus/src/cmd/mod.rs`, accepts. Every command appears: `ingest` and `ingest <path.jsonl>`, `backfill`, `hook <event>`, `search <words>`, `show <id>...`, `sessions`, `verify`, `reindex`, `status`, `compact`, `usage`, `export <dir>`, `stats`, `observations` and `observations regenerate`, `replay`, `retention --dry-run`, `doctor`, `mcp`, `install`, `uninstall` and `data move <path>`, plus the top-level `--version`/`-V` and `--help`/`-h`. Each gets what it does and every option and argument its parser takes, with each option's value form and meaning read from its file.
  - **Run by Claude Code (D-18).** `hook <event>`, whose events are `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact` (`crates/terminus/src/cmd/hook.rs`), and `mcp` are listed as run by Claude Code and not typed. Name the three tools `mcp` serves, `recall_search`, `recall_context` and `recall_get`, from `crates/terminus/src/cmd/mcp/tools.rs`.
  - **`retention --dry-run` (D-18).** Documented as accepted and changing nothing, which is what `crates/terminus/src/cmd/retention.rs`'s parser does.
  - **Left out (D-18).** `backfill --work` does not appear, as `USAGE` leaves it out, because it is the internal hand-off a hook spawns.
  - **`--json`.** Say which commands accept `--json`, as their parsers show, and link `json-shapes.md` for the shapes and the exit codes rather than repeat them (D-17).
  - **Links.** Link `docs/configuration.md` where a command's behavior depends on a setting.
  - **Review stop (D-16).** Open `docs/commands.md` in RustRover for the owner.
- **Verify:**
  - Every long option a parser accepts is on the page: for each option printed by `grep -rhoE 'Long\("[a-z-]+"\)' crates/terminus/src | sed -E 's/Long\("([a-z-]+)"\)/--\1/' | sort -u`, plus `--json`, `grep -qF -- <option> docs/commands.md` succeeds. For each of `-V`, `-h` and `-y`, `grep -qw -- <form> docs/commands.md` succeeds; `-w` keeps `-h` from matching inside `--help`.
  - `grep -c -- '--work' docs/commands.md` prints 0.
  - For each of `ingest`, `backfill`, `hook`, `search`, `show`, `sessions`, `verify`, `reindex`, `status`, `compact`, `usage`, `export`, `stats`, `observations`, `observations regenerate`, `replay`, `retention`, `doctor`, `mcp`, `install`, `uninstall` and `data move`, `grep -qF "terminus <command>" docs/commands.md` succeeds.
  - For each of `SessionStart`, `UserPromptSubmit`, `SessionEnd`, `PostCompact`, `recall_search`, `recall_context` and `recall_get`, `grep -qF` finds it in `docs/commands.md`, and `grep -n 'json-shapes.md' docs/commands.md` finds the link.
  - Every relative link in the page resolves, checked as in Task 1, and `grep -niE 'no release|not (yet )?published|first release|never (been )?run' docs/commands.md` prints nothing.
  - human-verify: the owner reviews `docs/commands.md` in RustRover and accepts it.

### Task 4: Write the install page

- **Files:** docs/install.md
- **Action:**
  - **Installing (D-17, D-21).** The one line, `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`, and what it does: it checks the release's signature against the pinned key before it reads the manifest or downloads the binary (link `docs/release.md` for the checks), stores the binary under `~/.local/lib/crenshawdev/terminus/versions/<version>/`, links `~/.local/bin/terminus` to it, and runs `terminus install`. The form ending `sh -s -- https://<source>` names another source, and the same three file names are fetched under it. This form is in the user docs only.
  - **What `terminus install` writes.** From `crates/terminus/src/cmd/install/`: the hook entries in Claude Code's `settings.json`, the MCP registration made through `claude mcp add-json`, the folder `CLAUDE_CONFIG_DIR` moves them to, and the receipt it prints. Give the conditions under which `claude` runs and the install exits 1 exactly as `register` in `install/mod.rs` decides them.
  - **What it needs (D-10).** Linux or macOS on x86_64 or aarch64, `curl`, `ssh-keygen`, `sha256sum` or `shasum`, and `claude` on `PATH`. `git` is optional; without it, project identity degrades as `crates/terminus-core/src/project.rs` describes. State no minimum Claude Code version and no OpenSSH version, because `doctor` calls the 2.1.227 floor unproven and the OpenSSH question is left flagged. Name the `ssh-keygen` packages `install.sh` names.
  - **A local build (D-21).** `cargo build --release && target/release/terminus install`, which stores the running binary under its version and moves the link, as `crates/terminus/src/cmd/install/binary.rs` describes.
  - **Updating.** Rerunning the one line installs the newer version beside the old one and moves the link. Old version folders are kept, a session already running keeps the file it started from, and the hooks and the MCP entry are not rewritten.
  - **Uninstalling (D-22).** `terminus uninstall` removes the hook entries, the MCP entry and the link, keeps the versions folder and the data directory, and prints both paths. `--purge` deletes the data directory after showing its path and size and asking once, and `-y`/`--yes` behaves as `uninstall.rs`'s parser says. Removing everything by hand means deleting the config directory and `~/.local/lib/crenshawdev/terminus`, never the whole `crenshawdev` folder, because Baley uses it too.
  - **Links.** Link `docs/release.md`, `docs/configuration.md` for the data and config paths, and `docs/commands.md`.
  - **Review stop (D-16).** Open `docs/install.md` in RustRover for the owner.
- **Verify:**
  - `grep -cF 'curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh' docs/install.md` prints at least 1, and `grep -cF 'sh -s -- https://' docs/install.md` prints at least 1.
  - `grep -cF 'cargo build --release && target/release/terminus install' docs/install.md` prints at least 1.
  - `grep -n 'terminus uninstall\|--purge\|~/.local/lib/crenshawdev/terminus' docs/install.md` finds each term, and `grep -n 'crenshawdev' docs/install.md` shows no line telling the reader to delete the `crenshawdev` folder itself.
  - For each of `curl`, `ssh-keygen`, `sha256sum`, `shasum`, `claude` and `git`, `grep -qw` finds it in `docs/install.md`.
  - Every relative link in the page resolves, checked as in Task 1, and `grep -niE 'no release|not (yet )?published|first release|never (been )?run' docs/install.md` prints nothing.
  - `grep -niE 'claude-mem|codex|cursor|copilot|windsurf' docs/release.md docs/configuration.md docs/commands.md docs/install.md` prints nothing.
  - human-verify (AC7): the owner runs `/cad-docs-verify docs/`, and it reports no stale claim in the four pages this plan wrote. A stale claim in one of them is fixed in this task. A stale claim in `docs/json-shapes.md`, which no plan in this phase writes, is reported to the owner for a ruling and not edited.
  - human-verify: the owner reviews `docs/install.md` in RustRover and accepts it.

## Notes

- **Why `/cad-docs-verify` runs here for `docs/`.** AC7 asks for no stale claim over `README.md` and `docs/`. A finding in a docs page has to be fixed by the plan that holds that page's lease, so `docs/` is checked here and `README.md` at the end of plan 4. Together the two runs cover what AC7 names, once each.
- **Recalled prior art.** Phase 6's UAT found that the unmodified `install.sh` refuses a release signed by a throwaway key even when that key is in `$HOME/.ssh/allowed_signers` (`.planning/phases/6/UAT.md`, phase 6). Task 1's throwaway-key check runs the page's command against an allowed-signers file the reader writes, not against `install.sh`, so it tests the by-hand procedure and leaves the pinned-key refusal as it is.
