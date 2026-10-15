# Roadmap: Terminus

## Overview

The archive is the product, so it is built first and everything else is built on top of it: a store that can hold a session losslessly and prove it, then an ingest that survives a real transcript tree, then a query path that finds a turn from the terminal and from the model. Only once recall works is the thing wired into Claude Code, because hooks and install are worth nothing until there is something to recall. Injection follows, then the feedback loop that measures whether injection helped — deliberately after injection exists, since it labels injection's own decisions. Observations and their egress boundary come next as the one opt-in, one network-touching feature, and lifecycle management lands last because retention is meaningless until there is data worth aging out. Each phase leaves something exercisable: a store you can verify, an ingest you can run, a search you can type, hooks that fire, a brief that appears, stats that prove precision.

## Phases

- [x] **Phase 1: No Model Calls** - the summary half of observations goes, so Terminus never calls a model, handles no API key and opens no network connection
- [x] **Phase 2: Linux and macOS Only, No npm** - the npm package and every Windows code path go, and a Windows build fails at compile time
- [x] **Phase 3: Install Asks Nothing** - `terminus install` applies its defaults without a question, registers through `claude mcp`, and ends with a receipt
- [x] **Phase 4: One-Command Installer** - `install.sh` downloads, checks and stores a version behind the stable path, then runs `terminus install`
- [x] **Phase 5: Install from GitHub Releases** - `install.sh` downloads from the GitHub releases by default, so the whole install is one curl line piped into `sh`
- [x] **Phase 6: Signed Install** - `install.sh` verifies each release's signature against a pinned key before it reads the manifest
- [x] **Phase 7: Blocks as rows** - each session's archive is stored as numbered blocks, one row each, so an append writes only the tail block
- [x] **Phase 8: Release Builds** - the release machinery: a tag builds four platforms in CI into a draft, and the owner's script signs and publishes it
- [x] **Phase 9: README and User Docs** - a README, user docs and a trust page for someone who has never heard of Terminus, leading with the signed release
- [ ] **Phase 10: Publish 0.1.0** - phases 1 to 9 land on `main`, `v0.1.0` is tagged, signed and published, and it installs on every platform
- [ ] **Phase 11: How Terminus Works** - design docs with diagrams of the system as it stands, in the shape of Baley's `docs/design/`

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

### Phase 5: Install from GitHub Releases
**Goal:** `install.sh` downloads from the GitHub releases on crenshawdev/terminus by default, so the whole install is one curl line piped into `sh`. A GitHub release holds no folders, so a release carries two flat files per platform: `terminus-<os>-<arch>`, and `terminus-<os>-<arch>.manifest` with the same two lines as today. Only the argument handling and the two downloads change. The checksum check, the version folders and the link stay as Phase 4 built them. Publishing a release and signing stay out. Before the README, so the README describes the finished script.
**Depends on:** Phase 4
**Requirements:** INST-13
**Success Criteria:**
1. With no argument, `install.sh` fetches `<default>/terminus-<os>-<arch>.manifest` and `<default>/terminus-<os>-<arch>`, where `<default>` is `https://github.com/crenshawdev/terminus/releases/latest/download`.
2. One `https://` argument replaces the default source, and the same two file names are fetched under it. A non-https argument, or more than one argument, is refused.
3. `cat install.sh | sh` installs with no argument, and `cat install.sh | sh -s -- https://<source>` passes a source.
4. A SHA-256 mismatch is still refused before anything is placed, and the version folders and the link behave as they did in Phase 4.
5. The Phase 4 tests pass against the flat names, `sh -n install.sh` passes, and the full suite passes.

### Phase 6: Signed Install
**Goal:** `install.sh` refuses a release it cannot verify. Each release carries `terminus-<os>-<arch>.manifest.sig`, an OpenSSH signature (SSHSIG, Ed25519) over the manifest, made with the crenshawdev release key under the namespace `terminus-release`. The script pins the key's public line, downloads the signature with the manifest, and runs `ssh-keygen -Y verify` before it reads the manifest. The SHA-256 check then runs as before. The key is never fetched. Baley pins the same key under `baley-release` (Baley #14). An update is a rerun of the same command, so it gets the same check.
**Depends on:** Phase 5
**Requirements:** INST-14
**Success Criteria:**
1. `install.sh` pins `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDRKdhQ8g59SckbR4oYy0JQlObLTSwfVjGa9UqwoPo4q` and downloads `terminus-<os>-<arch>.manifest.sig` from the same source as the manifest.
2. A signature that is missing, made by another key, or made under another namespace is refused before the manifest is read, and the stable path and the versions folder are unchanged.
3. Without `ssh-keygen` on `PATH`, the run is refused with a message naming `openssh-client`.
4. A correctly signed release installs as in Phase 5, under `/bin/sh` and dash.
5. No argument, setting or environment variable changes the pinned key.

### Phase 7: Blocks as rows
**Goal:** Each session's archive is stored as numbered blocks, one row per block (`session_no`, `block_no`, bytes, hash), in place of one whole-session blob, with the session checksum a hash over the block hashes. Today `blob::append` reads and re-hashes the whole session stream (`blob/writer.rs:99`) and the upsert writes the whole blob back (`ingest/mod.rs:466`), so every append costs I/O in the session's full size, and one excerpt loads the whole compressed session from the database, though it decompresses only the blocks its turn overlaps (`recall/excerpt.rs:98-118`). With blocks, an append touches only the tail block, a read fetches only its blocks, and corruption is pinned to one block. No release ships until this phase is done. There is no conversion tool: an existing store is not converted, and the owner's store is rebuilt by a clean install. One can come later if it is needed.
**Depends on:** Phase 6
**Requirements:** STOR-08, STOR-09, STOR-10
**Success Criteria:**
1. An append writes the session's new blocks and rewrites at most its last block, whatever the session's size.
2. An excerpt reads only the blocks that hold its turn.
3. `terminus verify` checks every block against its hash and names the session and block number of a mismatch.
4. A clean install on the owner's machine ingests every session still on disk, and `terminus verify` then reports no mismatch.

