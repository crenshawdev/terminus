# Terminus

When a Claude Code session ends, the next one starts without it, and when a long session compacts to make room, the earlier part of it drops out of what Claude can see. The work is still written down in Claude Code's transcripts, until Claude Code deletes them after `cleanupPeriodDays`, 30 days by default.

Terminus keeps a copy of every session you run in Claude Code and lets you and Claude find the work in it again. A new session starts knowing where the last one in the project left off. A prompt that names a file or an identifier from earlier work can bring back the turns where it came up, and after a compaction, the next prompt can bring back the turns that dropped out. The whole history stays searchable, from the command line and by Claude itself.

> **Status: built, with known gaps.** Archiving, the recall tools Claude uses, the context Claude is given at the start of a session and on a prompt, and every command below work today. 0.1.1 is published, and the one-line install below is checked on Linux and macOS, on x86_64 and aarch64, each time a release is published. The gaps that touch what this page says are under [Current state](#current-state).

## Trust

Terminus collects nothing about you, sends no telemetry, and sends data only to services you configure. Because that guarantee depends on the operating system underneath, the same standard decides which platforms Terminus supports. Linux and macOS meet it. Windows fails it on privacy and on security and is not supported. A supported platform that stops meeting the standard loses support. The criteria and reasoning are in [TRUST.md](TRUST.md).

**What leaves your machine.** Downloads when you install or update: the install line fetches `install.sh` from `raw.githubusercontent.com`, and `install.sh` uses `curl` to fetch the manifest, its signature and the binary for your system from the GitHub releases of crenshawdev/terminus, or from the `https://` source you pass it. Archived text Terminus hands Claude Code: at the start of a session the `SessionStart` hook may hand it a brief of where the last session left off, on a prompt the `UserPromptSubmit` hook may hand it earlier turns, and the `recall_search`, `recall_context` and `recall_get` tools return archived turns when Claude calls them. Claude Code sends that text to its model with the rest of the session. `[privacy] redact_recall = true` filters it on its way into Claude Code, and `[injection] brief_chars` and `prompt_chars` cap how much each hook hands over.

Every release is signed on the maintainer's machine, never in CI. The release workflow only builds the binaries and drafts the release, and no workflow holds a signing key. `install.sh` checks the manifest's signature against the key pinned in the script before it reads the manifest or downloads the binary, then checks the binary against the manifest's SHA-256, and installs nothing that fails either check. The Terminus binary opens no network connection, and a test fails if any source file in either crate names a socket type. How a release is built, signed and checked, and how to check one by hand, is in [docs/release.md](docs/release.md).

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
- It has no encryption at rest. Every file Terminus writes in its data directory is readable only by your user, and the transcripts it copies sit unencrypted in Claude Code's own folder as well.

It also refuses, under these conditions:

- `install.sh` installs nothing and exits 1 when the manifest's signature is missing or does not verify against the pinned key, when `ssh-keygen` is not on `PATH`, when `TERMINUS_BIN_DIR` is set, when the system is not Linux or macOS on x86_64 or aarch64, or when the file at `~/.local/bin/terminus` is not a Terminus build. Every check is listed in [docs/release.md](docs/release.md#what-installsh-checks).
- A build for any operating system other than Linux or macOS fails at compile time.
- A store in an older archive format is refused before anything in it changes, with the steps for a clean install, because no conversion exists. A store in a newer archive format is refused with a message to upgrade Terminus.
- A project listed in `exclude` is never opened. Its transcript files are skipped before they are read, and anything archived from it before it was excluded is left out of every search, recall, brief and injection.
- Retention does nothing unless `[retention] action` and `age_days` are both set, and `delete` removes a session only once Claude Code has already deleted its transcript.
- Hooks fail open. Once the event name is known, a hook exits 0 whatever goes wrong, with one line on stderr, so it never blocks a prompt, and an injection that misses its deadline hands over nothing.

## Current state

- `[privacy] redact_recall` misses a bare OpenAI-style `sk-` token in plain prose, because none of its rules recognizes that shape, so the token reaches Claude Code unfiltered ([#5](https://github.com/crenshawdev/terminus/issues/5)).
- What the `recall_search`, `recall_context` and `recall_get` tools return is not documented, and the tool table in `DESIGN-BRIEF.md` is out of date ([#26](https://github.com/crenshawdev/terminus/issues/26)).

## How it works

- **One Rust binary.** Claude Code runs it two ways. As a hook on `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`, each entry naming `~/.local/bin/terminus` with no shell in between. And as an MCP server over stdio, which Claude Code starts for each session. Everything else is a command you type.
- **The hooks.** Each hook first starts an ingest in a detached process, then reads the event from stdin. `SessionStart` may hand Claude Code the brief, and `UserPromptSubmit` the earlier turns it adds to the prompt. `SessionEnd` and `PostCompact` hand over nothing, and run so the ingest they start picks up the end of the session and the record a compaction appends.
- **The ingest.** One pass runs at a time, under an OS file lock that is released when its process exits, so a pass that finds another running exits having done nothing and no stale lock is ever left behind. The pass reads each transcript from where the last one stopped and archives the complete records it finds. Then it closes the sessions that have gone idle, records their observations, applies `[retention]` and takes a snapshot when one is due.
- **The MCP server.** It opens the store read-only for each call, so a session archived while it runs is found by the next call. It opens no port, takes no lock, and exits when Claude Code closes its input.
- **Where the data is.** One SQLite database, `terminus.db`, in the data directory ([where that is](docs/configuration.md#the-data-directory)), with its snapshots beside it. Every file Terminus writes there is readable only by your user.
- **The archive.** Each session is stored as numbered blocks of its transcript, one row each, holding the block's number, its zstd-compressed bytes and a BLAKE3 hash of its uncompressed bytes. The session's checksum is a BLAKE3 hash over its block hashes in order. An append rewrites at most the session's last block and adds blocks after it. Under `[capture] mode = "full"`, the default, the archived bytes are the transcript exactly. `terminus verify` checks every block against its hash.
- **What is derived.** The turns, the full-text search index, and the paths, commands, errors and identifiers each turn names are built from the archive. `terminus reindex` drops and rebuilds them from the block rows alone, with the same turn ids. The log of injection decisions is kept beside the archive, because it records what happened at each prompt and the transcripts do not hold that. The observations are kept too: `terminus observations regenerate` recomputes them, and `reindex` leaves them alone.
- **On a crash.** Each transcript's new blocks, its session record, its turns and the position the next pass reads from commit in one transaction, so a pass killed partway leaves that transcript as its last commit left it, and the next pass reads the rest. Every ingest starts by rebuilding the derived tables of a store an older build wrote, and by lowering any read position that claims more bytes than the archive holds, so the gap is read again. Recovery never deletes archived bytes.
- **On a context loss.** A new session gets the brief. A compaction fires `SessionStart` again, and Terminus leaves a note for that session. The next prompt then looks through a wider set of matches and is offered only the turns the compaction dropped, worked out from the record Claude Code appends to the transcript. If the ingest has not archived that record yet when the prompt arrives, the note carries to the prompt after.

The design decisions are in [DESIGN-BRIEF.md](DESIGN-BRIEF.md).

## Hosts and platforms

Claude Code only, on Linux and macOS, on x86_64 and aarch64.

Installing needs `curl`, `ssh-keygen`, and `sha256sum` or `shasum`. It also needs `claude` on `PATH` whenever the `terminus` MCP entry in `~/.claude.json` is missing or not in the shape install writes, because install registers the server by running `claude mcp`. Without `claude` in that case, install does everything else, prints the command that finishes the registration, and exits 1.

`git` is optional. With it, a session belongs to the repository its working directory is in. Without it, or when that directory is not a repository, a session belongs to the working directory itself, so one started in a subfolder of a repository is filed under that subfolder.

## Installing and updating

```sh
curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh
```

`install.sh` checks the release's signature against the key pinned in the script before it installs anything, places the binary behind `~/.local/bin/terminus`, and runs `terminus install`, which writes the hook entries and the MCP entry and starts archiving the transcripts already on disk. Running the same line again is how Terminus updates. It installs on Linux and macOS only.

To install a local build from a checkout instead:

```sh
cargo build --release && target/release/terminus install
```

A local build is not checked against any signature. What install writes, when it exits 1, how an update leaves older versions in place, and what `terminus uninstall` removes and leaves behind are in [docs/install.md](docs/install.md).

## Commands

Every command is `terminus <command>`.

- `ingest`: archives what is new in every transcript under `roots`.
- `ingest <path.jsonl>`: archives one transcript file.
- `backfill`: archives the transcripts already on disk, in a detached process, and returns.
- `search <words>`: finds past turns by text, path, error or identifier.
- `show <id>...`: prints the archived record behind each turn id.
- `sessions`: lists the archived sessions.
- `verify`: checks every archived block against its hash.
- `reindex`: rebuilds everything derived from the archive.
- `status`: prints the store's size and counts, how far into the transcripts it has read, and the last ingest pass.
- `compact`: gives back the disk space retention freed.
- `usage`: prints where the bytes went, by project, by month and by table.
- `export <dir>`: writes the archived transcripts out, unfiltered, with a manifest.
- `stats`: reports the hits, false positives, precision and misses of what was added to prompts.
- `observations`: lists what happened in each closed session.
- `observations regenerate`: recomputes those facts from the archive.
- `replay`: re-scores the logged prompts under other thresholds, and changes nothing.
- `retention --dry-run`: prints what the next ingest pass would evict and delete, and changes nothing.
- `doctor`: reports what is wired into Claude Code and what is not.
- `install`: wires Terminus into Claude Code's hooks and MCP servers, then starts a backfill.
- `uninstall`: removes the hook entries, the MCP entry and the `~/.local/bin/terminus` link that `install` added. It keeps the archive, and keeps the versions folder because a session that is already running may be running from it. `--purge` also deletes the data directory, after asking.
- `data move <path>`: moves the data directory and records where it went.

Claude Code runs two more, which are never typed:

- `hook <event>`: runs on `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`, starts an ingest, and may hand Claude Code the brief or earlier turns.
- `mcp`: serves `recall_search`, `recall_context` and `recall_get` over stdio.

Every command above except `ingest`, `backfill`, `install`, `uninstall`, `data move`, `hook` and `mcp` accepts `--json`, and then writes one JSON document on stdout ([docs/json-shapes.md](docs/json-shapes.md)). Every option of every command is in [docs/commands.md](docs/commands.md).

## Settings

Terminus runs on its defaults with no file. To change one, write `terminus.toml` in `$TERMINUS_CONFIG_DIR`, otherwise `$XDG_CONFIG_HOME/terminus`, otherwise `~/.config/terminus`. Terminus never writes the file, ignores keys it does not know, and refuses a `[provider]` table.

| Key | Default | What it sets |
|---|---|---|
| [`roots`](docs/configuration.md#roots) | `$CLAUDE_CONFIG_DIR`, otherwise `~/.claude` | The Claude Code config directories whose transcripts are archived. |
| [`exclude`](docs/configuration.md#exclude) | none | Projects that are never read and never returned. |
| [`[injection] brief_chars`](docs/configuration.md#injection) | 6000 | The most the brief at the start of a session may carry, in characters, capped at 10000. |
| [`[injection] prompt_chars`](docs/configuration.md#injection) | 4000 | The most one prompt's injection may carry, in characters, capped at 10000. |
| [`[retention] action`](docs/configuration.md#retention) | `keep` | `keep`, `evict` or `delete` for sessions past `age_days`. |
| [`[retention] age_days`](docs/configuration.md#retention) | 0, which is off | How old a session's last turn must be before `action` applies. |
| [`[retention.project."<path>"]`](docs/configuration.md#retentionprojectpath) | none | `action` and `age_days` for one project, in place of the global rule. |
| [`[snapshot] enabled`](docs/configuration.md#snapshot) | `true` | Whether rolling copies of the archive are taken. |
| [`[snapshot] interval_hours`](docs/configuration.md#snapshot) | 24 | How long after the last snapshot the next one is taken. |
| [`[snapshot] keep`](docs/configuration.md#snapshot) | 3 | How many snapshots are kept. |
| [`[capture] mode`](docs/configuration.md#capture) | `full` | `full`, `lean` or `minimal`: how much of each record is stored. Only `full` keeps the transcript exactly. |
| [`[privacy] redact_recall`](docs/configuration.md#privacy) | `false` | Filters text shaped like a secret out of what Claude Code is handed. |

Each key's full effect, and the environment variables Terminus reads, are in [docs/configuration.md](docs/configuration.md).

## How it is designed and built

- **How it is designed:** the design decisions are in [DESIGN-BRIEF.md](DESIGN-BRIEF.md) and in [.planning/](.planning/).
- **How it is built:** `main` is protected, and every change reaches it through a pull request. The test workflow, `.github/workflows/test.yml`, runs the whole suite on every push and pull request, on Linux and macOS, on x86_64 and aarch64. No status check is required. To run the tests locally, use `cargo test --workspace --all-features`.
- **The issues:** bugs and known gaps are tracked at [github.com/crenshawdev/terminus/issues](https://github.com/crenshawdev/terminus/issues).

## Issues and contributions

Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

## Sponsoring

Terminus is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it.

## License

[MIT](LICENSE)
