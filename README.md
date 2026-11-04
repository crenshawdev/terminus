# Terminus

Claude Code forgets. A new session starts blank, a long session that compacts loses its earlier part, and by default Claude Code deletes transcripts older than 30 days.

Terminus keeps every session and offers it back to Claude:

* A new session starts knowing where the last one in the project left off.
* When a prompt names a file or a function from earlier work, Claude can be handed up to three of the turns where it came up.
* After a compaction, the next prompt that names a file or a function can get back up to three of the turns that dropped out. Claude can search for the rest.
* Claude can search the whole history itself, by text, path, error or identifier. "Where did the retry budget come from?" still has an answer three weeks later.
* You can search it too, from the command line.
* Every subagent's transcript is kept as well, so you can see what each agent was asked, what it ran and what it reported.

Other memory tools for Claude Code send your sessions to a model, on your account, and keep what it chose to write down. Terminus keeps the session: every prompt, every reply, every tool call and its output, with nothing summarized. Search covers what each turn said, up to 256 KiB of text per turn, whether it happened a few minutes ago or in the first session Terminus archived. Text past that limit stays in the archive, where `recall_get` and `terminus show` read it and search does not find it. [`terminus search`](docs/commands.md#terminus-search-words) lists what is indexed, and [`[capture]`](docs/configuration.md#capture) and [`[retention]`](docs/configuration.md#retention) can keep less.

It is one local binary. It makes no model call of its own, so nothing runs up your bill or your usage limit in the background. It runs no daemon and opens no network connection. What it hands Claude goes into the session like anything else you type, by default at most 6,000 characters when a session starts and 4,000 on a prompt. [`[injection]`](docs/configuration.md#injection) can set either one up to 10,000.

## Install

```console
$ curl -fsSL https://raw.githubusercontent.com/jcrenshaw-dev/terminus/main/install.sh | sh
```

Linux and macOS, on x86_64 and aarch64. It needs `curl`, `ssh-keygen`, `sha256sum` or `shasum`, and Claude Code's `claude` command, which registers the MCP server. The script checks the release's signature before it installs anything, wires Terminus into Claude Code, and archives the transcripts already on disk.

Run the same line again to update.

Each release bumps the middle number of the version, a release with only fixes bumps the last, and the first moves only for a large change. The number does not promise compatibility. The [changelog](CHANGELOG.md) says what each release changes, and `terminus --version` prints the one you have.

To build from source instead:

```console
$ cargo build --release
$ target/release/terminus install
```

A local build is not checked against any signature.

## Documentation

**Using it:** [Installing, updating and uninstalling](docs/install.md) · [Commands](docs/commands.md) · [Settings](docs/configuration.md) · [JSON output](docs/json-shapes.md)

**How it is built:** [Design](docs/design/README.md) · [How a release is built, signed and checked](docs/release.md)

**Project:** [Changelog](CHANGELOG.md) · [Security policy](SECURITY.md) · [Trust](TRUST.md)

## How Claude gets it

Terminus offers Claude earlier work at three points. Each is an offer, not a guarantee: it depends on what the archive holds, and a hook that misses its deadline hands over nothing.

* **When a session starts:** a brief of where the last session in the project left off: its date, the branch it ended on, and the last thing said each way.
* **On a prompt:** up to three earlier turns that named the same paths or identifiers, when they clear the match threshold. Most prompts get nothing.
* **After a compaction:** the next prompt that names a path or identifier looks only at the turns the compaction dropped, under the same match threshold and character budget. It gets at most three of them and often none, and `recall_search` is how Claude reaches the rest.

`[injection]` caps each in characters, and `[privacy] redact_recall` filters text shaped like a secret out of what is handed over.

Claude can also look things up itself, at any time, through three MCP tools that go from cheap to complete:

1. `recall_search` finds past turns by text, path, error or identifier, and returns an excerpt and a turn id for each hit.
2. `recall_context` reads the turns around one hit, so Claude can see what led to it and what followed.
3. `recall_get` reads the full archived text of the turns it names.

Claude reads in full only the turns it has already narrowed down to.

## Reading it yourself

```console
$ terminus sessions
2026-08-13  4f1c8d2a-7b3e-4c91-a0d5-2e6b9f13c7e2.jsonl  3 turn(s)  /home/you/code/project-alpha  phase-5
2026-08-12  c3e21f07-9d4b-4a6e-8f15-b02a7d9e4c61.jsonl  6 turn(s)  /home/you/code/project-beta  phase-3
2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  6 turn(s)  /home/you/code/project-alpha  phase-3
3 session(s)

$ terminus search retry budget
50331652  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    Grep src retryBudget
50331648  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    where did SearchManager get its retry budget
50331649  2026-08-12  9a07e5b1-3c2d-4f8a-b6e0-71d4c2a85f3b.jsonl  /home/you/code/project-alpha
    the retry budget lives in src/worker/S.ts near the top
3 hit(s)
```

`terminus show <id>` prints the archived turn behind an id. Each of these answers for the project you are in; `--project <path>` picks another, and `--project '*'` covers every project.

`terminus observations` lists what happened in each closed session, taken from the transcript's own records with no model involved: the files modified and read, the tools, the commands, the errors and the commits.

```console
$ terminus observations --project ~/code/project-beta
c3e21f07-9d4b-4a6e-8f15-b02a7d9e4c61.jsonl  phase-3  6 turn(s)  2m 9s  0 compaction(s)
  tools           Bash
  commands        cargo test -p terminus-core --features testkit, cargo build --release, sleep 600
  errors          Bash command failed, error: could not compile terminus-core due to 7 previous errors
1 observation(s)
```

Every command and option is in [docs/commands.md](docs/commands.md).

## When something is wrong

* `terminus doctor` reports what is and is not wired into Claude Code: the hook entries, the MCP entry, the store and the last ingest pass. Where there is a fix to run, it prints the command.
* `terminus verify` checks every archived block against its hash.
* `terminus reindex` rebuilds the search index and the other tables derived from the archived blocks, with the same turn ids. It does not touch the archive, the observations or the injection log, and it keeps an evicted session's rows as they are, because its blocks are gone.
* `[snapshot]` keeps rolling copies of the archive: by default the last three, taken at most once every 24 hours.

## Design principles

* **Lossless.** Each session is stored as numbered blocks of its transcript, compressed with zstd. Under the default capture mode the archived bytes are the transcript exactly.
* **Verifiable.** Each block carries a BLAKE3 hash of its bytes, and each session a hash over its block hashes.
* **No model.** Search is SQLite FTS5 with no model and no embeddings, and an identifier is indexed by its parts, so `manager` finds `SearchManager`. What Terminus records about a session comes from the transcript's own records.
* **Rebuildable.** The search tables are derived from the blocks and can be rebuilt from them, except an evicted session's, whose blocks are gone. Observations have their own rebuild, `terminus observations regenerate`, and the injection log has none. A pass killed partway leaves each transcript as its last commit left it, and recovery loses no archived byte.
* **Local.** One SQLite file in your data directory, with its snapshots beside it, readable only by your user. The binary opens no network connection, and a test fails if any source file names a socket type.
* **Out of the way.** No daemon: each hook starts a detached pass and exits. A hook exits 0 whatever goes wrong, so it never blocks a prompt.

## Why shouldn't I use Terminus?

* You run Windows. It is not supported, and a build for it fails at compile time. [TRUST.md](TRUST.md) says why.
* You use a coding agent other than Claude Code. Terminus reads only Claude Code's transcripts and wires only into Claude Code.
* You need encryption at rest. Terminus has none. Its files are protected only by being readable by your user alone, and Claude Code's own transcripts sit unencrypted in its folder too.
* You want semantic search or summaries. Terminus calls no model. It finds turns by text, path, error and identifier.
* You don't want earlier turns sent back to the model. That is what Terminus does.

## Trust

Terminus collects nothing about you and sends no telemetry. The platforms it supports follow from that: Linux and macOS meet the standard, Windows fails it on privacy and on security and is not supported, and a platform that stops meeting it loses support. The criteria are in [TRUST.md](TRUST.md).

**What leaves your machine.** Install and update fetch `install.sh` from `raw.githubusercontent.com`, then the manifest, its signature and the binary from this repository's GitHub releases, or from the `https://` source you pass. The archived turns Terminus hands Claude Code go to Claude's model with the rest of the session. Nothing else.

Releases are signed on the maintainer's machine, never in CI, and `install.sh` checks the signature before it installs anything. How these claims are checked is in [SECURITY.md](SECURITY.md).

## Current state

Every open bug is in the [issues](https://github.com/jcrenshaw-dev/terminus/issues).

## Issues and contributions

Issues are welcome: bugs, questions, and reports of anything in the docs that is wrong. Pull requests are by invitation only; open an issue first. Security problems go through [SECURITY.md](SECURITY.md), not a public issue.

The tests run with `cargo test --workspace --all-features`. CI runs them on every pull request and every push to `main`, on the debug and the release build, on Linux and macOS, on x86_64 and aarch64. CI skips three tests of a hook spawn's p99 on every runner and three time budgets on a single spawn on the macOS runners, because a hosted VM's timing measures the VM. The tests over a real transcript corpus need `TERMINUS_TEST_CORPUS` and that private data, so they skip themselves in CI and on any machine without it. `main` takes changes only through pull requests with signed commits, and only once the tests pass.

## Sponsoring

Terminus is open source, built by one person, and sold to no one. If it is useful to you, [GitHub Sponsors](https://github.com/sponsors/crenshawdev) is the way to support it.

## License

[MIT](LICENSE)
