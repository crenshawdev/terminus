# Roadmap: Terminus

## Overview

The archive is the product, so it is built first and everything else is built on top of it: a store that can hold a session losslessly and prove it, then an ingest that survives a real transcript tree, then a query path that finds a turn from the terminal and from the model. Only once recall works is the thing wired into Claude Code, because hooks and install are worth nothing until there is something to recall. Injection follows, then the feedback loop that measures whether injection helped — deliberately after injection exists, since it labels injection's own decisions. Observations and their egress boundary come next as the one opt-in, one network-touching feature, and lifecycle management lands last because retention is meaningless until there is data worth aging out. Each phase leaves something exercisable: a store you can verify, an ingest you can run, a search you can type, hooks that fire, a brief that appears, stats that prove precision.

## Phases

- [x] **Phase 1: No Model Calls** - the summary half of observations goes, so Terminus never calls a model, handles no API key and opens no network connection
- [ ] **Phase 2: Linux and macOS Only, No npm** - the npm package and every Windows code path go, and a Windows build fails at compile time
- [ ] **Phase 3: Install Asks Nothing** - `terminus install` applies its defaults without a question, registers through `claude mcp`, and ends with a receipt
- [ ] **Phase 4: One-Command Installer** - `install.sh` downloads, checks and stores a version behind the stable path, then runs `terminus install`
- [ ] **Phase 5: README** - a README at the repo root for someone who has never heard of Terminus

## Phase Details

### Phase 1: No Model Calls
**Goal:** Terminus never calls a model and never handles an API key, matching Baley's ADR 0039 rule that the binary never calls a model. The summary half of observations goes, with everything that exists only for it: `observe/judgment.rs`, `provider.rs`, `net.rs` (the only network code), `cost.rs`, `credentials.rs`, the `[provider]` table in `terminus.toml`, the `ureq` dependency, and `terminus doctor`'s key and config-permission checks. The mechanical half stays, and so do the redaction rules in `egress.rs`, which `redact_recall` uses. What happens to the summaries already stored is settled at context. First because it deletes code Phase 2 would otherwise edit.
**Depends on:** Nothing (first phase of this cycle)
**Requirements:** PRIV-05
**Success Criteria:**
1. Neither crate depends on an HTTP client (`cargo tree` shows no `ureq`, `reqwest` or `hyper`), and no source file outside the tests opens a socket.
2. Nothing reads an API key: no `_API_KEY` lookup, and nothing reads `credentials.toml` or an `api_key` setting.
3. A `terminus.toml` that still has a `[provider]` table fails to load with a message saying model calls were removed, naming the file and none of its values.
4. Every finished session still gets its mechanical observation, which `terminus observations` lists, and its files, commands and errors stay searchable.
5. With `redact_recall` on, recall output is still filtered by the same rules.
6. `terminus doctor` no longer carries the key, config-permission and provider checks, and the full suite passes.

### Phase 2: Linux and macOS Only, No npm
**Goal:** Remove `npm/` (the package, its five platform packages and `pack-local.sh`) and its references in `.gitignore`, `DESIGN-BRIEF.md`, `install/binary.rs` and the v2 Distribution note in REQUIREMENTS.md, and remove every Windows branch. Counting `cfg(windows)`, `cfg!(windows)`, `target_os = "windows"`, `cfg(unix)` and `cfg(not(windows))` guards and `LOCALAPPDATA`, that was 40 lines in 12 source files and 50 in 18 test files, measured 2026-10-12 before Phase 1 deleted some of those files. Guards that existed only to exclude Windows go, and their code becomes unconditional. Before the install rework, so Phase 3 edits code with one platform shape.
**Depends on:** Nothing
**Requirements:** INST-09
**Success Criteria:**
1. `npm/` is gone, and nothing mentions npm outside `CHANGELOG.md`, the shipped requirement rows and the archived planning files.
2. No `cfg(windows)`, `cfg!(windows)`, `target_os = "windows"` or `LOCALAPPDATA` remains in `crates/`.
3. A build for Windows fails at compile time with a message naming Linux and macOS.
4. The full suite passes, and every test removed because it only covered Windows is listed in the phase summary.
5. The docs say Linux and macOS, `terminus doctor` prints no Windows fix, and the deferred Windows ACL check (D-15) is closed as no longer applicable.

