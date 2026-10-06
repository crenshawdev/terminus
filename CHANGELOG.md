# Changelog

All notable changes to Terminus, named Verbatim through 0.1.1, are recorded
here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Verbatim is now Terminus. The binary is `terminus`, the crates are `terminus`
  and `terminus-core`, and the MCP server registers as `terminus`, so its tools
  arrive as `mcp__terminus__recall_search`, `recall_context` and `recall_get`.
  The data directory is `~/.local/share/terminus` (macOS
  `~/Library/Application Support/terminus`, Windows `%LOCALAPPDATA%\terminus`)
  holding `terminus.db`, the config file is `terminus.toml`, every `VERBATIM_*`
  environment variable is `TERMINUS_*`, snapshots are `terminus-<time>.db`, and
  `lean` and `minimal` capture mark an elided body `{"terminusElided": <bytes>}`.
- Nothing reads the old names, so an existing install moves once by hand:
  `verbatim uninstall` with the old binary, checkpoint the WAL, rename the data
  directory, `verbatim.db` and its snapshots, then `terminus install`. A store
  written in `lean` or `minimal` mode keeps its old elision marks, which this
  build does not count. Archived calls to the old `mcp__verbatim__recall_search`
  are still recognized.

## [0.1.1] - 2026-09-04

A privacy and correctness release. Three of the four things in it are defects
in shipped 0.1.0 behaviour, and one of them meant the redaction you thought you
had was mostly not running.

### Fixed

- The `SessionStart` resume brief quoted the wrong thing. Claude Code writes
  tool results as `type: "user"` records, so any session that ended inside a
  tool loop had its harness payload quoted back under "It last asked", which
  attributes words to you that you never typed. Every `user` record is now
  classified typed or not at parse time into a new `turns.is_typed` column, and
  the brief reads it. A session with no typed prompt renders no prompt line
  rather than falling back to the tool result. An existing store gains the
  column by backfill from its own blobs, no re-ingest.
- Egress redaction was mostly inert on the wire. The filter ran on the
  already-serialized JSON body, and a serialized body has no real newlines, so
  the header rule saw one enormous line whose first colon sat inside `{"model":`
  and rejected it, and the JSON-pair rule consumed the whole transcript as a
  single candidate name. Only two of the five rules ever fired. The filter now
  runs per message content before the body is built, and the rule set went from
  5 rules to 9: bare JWTs, GitHub tokens, connection URLs carrying userinfo and
  space-separated `--token` flags are caught now, `cookie` joined the secret
  names, and the header rule tests every colon on a line instead of the first.
  The tests assert over the exact bytes handed to the socket, not hand-written
  samples.

### Added

- Everything Verbatim writes lands owner-only. The data directory and
  `verbatim.db` are created 0700/0600 under umask 022, snapshots, the lock file,
  injection scratch, decision records, export output and `data move`'s
  destination tree all inherit it, and a store that was already group- or
  world-readable is tightened once on a writable open. A group- or
  world-readable `verbatim.toml` is refused where its `api_key` is consumed, the
  same refusal the shared credentials file already had. Unix only; `doctor`
  reports the mode and states the Windows deferral.
- `[privacy] redact_recall`, off by default. Turn it on and the egress filter
  runs over every projection that renders archived text into model context:
  `recall_search`'s excerpt and observation claims, `recall_context`'s window,
  `recall_get`'s body, the `SessionStart` brief's quotes and the per-prompt
  injection. A judgment's `topic`, `outcome` and claim text are filtered before
  they reach SQLite, and observation rows written before you flipped the knob
  are filtered on read, so nothing needs regenerating. `verbatim search --raw`
  and `verbatim show --raw` are the owner's escape hatch, per invocation, and no
  environment variable can reach past the knob into the hook or the MCP server.
  The archive on disk is never touched in either setting, and `verbatim export`
  stays unfiltered with the notice it already carries.

### Known issues

- With `redact_recall` on, `recall_get` runs its body through a lossy UTF-8
  round trip, so an archived line that is not valid UTF-8 comes back with
  U+FFFD replacement characters even when it carries no secret. All 47,157
  archived records measured on the development corpus are valid UTF-8, so this
  costs nothing observable there. Tracked as
  [#1](https://github.com/crenshawdev/verbatim/issues/1).

## [0.1.0] - 2026-08-23

First release. A single static Rust binary that tails Claude Code's session
transcripts, stores every session unmodified and permanently, indexes it at turn
granularity, and gives the model recall over its own history through three MCP
tools and hook-driven context injection. Local only, no daemon, no port, no
telemetry.

[Unreleased]: https://github.com/crenshawdev/verbatim/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/crenshawdev/verbatim/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/crenshawdev/verbatim/releases/tag/v0.1.0
