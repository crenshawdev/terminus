# How Terminus works

These docs describe how Terminus works as it stands, for an engineer who wants to see inside it. Start with [0001: Overview](0001-overview.md), which opens on one diagram of the whole system, from the hook to ingest to the archive to search to recall, and then read the doc for the part you want.

## Current design only

Each doc states the current design and nothing else. When the design changes, the doc is edited in place, in every section the change touches. A doc carries no amendment lists and no superseded copies, and git holds the history.

A doc describes the design. It never tracks work: no task lists, no next steps and no schedule.

Numbers are four digits, assigned in order and never reused.

## The form of a doc

The overview has its own form and opens on the whole-system diagram. Every other doc describes one part and follows the seven sections of [TEMPLATE.md](TEMPLATE.md), in order: purpose and scope, terms, crate, modules and boundaries, records, commands and operations, states and workflows, and settings. A section with nothing to say for its part keeps its heading and says why in one sentence, so every part doc has the same shape.

## Diagrams

Diagrams are [Mermaid](https://mermaid.js.org/) inside the Markdown, so GitHub renders them with nothing else installed, and they diff as text. Each one is written by hand, in one of four types:

| Question the diagram answers | Diagram |
|---|---|
| How the parts connect, and which way the data goes | `flowchart` |
| Who does what, in what order, across actors, with `alt` for branches | `sequenceDiagram` |
| What states a record passes through | `stateDiagram-v2` |
| What the stored data looks like | `erDiagram` |

Every diagram has a caption line directly under it, in the form `*Figure N. What the diagram shows.*`, numbered in order through its doc. No diagram sets a `%%{init` line, `style`, `classDef` or a fill, so GitHub themes it for both light and dark.

## Naming code

Code is named by its full path from the repository root and linked relative to the doc, such as [`crates/terminus-core/src/store/open.rs`](../../crates/terminus-core/src/store/open.rs). It is never cited by line number, because a line number goes stale on the next edit.

## What the user docs hold

What the user docs already pin down is linked from here and not restated, so there is one place to change it:

- every command and option, in [the command reference](../guide/reference/commands.md);
- every key in `terminus.toml` and its default, in [the settings reference](../guide/configuration/index.md);
- the `--json` fields and the exit codes, in [JSON output and exit codes](../guide/reference/json-shapes.md);
- what the installer and `terminus install` do, in [the install page](../guide/getting-started/install.md);
- how a release is built, signed and checked, in [the release page](../guide/reference/release.md).

## Index

| Document | What it covers |
|---|---|
| [0001: Overview](0001-overview.md) | The whole system in one diagram, the two crates, the data directory, the config file, the format versions, and the processes and threads. |
| [0002: Hooks](0002-hooks.md) | The four hook events, the detached spawn, the bounded read of the payload, and what the hook may write back. |
| [0003: Ingest](0003-ingest.md) | One ingest pass from the lock to the snapshot: recovery, discovery, the per-file transaction, observations, retention and the backfill. |
| [0004: The archive and its block rows](0004-archive-and-block-rows.md) | The schema, the block rows and their hashes, and the commands that check, rebuild, compact, export and move the store. |
| [0005: Search](0005-search.md) | What a turn puts in the index, how a query is ranked and scoped, and the terminal `search`, `show` and `sessions`. |
| [0006: Recall through MCP](0006-recall-through-mcp.md) | The stdio JSON-RPC server, its three tools and what each returns, and the egress filter. |
| [0007: Injection](0007-injection.md) | The resume brief, the per-prompt injection, the decision log, labels, `stats` and `replay`. |
| [0008: Install and release](0008-install-and-release.md) | `install.sh`, `terminus install`, `doctor` and `uninstall`, and how a release is built, signed and checked. |