### Phase 3: Install Asks Nothing
**Goal:** `terminus install` applies its defaults and asks nothing, as `baley install` does. It drops the diff-and-confirm step and `--yes`, registers the MCP server by running `claude mcp` so Claude Code stays the only writer of `~/.claude.json`, and ends with a receipt. Uninstall keeps its confirmation and `--purge`, and removes the registration with `claude mcp remove`.
**Depends on:** Phase 2
**Requirements:** INST-10, INST-11
**Success Criteria:**
1. With stdin closed, `terminus install` completes with the same result an interactive run has, prints no question, and rejects `--yes` as an unknown flag.
2. No Terminus write touches `~/.claude.json`. The entry is placed with `claude mcp add-json --scope user`, and a rerun with the entry already correct runs no `claude` command.
3. When `claude` is missing or fails, install prints the exact command to run and reports the install incomplete rather than done.
4. The hook entries in `~/.claude/settings.json` are still merged in place with a backup, and a rerun changes nothing.
5. The receipt lists every change, the backfill estimate and where to watch it, and the current `cleanupPeriodDays`, with the fact that transcripts older than it are already gone and the line that raises it. Install never changes `cleanupPeriodDays` or `autoCompactEnabled`.
6. Uninstall removes the registration with `claude mcp remove`, and its confirmation and `--purge` behaviour are unchanged.

### Phase 4: One-Command Installer
**Goal:** An `install.sh` with the shape of Baley's: given a source URL, it downloads the manifest and binary for the machine, checks the SHA-256 in a `verify_download` step that Baley's release design (#14) later turns into a signature check, stores the binary by version behind `~/.local/bin/terminus`, and runs `terminus install`. Terminus has no releases yet, so the phase is qualified with unsigned development artifacts only, as Baley's T14 and T17 were. How a local build gets installed is settled at context.
**Depends on:** Phase 3
**Requirements:** INST-12
**Success Criteria:**
1. `sh install.sh https://<source>` fetches `<source>/<os>-<arch>/manifest` and the binary, refuses a SHA-256 mismatch before anything is placed, and refuses any system other than Linux or Darwin on x86_64 or aarch64.
2. Each version lands in its own folder and `~/.local/bin/terminus` points at the current one. Installing a newer version leaves the old one on disk, so a running session's MCP server keeps working.
3. The script ends by running `~/.local/bin/terminus install`, and the whole install is that one command.
4. A stable path held by something other than a Terminus install is refused, as `terminus install` refuses it today (D-07).
5. A local build can still be installed without publishing anything.
6. Qualified on Linux. macOS stays unverified until it is run on a Mac.

### Phase 5: README
**Goal:** The repository is public and has no README at its root. The only user-facing one is `npm/terminus/README.md`, which Phase 2 deletes; git keeps it to draw from. Write `README.md` for someone who has never heard of Terminus: the reader's problem and what Terminus does about it first, then how it works, how to install it, every command and setting, and what it refuses to do. Last, because it describes the installer from Phase 4 and the platform support from Phase 2.
**Depends on:** Phase 4
**Requirements:** DOC-01
**Success Criteria:**
1. The README opens on the reader's problem and what Terminus does about it, before any mechanism.
2. The install section is the one installer command, plus how to install a local build, and says Linux and macOS only.
3. Every command is listed with one line on what it does, and every `terminus.toml` setting with its key and default.
4. It states what Terminus refuses to do: no daemon, no telemetry, no model calls, no API keys and no network use at all.
5. `/cad-docs-verify` reports no stale claim: every command, path and key the README names matches the code.
