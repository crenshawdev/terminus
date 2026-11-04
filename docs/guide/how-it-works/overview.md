# How it fits together

Terminus is one binary with no daemon. Claude Code starts it when something happens, and it exits when that's done.

Claude Code writes each session to a transcript and runs `terminus hook` on four events: `SessionStart`, `UserPromptSubmit`, `SessionEnd` and `PostCompact`. Each run starts a detached `terminus ingest` and then, on the first two events, may hand text back. The ingest copies each transcript's new bytes into the store, indexes them and exits. One pass runs at a time, and a pass that finds another one running exits having done nothing.

Claude Code also starts `terminus mcp` and calls its three recall tools over stdio. The server opens no port, takes no lock and opens the store read-only for each call. The terminal commands read the same store.

The archived session is the record. Everything else in the store, the turns, the search index and the structured matches, comes from those blocks and can be rebuilt from them, except for an evicted session, whose blocks are gone.

## The numbers that keep it out of your way

- A hook has 50 ms for its text. Past that, it hands over nothing for that event.
- A hook reads at most 1 MiB of stdin and waits at most 500 ms for it.
- Once a hook knows its event, it exits 0 whatever goes wrong, so it can't block a prompt.
- Anything that goes wrong in a hook is one line on stderr. Failures in a pass are recorded for `terminus status`, since there's no log file.
- A format change to the derived tables rebuilds them at the start of the next pass. An archive in another format is refused, never converted.

## The design docs

Each part has a design doc that says how it works now, for an engineer who wants to see inside it. They live on GitHub, where the Mermaid diagrams render with nothing else installed. Start with the overview, which opens on one diagram of the whole system.

| Doc | What it covers |
|---|---|
| [0001: Overview](https://github.com/crenshawdev/terminus/blob/main/docs/design/0001-overview.md) | The whole system in one diagram, the two crates, the data directory, the config file, the format versions, and the processes and threads. |
| [0002: Hooks](https://github.com/crenshawdev/terminus/blob/main/docs/design/0002-hooks.md) | The four hook events, the detached spawn, the bounded read of the payload, and what the hook may write back. |
| [0003: Ingest](https://github.com/crenshawdev/terminus/blob/main/docs/design/0003-ingest.md) | One ingest pass from the lock to the snapshot: recovery, discovery, the per-file transaction, observations, retention and the backfill. |
| [0004: The archive and its block rows](https://github.com/crenshawdev/terminus/blob/main/docs/design/0004-archive-and-block-rows.md) | The schema, the block rows and their hashes, and the commands that check, rebuild, compact, export and move the store. |
| [0005: Search](https://github.com/crenshawdev/terminus/blob/main/docs/design/0005-search.md) | What a turn puts in the index, how a query is ranked and scoped, and the terminal `search`, `show` and `sessions`. |
| [0006: Recall through MCP](https://github.com/crenshawdev/terminus/blob/main/docs/design/0006-recall-through-mcp.md) | The stdio JSON-RPC server, its three tools and what each returns, and the egress filter. |
| [0007: Injection](https://github.com/crenshawdev/terminus/blob/main/docs/design/0007-injection.md) | The resume brief, the per-prompt injection, the decision log, labels, `stats` and `replay`. |
| [0008: Install and release](https://github.com/crenshawdev/terminus/blob/main/docs/design/0008-install-and-release.md) | `install.sh`, `terminus install`, `doctor` and `uninstall`, and how a release is built, signed and checked. |