### Phase 8: Release Builds
**Goal:** The machinery that produces a GitHub release Phase 6's `install.sh` accepts. A GitHub Actions workflow builds `terminus-<os>-<arch>` for Linux and macOS on x86_64 and aarch64, writes each manifest, and attaches both to a draft release. CI never holds the key. A script on the owner's machine downloads the four manifests, signs each with `ssh-keygen -Y sign -n terminus-release`, uploads the `.sig` files and publishes the release. The Linux binaries are static musl builds with mimalloc. Running it for real is Phase 10, after the README.
**Depends on:** Phase 7
**Requirements:** REL-01
**Success Criteria:**
1. The release workflow builds the four platforms on a `v<X.Y.Z>` tag and makes one draft only when all four pass at the tag's version, and the check workflow installs a published release on the four runners.
2. The signing script publishes only when all four signatures verify against the pinned key, and refuses a release that is not a draft.
3. No workflow names a key or a signing secret.
4. The musl build is static, uses mimalloc, and keeps every hook event under its 10 ms p99 wall on the owner's corpus.

### Phase 9: README and User Docs
**Goal:** The repository is public and has no README at its root. The only user-facing one is `npm/terminus/README.md`, which Phase 2 deletes; git keeps it to draw from. Write `README.md` for someone who has never heard of Terminus: the reader's problem and what Terminus does about it first, then how it works, how to install it, every command and setting, and what it refuses to do. The README leans into the security and the signed release. Beside it, user docs cover installing, every command and every setting in full, and a trust page explains how a release is built and signed, what `install.sh` checks before it installs anything, and how to verify a release by hand. It lands before the release (Phase 10), and describes the installer from Phases 4 to 6, the release machinery from Phase 8 and the platform support from Phase 2. The owner's README brief is in `.planning/phases/9/BRIEF.md`, word for word, and its differences from DOC-01 are settled at context. The owner reviews each file in RustRover before it stands.
**Depends on:** Phase 8
**Requirements:** DOC-01, DOC-02, DOC-03
**Success Criteria:**
1. The README opens on the reader's problem and what Terminus does about it, before any mechanism.
2. The README makes the security model prominent: every release is signed on the owner's machine and never in CI, `install.sh` checks the signature against a pinned key before it installs anything, and the binary opens no network connection.
3. The install section is the one installer command, says it checks the release's signature, and that rerunning it is how Terminus updates, plus how to install a local build, and says Linux and macOS only.
4. Every command is listed with one line on what it does, and every `terminus.toml` setting with its key and default; the user docs cover each in full.
5. It states what Terminus refuses to do: no daemon, no telemetry, no model calls, no API keys and no network use at all.
6. The trust page shows how to check a downloaded release by hand with `ssh-keygen -Y verify` against the pinned key.
7. The license is MIT: `LICENSE`, the `license` field in `Cargo.toml` and the README all say so, before Phase 10 publishes anything.
8. `/cad-docs-verify` reports no stale claim: every command, path and key the README and the user docs name matches the code.

### Phase 10: Publish 0.1.0
**Goal:** Terminus 0.1.0 is published. Phases 1 to 9 land on `main`, `v0.1.0` is tagged there, the release workflow from Phase 8 makes the draft, and the owner signs and publishes it with the signing script. The check installs it on all four runners, the owner removes his local install entirely and installs the release from nothing, and then the docs stop saying no release is published. A published release cannot be taken back, so every step before publishing stays reversible. Moved out of Phase 8 so the README is in place before anyone can install.
**Depends on:** Phase 9
**Requirements:** REL-02
**Success Criteria:**
1. Pushing `v0.1.0` on `main` builds all four platforms and leaves a draft release holding each binary and its manifest, at the workspace `Cargo.toml` version; the Linux binaries are static and the macOS binaries link only `/usr/lib/` libraries.
2. After the owner runs the signing script, the release is published as latest with twelve files: four binaries, four manifests and four signatures.
3. After publishing, a workflow run on each of the four runners installs the release with the one curl line and runs `terminus --version`. This is the first time Terminus runs on macOS.
4. On the owner's machine, a fresh install after `terminus uninstall --purge` installs 0.1.0, registers the MCP server, and rebuilds the archive with `terminus verify` clean.
5. None of the README, `install.sh`, `DESIGN-BRIEF.md`, `CHANGELOG.md` or `.planning/PROJECT.md` says no release is published, and the CHANGELOG has a 0.1.0 section.

### Phase 11: How Terminus Works
**Goal:** How Terminus works, with pictures, for an engineer who wants to see inside it. Design docs in the shape of Baley's `docs/design/`: an overview first, with one diagram of the whole system from the hook to ingest to the archive to search to recall, then one doc per part, each describing the system as it stands now and never as a log of changes. Diagrams are Mermaid in Markdown, so GitHub renders them with no other tool. The owner reviews each file in RustRover before it stands.
**Depends on:** Phase 10
**Requirements:** DOC-04
**Success Criteria:**
1. An overview doc opens with one diagram of the whole system, and links to a doc for each part.
2. Each part has its own doc with at least one diagram: the hooks, ingest, the archive and its block rows, search, recall through the MCP server, injection, and install and release.
3. Every diagram is Mermaid in Markdown, renders on GitHub, and stays readable in GitHub's dark theme.
4. `/cad-docs-verify` reports no stale claim: every path, command, table and key the docs name matches the code.
