# Terminus

When a Claude Code session ends, the next one starts without it, and when a long session compacts to make room, the earlier part of it drops out of what Claude can see. The work is still written down in Claude Code's transcripts, until Claude Code deletes them after `cleanupPeriodDays`, 30 days by default.

Terminus keeps a copy of every session you run in Claude Code and lets you and Claude find the work in it again. A new session starts knowing where the last one in the project left off. A prompt that names a file or an identifier from earlier work can bring back the turns where it came up, and after a compaction, the next prompt can bring back the turns that dropped out. The whole history stays searchable, from the command line and by Claude itself.

> **Status: built, with known gaps.** Archiving, the recall tools Claude uses, the context Claude is given at the start of a session and on a prompt, and every command below work today. No release is published yet and the macOS builds have never run, so today Terminus runs from a local build on Linux. The gaps that touch what this page says are under [Current state](#current-state).

## Trust

Terminus collects nothing about you, sends no telemetry, and sends data only to services you configure. Because that guarantee depends on the operating system underneath, the same standard decides which platforms Terminus supports. Linux and macOS meet it. Windows fails it on privacy and on security and is not supported. A supported platform that stops meeting the standard loses support. The criteria and reasoning are in [TRUST.md](TRUST.md).

**What leaves your machine.** Downloads when you install or update: the install line fetches `install.sh` from `raw.githubusercontent.com`, and `install.sh` uses `curl` to fetch the manifest, its signature and the binary for your system from the GitHub releases of crenshawdev/terminus, or from the `https://` source you pass it. Archived text Terminus hands Claude Code: at the start of a session the `SessionStart` hook may hand it a brief of where the last session left off, on a prompt the `UserPromptSubmit` hook may hand it earlier turns, and the `recall_search`, `recall_context` and `recall_get` tools return archived turns when Claude calls them. Claude Code sends that text to its model with the rest of the session. `[privacy] redact_recall = true` filters it on its way into Claude Code, and `[injection] brief_chars` and `prompt_chars` cap how much each hook hands over.

Every release is signed on the maintainer's machine, never in CI. The release workflow only builds the binaries and drafts the release, and no workflow holds a key. `install.sh` checks the manifest's signature against the key pinned in the script before it reads the manifest or downloads the binary, then checks the binary against the manifest's SHA-256, and installs nothing that fails either check. The Terminus binary opens no network connection, and a test fails if any source file in either crate names a socket type. How a release is built, signed and checked, and how to check one by hand, is in [docs/release.md](docs/release.md).

## What it does

- **Archives every session.** Each hook Claude Code fires starts a pass that archives what is new in every transcript under `roots`, subagent transcripts included, reading each file from where the last pass stopped. `terminus backfill` archives the transcripts already on disk, and `terminus install` starts one.
- **Opens each session with a brief.** At `SessionStart`, Claude is told where the last session in the project left off: its date, the branch it ended on and the last thing said each way, within `[injection] brief_chars`.
- **Brings back earlier turns on a prompt.** When a prompt names paths or identifiers that an earlier turn also named, that turn can be added to the prompt, within `[injection] prompt_chars`. Most prompts get nothing. The first prompt after a compaction is offered only the turns the compaction dropped.
- **Lets Claude search its own history.** The `recall_search`, `recall_context` and `recall_get` tools find past turns by text, path, error or identifier, read the turns around one, and read a turn's full archived text.
- **Searches from the command line.** `terminus search`, `terminus show` and `terminus sessions` answer about the project the current directory is in, another with `--project <path>`, or every project with `--project '*'`.
- **Records what happened in each session.** `terminus observations` lists, for each closed session, the files modified and read, the tools, the commands, the errors and the commits, taken from the transcript's own records.
- **Checks and rebuilds itself.** `terminus verify` checks every archived block against its hash, `terminus reindex` rebuilds everything derived from the archive, and `[snapshot]` keeps rolling copies of the archive.
- **Measures its own injections.** `terminus stats` reports the hits, false positives, precision and misses of what was added to prompts, and `terminus replay` re-scores the logged prompts under other thresholds without changing anything.
- **Keeps what you tell it to.** `exclude` keeps projects out entirely, `[retention]` evicts or deletes old sessions, `[capture] mode` stores less of each record, and `[privacy] redact_recall` filters text shaped like a secret out of what Claude Code is handed. `terminus export`, `terminus data move` and `terminus compact` write the archive out, move it, and give back the space retention freed.
- **Reports its own wiring.** `terminus doctor` checks the hook entries, the MCP entry, the store and the last pass, and reports Claude Code's `cleanupPeriodDays` and `autoCompactEnabled`, which Terminus never changes.

## What it is designed to refuse

- It runs no daemon. The ingest and backfill a hook starts run detached and exit when the work is done, and the MCP server is a child Claude Code starts over stdio for each session, which exits when Claude Code closes its input.
- It sends no telemetry, analytics or crash reports.
- It calls no model. The facts it records about each session come from the transcript's own records.
- It reads no API key. A `terminus.toml` that still has a `[provider]` table fails to load until the table is deleted.
- The binary opens no network connection.
- It has no encryption at rest. Every file Terminus writes is readable only by your user, and the transcripts it copies sit unencrypted in Claude Code's own folder as well.

It also refuses, under these conditions:

- `install.sh` installs nothing and exits 1 when the manifest's signature is missing or does not verify against the pinned key, when `ssh-keygen` is not on `PATH`, when `TERMINUS_BIN_DIR` is set, when the system is not Linux or macOS on x86_64 or aarch64, or when the file at `~/.local/bin/terminus` is not a Terminus build. Every check is listed in [docs/release.md](docs/release.md#what-installsh-checks).
- A build for any operating system other than Linux or macOS fails at compile time.
- A store in an older archive format is refused before anything in it changes, with the steps for a clean install, because no conversion exists. A store in a newer archive format is refused with a message to upgrade Terminus.
- A project listed in `exclude` is never opened. Its transcript files are skipped before they are read, and anything archived from it before it was excluded is left out of every search, recall, brief and injection.
- Retention does nothing unless `[retention] action` and `age_days` are both set, and `delete` removes a session only once Claude Code has already deleted its transcript.
- Hooks fail open. Once the event name is known, a hook exits 0 whatever goes wrong, with one line on stderr, so it never blocks a prompt, and an injection that misses its deadline hands over nothing.

## Current state

- The tests do not run in CI. The release workflow builds and drafts releases, and no workflow runs the test suite ([#47](https://github.com/crenshawdev/terminus/issues/47)).
- `[privacy] redact_recall` misses a bare OpenAI-style `sk-` token in plain prose, because none of its rules recognizes that shape, so the token reaches Claude Code unfiltered ([#5](https://github.com/crenshawdev/terminus/issues/5)).
- With `[privacy] redact_recall` on, the body `recall_get` returns goes through a lossy UTF-8 round trip, so an archived line that is not valid UTF-8 comes back changed even when it holds no secret ([#1](https://github.com/crenshawdev/terminus/issues/1)).
- What the `recall_search`, `recall_context` and `recall_get` tools return is not documented, and the tool table in `DESIGN-BRIEF.md` is out of date ([#26](https://github.com/crenshawdev/terminus/issues/26)).
