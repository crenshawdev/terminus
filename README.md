# Terminus

[![Test](https://github.com/jcrenshaw-dev/terminus/actions/workflows/test.yml/badge.svg)](https://github.com/jcrenshaw-dev/terminus/actions/workflows/test.yml)
[![Latest release](https://img.shields.io/github/v/release/jcrenshaw-dev/terminus?label=release)](https://github.com/jcrenshaw-dev/terminus/releases/latest)
[![License: MIT](https://img.shields.io/github/license/jcrenshaw-dev/terminus)](LICENSE)

Terminus is a local archive of your Claude Code sessions, for a developer who comes back to the same projects and wants what was said and decided in them to still be there.

Claude Code forgets. A new session starts blank, a long session that compacts loses its earlier part, and by default Claude Code deletes transcripts older than 30 days. So you explain the same decision again, and when you ask where a retry budget came from, the session that set it is gone.

Terminus keeps every session, subagents included, and offers it back. A new session starts knowing where the last one in the project left off. A prompt that names a file or a function can be handed up to three of the turns where it came up, and after a compaction the next such prompt looks at the turns that dropped out. Most prompts get nothing, because a turn has to match on something specific. Claude can also search the whole history itself through three MCP tools, and you can search it from the command line.

You decide which projects it never reads, how long sessions stay and how much text it may hand back. With no configuration file at all, it keeps everything.

## Every turn is kept, and no model is involved

A memory tool that summarizes sends your sessions to a model, on your account, and keeps what it chose to write down. The detail you go looking for three weeks later may be the one it left out.

Terminus keeps the session instead: every prompt, every reply, every tool call and its output.

Under the defaults:

- Each session is stored as compressed blocks of its transcript, and the archived bytes are the transcript exactly. Each block carries a BLAKE3 hash, and `terminus verify` checks every one. [`[capture]`](https://jcrenshaw-dev.github.io/terminus/configuration/archiving.html#capture) can store less.
- Search is SQLite FTS5 with no embeddings. It finds turns by text, path, error and identifier, and it indexes an identifier by its parts, so `manager` finds `SearchManager`. It doesn't find turns by meaning.
- The index covers up to 256 KiB of text per turn, whether it happened a few minutes ago or in the first session Terminus archived. Text past that limit stays in the archive, where `recall_get` and `terminus show` read it and search doesn't find it.
- Terminus makes no model call of its own, so nothing runs up your bill or your usage limit in the background. It runs no daemon and opens no network connection, and a test fails if any source file names a socket type.
- What it hands Claude goes into the session like anything else you type, so it goes to Claude's model and takes up context. That's at most 6,000 characters when a session starts and 4,000 on a prompt, and [`[injection]`](https://jcrenshaw-dev.github.io/terminus/configuration/recall.html#injection) can set either one up to 10,000.

Terminus doesn't manage your active context. It offers at most three matching turns on a prompt, and no measurement shows that replaces compaction. `terminus stats` reports how often what it handed over came up again in your own sessions, and `terminus replay` re-scores your logged prompts under other thresholds, so you can judge it on your own work.

Read [the archive chapter](https://jcrenshaw-dev.github.io/terminus/archive/what-is-kept.html) for what's stored, what search covers, and how to keep less.

## Get started

You'll need Linux or macOS on x86_64 or aarch64, Claude Code and its `claude` command, `curl`, `ssh-keygen`, and `sha256sum` or `shasum`. Terminus reads only Claude Code's transcripts, so it does nothing for another coding agent. Windows isn't supported, and a build for it fails at compile time. [TRUST.md](TRUST.md) says why.

```console
$ curl -fsSL https://raw.githubusercontent.com/jcrenshaw-dev/terminus/main/install.sh | sh
```

The script checks the release's signature before it installs anything, wires Terminus into Claude Code, and archives the transcripts already on disk. Start a new Claude Code session and it's working. Typing `terminus` by name needs `~/.local/bin` on your `PATH`, and nothing edits your shell profile to add it.

Run the same line to update. `terminus uninstall` takes Terminus out of Claude Code and keeps the archive.

## Reading it yourself

```console
$ terminus search retry budget
50331652  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    Grep src retryBudget
50331648  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    where did SearchManager get its retry budget
50331649  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    the retry budget lives in src/worker/S.ts near the top
3 hit(s)
```

| Command | What it does |
|---|---|
| `terminus search <words>` | Find past turns by text, path, error or identifier. |
| `terminus show <id>` | Print the archived turn behind an id, byte for byte. |
| `terminus sessions` | List the archived sessions, subagents' included. |
| `terminus observations` | List what happened in each closed session: the files, tools, commands, errors and commits. |
| `terminus stats` | Report how what was handed to Claude is doing. |
| `terminus doctor` | Report what is and isn't wired into Claude Code. |
| `terminus verify` | Check every archived block against its hash. |

The first four answer for the project you're in. `--project <path>` picks another, and `--project '*'` covers every project. Claude reaches the same archive through `recall_search`, `recall_context` and `recall_get`, which go from an excerpt to the full text, so it reads in full only the turns it has narrowed down to.

## Documentation

[Read the Terminus guide](https://jcrenshaw-dev.github.io/terminus/), or [browse the Markdown on GitHub](./docs/guide/SUMMARY.md).

| Start here | What you'll find |
|---|---|
| [Getting started](https://jcrenshaw-dev.github.io/terminus/getting-started/install.html) | Installing, checking the wiring, updating and removing. |
| [Working with Terminus](https://jcrenshaw-dev.github.io/terminus/working/what-claude-gets.html) | What Claude gets and when, reading the archive, and what to do when something's wrong. |
| [The archive](https://jcrenshaw-dev.github.io/terminus/archive/what-is-kept.html) | What's stored, what search finds, whether it's helping, and how to keep less. |
| [Configuration](https://jcrenshaw-dev.github.io/terminus/configuration/index.html) | Every key in `terminus.toml`, its default and what it changes. |
| [Commands](https://jcrenshaw-dev.github.io/terminus/reference/commands.html) · [JSON output](https://jcrenshaw-dev.github.io/terminus/reference/json-shapes.html) | Every command and option, and the `--json` contract with its exit codes. |
| [Permissions and data](https://jcrenshaw-dev.github.io/terminus/reference/permissions.html) | What it reads, writes, runs and sends, and what is and isn't protected. |
| [Releases](https://jcrenshaw-dev.github.io/terminus/reference/release.html) | How a release is built, signed and checked, and how to check one by hand. |
| [How it works](https://jcrenshaw-dev.github.io/terminus/how-it-works/overview.html) · [Design docs](./docs/design/README.md) | How the parts fit together, the limits of each check, and the detailed design. |

## Trust

Terminus collects nothing about you and sends no telemetry. The platforms it supports follow from that: Linux and macOS meet the standard, Windows fails it on privacy and on security and isn't supported, and a platform that stops meeting it loses support. The criteria are in [TRUST.md](TRUST.md).

What leaves your machine: install and update fetch `install.sh` from `raw.githubusercontent.com`, then the manifest, its signature and the binary from this repository's GitHub releases, or from the `https://` source you pass. The archived turns Terminus hands Claude Code go to Claude's model with the rest of the session. Nothing else.

The archive has no encryption at rest. Its files are protected only by being readable by your user alone, and it holds whatever your transcripts hold, secrets you pasted included. Claude Code's own transcripts sit unencrypted in its folder too. [Permissions and data](https://jcrenshaw-dev.github.io/terminus/reference/permissions.html) lists what is and isn't protected.

Releases are signed on the maintainer's machine, never in CI, and `install.sh` checks the signature before it installs anything. How these claims are checked is in [SECURITY.md](SECURITY.md).

## Help

Questions go on the [jcrenshaw.dev Discord](https://jcrenshaw.dev/discord). Bugs and anything in the docs that's wrong go to [GitHub issues](https://github.com/jcrenshaw-dev/terminus/issues). Pull requests are by invitation only, so open an issue first, and see [Contributing](https://jcrenshaw-dev.github.io/terminus/contributing.html). Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

Terminus is maintained by John Crenshaw and distributed under the [MIT License](LICENSE).

[Support Terminus on GitHub Sponsors](https://github.com/sponsors/crenshawdev).
