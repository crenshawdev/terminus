# Terminus — Design Brief

Input for `/cad-new-project`. Every decision below is settled unless marked **OPEN**.

---

## 1. What it is

Persistent, cross-session memory for Claude Code, built on the premise that **the session transcript is the record**.

Claude Code already writes every prompt, tool call, tool result and assistant turn to `~/.claude/projects/**/*.jsonl`. Terminus tails those files, stores each session unmodified and permanently, indexes it, and gives the model precise recall over its own history.

Nothing is summarized at write time. Nothing is lossy. No inference is on the hot path.

## 2. Why it exists

The incumbent, `claude-mem` (90k stars, 224 open issues), runs a **second Claude instance** that watches your first one, compresses every tool call into an LLM-authored "observation", and throws the raw turn away. Consequences, all documented in its tracker:

- Users burn their token budget on the memory system itself; one reported $90 in three hours.
- The LLM's guess is the only copy, so retrieval quality can never be audited or measured.
- Context injection is `ORDER BY created_at DESC LIMIT n` — recency, not relevance.
- 45 open Windows issues, 36 Chroma issues, orphaned processes reaching 157 GB and OOM.

Terminus inverts all of it: keep the truth, derive everything else, make retrieval measurable.

## 3. Non-goals

- No multi-machine sync, teams, auth, or cloud.
- No harnesses other than Claude Code in 0.x (Codex, Cursor, OpenCode deferred — each is a parser, not an integration).
- No web viewer or HTTP server.
- No telemetry of any kind.
- No daemon, no port, no long-lived process.
- No encryption at rest.

## 4. Language and shape

Rust. Single static binary. No Node, Bun, Python, or native shared libraries at runtime.

Concurrency: no async runtime. Cold start is the metric that matters — the hook path must not pay for a runtime it doesn't use. `rayon` for backfill parse/compress fan-out; a single writer thread for SQLite.

---

## 5. Storage

One mechanism: **SQLite** via `rusqlite` with the `bundled` feature (vendored C source, statically linked, no system dependency, no `dlopen`).

```
<data dir>/
  terminus.db          everything
  terminus.db-wal
  terminus.db-shm
  LOCK                 ingest exclusivity
```

| Table | Role |
|---|---|
| `sessions` | session_key → session_no |
| `blocks` | (session_no, block_no) → zstd-compressed bytes + BLAKE3 hash of the uncompressed bytes |
| `session_meta` | project, cwd, branch, times, source, final?, evicted?, parent, continues-from |
| `turns` | turn_seq, kind, tool, ts, offset/len into the blob |
| `turns_fts` | FTS5 over expanded turn text. Contentless (`content=''`, `contentless_delete=1`). Derived, droppable |
| `entities` | (turn_id, kind, value_norm) — exact match for paths, symbols, commands, errors, tools |
| `paths` | turn → files touched |
| `watermarks` | transcript path → byte offset |
| `observations` | per finalized session: mechanical facts off the blob, kept rather than rebuilt |
| `decisions`, `labels` | injection decision log + replay outcomes |
| `runs` | last N ingest runs: timestamp, duration, counts, error |
| `meta` | store format version, compression flag |

Blob write, turn rows, FTS rows, entities and watermark **all commit in one transaction**. Nothing can be out of sync.

### Blob format

Each session's stream is cut into blocks of the size its `session_meta` row records, 64 KB today. Each block is compressed with zstd on its own and stored as one row in `blocks` with its hash. Reading one turn fetches and decompresses only the blocks its range covers, and an append rewrites at most the last block.

**Compression is on by default.** Measured corpus: 895 MB / 1,896 sessions / 25 days ≈ 13 GB/year uncompressed for very heavy use; ~2 GB/year compressed. A typical user is well under that.

No stored preview column — search results decompress the hit's block on demand (~60 µs per 64 KB block).

### Why not the alternatives

| Rejected | Reason |
|---|---|
| redb | Exclusive file lock blocks *all* other processes, including readers. Young library holding the only copy of user history |
| Tantivy | Second store, second commit, divergence handling. FTS5 is adequate at this corpus size and gives single-transaction consistency |
| Turso / limbo | Beta; maintainers explicitly advise against mission-critical use |
| Postgres | Requires a server. |
| DuckDB / SurrealDB embedded | Single-process write access; forces a daemon |
| Flat files / one file per session | Rejected outright |

### Truth and derived data

`sessions` is truth. Turns, FTS, entities and paths are derived and rebuildable by replaying blobs. `decisions` and `labels` are kept, not rebuilt: what an injector decided at prompt time is in no blob. `observations` is kept too: its facts come off the session blob, and retention eviction empties that blob, so an evicted session's observation cannot be recomputed.

**The archive tables never migrate.** A store in an older archive format is refused with the clean-install steps (move the data folder and the binary aside, rerun `install.sh`), a store in a newer one is refused with a message to upgrade, and nothing converts either. Only a derived schema that differs from the binary's rebuilds the derived tables. This is the property that prevents claude-mem's 49-migration situation.

### Durability

- A BLAKE3 hash per block, and a session checksum over the block hashes in `session_meta`, independent of SQLite's own integrity checks, so corruption is pinned to a named session and block rather than "the store is gone".
- `PRAGMA integrity_check`, plus what each append checks: the stored block hashes against the session checksum, and the tail block against its own hash. Complete blocks are neither read nor rewritten.
- Online backup / `VACUUM INTO` for consistent snapshots without stopping ingest.
- Rolling snapshots on by default.
- Upstream transcripts are a second recovery path, and Claude Code deletes them after `cleanupPeriodDays` (default 30 days). The install receipt shows the current value, says that transcripts older than it are already gone, and prints the line that raises it to 3650. Terminus never changes the setting itself.

---

## 6. Ingest

### Trigger model

Four hooks, all spawning a **detached** ingest process and returning 0 immediately.

| Hook | Synchronous work | Purpose |
|---|---|---|
| `SessionStart` | read index, emit resume brief, spawn ingest | **Required.** Only injection point; catch-up path when other triggers didn't fire |
| `UserPromptSubmit` | one entity query, inject 0–3 turns or nothing | Relevance. Hard deadline, fail open |
| `SessionEnd` | none | Freshness — captures a finished session in seconds. Does not fire on crash/`kill -9` |
| `PostCompact` | none | Records the compaction boundary. Cannot inject (no `hookSpecificOutput`) |

Hook command is the **absolute path to the binary**. No shell, no `$SHELL -lc` PATH probe, no version resolution. This single choice retires most of claude-mem's Windows issue class.

Spawn is fully detached: stdio to null, no inherited handles, new process group.

No daemon, no systemd/launchd/Task Scheduler. The hook spawn is the scheduler.

### Ingest run

1. Try-lock (`flock`). Held → exit 0 immediately. No waiting, no retry storms.
2. Read watermarks.
3. Tail each transcript to the last complete record; skip excluded projects.
4. Cut the stream into blocks, compress each one.
5. Commit the new block rows, the replaced tail row, turns, FTS, entities and watermark in one transaction.
6. Apply retention if configured; compact if needed.
7. Re-check for new work; loop. Else exit, bounded by a max wall clock.

Recovery lives entirely in the binary at the top of every run. There is no `repair` command and no external supervisor. OS locks are used precisely because they die with the process — no PID files, no stale-lock wedging.

### Transcript facts (verified against 1,896 real files)

- **Compaction is append-only.** Same file, same session id; two records appended (`system`/`compact_boundary`, then `user` with `isCompactSummary`). Nothing is deleted or rewritten. Byte-offset tailing is safe.
- `compact_boundary` carries `preTokens`, `postTokens`, `cumulativeDroppedTokens`, and `preservedMessages.uuids` / `allUuids` — **exactly which turns fell out of the model's context.**
- **Subagents live in a nested sidecar directory**: `<project>/<sessionId>/subagents/agent-*.jsonl` plus `.meta.json`, `isSidechain: true`. 214 dirs, 1,135 files, 84,825 records — invisible to a top-level glob. Discovery must recurse and set `dot: true`.
- 1 file = 1 session = filename. No file mixes session ids. 0 unparseable lines in 5,915 sampled.
- **1.2% of sessions continue across files** (resume/fork), linked by `parentUuid`. Needs a `continues_from` link.
- Every record carries `cwd` and `gitBranch` directly.

### Project identity

**Read the project from the record's `cwd`, never from the encoded directory name** (which is lossy — you cannot distinguish `-` from `/`) and never from `basename()`. Claude-mem's four separate identity bugs all trace to `basename()`.

Key on the **canonical git toplevel path**, falling back to canonical cwd. Store basename only as a display label. Map worktrees to the parent repo via the git common dir and store both keys.

Canonicalize everything before comparing — resolve symlinks, normalize separators, case-fold on macOS.

### Byte distribution (measured)

| Share | Record |
|---|---|
| 41.0% | tool results |
| 35.8% | assistant turns |
| 16.4% | attachments |
| **2.6%** | **user prompts** |

Capture modes: `full` (default), `lean` (elide large tool-result and attachment bodies, ~50%), `minimal` (prompts, assistant text, tool names and args, ~10–15%). Elision is marked in the record.

---

## 7. Search and retrieval

Index at **turn** granularity; store at **session** granularity.

Three MCP tools. Not twenty. Every tool description sits in every session's context forever.

| Tool | Params | Returns |
|---|---|---|
| `recall_search` | query, project?, paths?, tool?, kind?, since?, until?, limit?, offset? | ranked turns: id, session, ts, project, excerpt |
| `recall_context` | id, before?, after? | chronological turns around a hit |
| `recall_get` | ids[] | full verbatim text, plus `body_evicted` flag |

- `readOnlyHint: true` on all three, so plan mode stops re-prompting each session.
- One-line descriptions. The workflow is implied by the return shapes; no meta-tool explaining the other tools.
- Auto-scoped to the current project; `project: "*"` opts into cross-project.
- Hard result budget enforced server-side.
- Errors return empty results with a reason. Never throw, never block.
- Observations are not a recall target. The files, commands and errors they list are searchable through the turn index.
- Transport: short-lived stdio server, WAL reads. No worker, no port, no HTTP, no SSE.

`stats`, `usage`, `verify`, `reindex`, `export` are CLI only — never MCP.

### Tokenizer

Do not fight FTS5's tokenizer and do not write a custom one. **Normalize in Rust at ingest** and feed FTS5 expanded text: the original plus camel/snake/kebab components and path components.

```
SearchManager      → SearchManager, search, manager
--setting-sources  → --setting-sources, setting, sources
src/worker/S.ts    → src/worker/S.ts, src, worker, S, ts
```

Plain `unicode61` then works for both query shapes, and every expansion rule is a unit-testable Rust function. Index grows ~1.3–1.6×.

`trigram` FTS deferred until substring search proves necessary.

### Entities

Extract from **structured tool records**, not prose.

| Kind | Source | Normalization |
|---|---|---|
| `path` | Read/Edit/Write inputs, Glob/Grep paths, Bash argv | canonical absolute + repo-relative, case-folded on macOS |
| `command` | Bash tool input | `argv[0]` + significant subcommand; flags/args to a separate raw field |
| `error` | non-zero exits, stderr, panics | **strip variable parts** (line numbers, addresses, timestamps, UUIDs); keep the stable core |
| `symbol` | Edit/Write content, Grep patterns, code fences | whole identifier plus components; ≥4 chars with a case transition or underscore |
| `tool` | record field | none |

Two rules:

1. **Never reject at index time — weight by IDF at query time.** "Too common" changes as the corpus grows, and the decision would be irreversible.
2. **Cap entities per turn**, so one huge tool result can't emit thousands.

---

## 8. Context injection

### SessionStart — the resume brief

Budget: single-digit milliseconds. mmap, one query, write stdout, spawn ingest, exit 0. Any error → emit nothing, exit 0.

No query exists yet, so the signal is **continuity**, not a memory dump:

| Block | Source |
|---|---|
| Last session in this project | verbatim last prompt + last assistant turn, truncated |
| Working-state delta | git HEAD then vs now, branch, dirty files — no LLM |
| Index pointer | "N sessions, M turns indexed here; search tools available" |

That last row does disproportionate work: ~30 tokens telling the model searchable memory exists, versus claude-mem pre-loading everything.

Rules: hard token budget (~1–2k, configurable); **no volatile text** — stable ordering, no clocks, dates rounded to the day (claude-mem busts the Anthropic prefix cache every 60 seconds with minute-granularity timestamps); scoped by canonical project path; excluded projects emit nothing; no subagent turns; settings must actually reach the query.

### UserPromptSubmit — relevance

Affordable because a local FTS query is sub-millisecond — claude-mem blocks for 60 s because it does an HTTP round trip to a daemon that may not exist.

**Precision, not recall. Inject 0–3 turns; inject nothing most of the time.** Three near-misses are worse than silence.

Signal hierarchy, ranked by precision:

1. Error strings / stack frames — near-zero false positives
2. File paths mentioned or recently edited
3. Symbols / identifiers / config keys
4. Command invocations
5. Free-text BM25 — weakest, fires the most false positives

1–4 are exact matches on rare tokens, which is where BM25 beats embeddings outright.

Thresholding is structural, not a score cutoff (BM25 scores aren't comparable across queries):

- Fire on an exact rank 1–3 entity match, project-scoped.
- Fire when ≥2 independent entities co-occur in one turn.
- Otherwise stay silent. Free-text-only matches never inject.
- Cap at 3 turns and a hard token budget.
- Never inject the same turn twice in a session.
- Suppress anything already visible in this session or in the SessionStart brief.

### After a compaction

`PostCompact` can't inject, so it fires ingest only. The `compact_boundary` record identifies the dropped turns; the **next** `UserPromptSubmit` uses that set as a scoped, high-signal candidate pool with the prompt as the query. Not a bulk re-injection — a small corpus of "things this session knew ten minutes ago and no longer does".

### The feedback loop

Log every injection decision, **including non-fires** — non-fires are where miss data lives.

```
ts, session_id, turn_seq, project
entities_extracted[]   by type
candidates[]           turn_id, score, matched_on
injected[]             turn_ids
suppressed[]           turn_id + reason
thresholds_used, tokens_spent
```

Outcome extraction runs in ingest over finalized sessions, joining decisions against the transcript that followed:

| Label | Detection |
|---|---|
| hit | injected turn's path/symbol/session referenced downstream |
| false positive | injected, never referenced |
| **miss** | model called `search` for an entity that was indexed and not injected |
| wasted budget | tokens injected vs. tokens referenced |

Because the archive is verbatim and complete, **retrieval changes can be replayed against history offline** — change the extractor, re-run every logged prompt against the index as it was, diff the labels. Relevance becomes regression-testable rather than tuned by feel.

`recall stats` surfaces precision, misses, and tokens injected vs. referenced. A memory product that can prove whether it helps.

Claude-mem cannot do any of this: it discards the turns and keeps only the model's summary of them.

**OPEN — the auto-tuner.** Adjusting thresholds automatically from these labels is a *hypothesis*, not a plan. Per-user volume may be too low, "referenced downstream" is a weak proxy, and the `miss` label depends on the model choosing to search. Ship logging and replay (cheap, high value); gate the tuner behind evidence it converges.

---

## 9. Observations

**Always on.** One row per finalized session, written by the ingest pass and listed by `terminus observations`.

**Principle: a parser does this, never a model.** Claude-mem asks the LLM for `files_read`/`files_edited` and has 0 of 5,059 rows populated.

**Mechanical** (exact, free): files read/modified, tools used, commands run, errors seen, branch, commits, turn count, duration, compactions.

---

## 10. Privacy and the data boundary

Front and centre in README and docs, as a contract:

> Terminus stores your sessions unmodified on your machine. Securing your machine is your responsibility. Terminus calls no model and handles no API key. The Terminus binary opens no network connection, and downloading it with `install.sh` is the one network use. With `redact_recall` on, recall output is filtered by the shape rules before it reaches the model's context.

**Redact at egress, never at ingest.** Ingest-time redaction makes the store lossy, defeats "verbatim", and destroys what it strips. The bytes already sit in plaintext in `~/.claude/projects`, so at-rest scrubbing buys little.

**Recall redaction.** With `[privacy] redact_recall = true` (off by default), `recall_search` excerpts, `recall_context`, `recall_get`, the `SessionStart` brief and the per-prompt injection pass through the shape rules on their way into the model's context, and each rule leaves its marker where it cut. `export` writes to a path the user chose but should still state what it contains.

No encryption at rest. No telemetry. No network connections from the binary: `install.sh`'s `curl` download is the one network use.

---

## 11. Retention and exclusion

Both off by default. Keep everything.

```toml
[retention]
action      = "evict"   # keep | evict | delete
age_days    = 0         # 0 = off
max_size_gb = 0         # 0 = off

[retention.project.scratch]
age_days = 7
action   = "delete"
```

| Action | Blob | Index | Result |
|---|---|---|---|
| `keep` | kept | kept | default |
| `evict` | dropped | kept, `body_evicted` | still searchable and listed, not readable |
| `delete` | dropped | removed | gone |

Enforced at the end of every ingest pass under the lock already held: exclusions, then age, then size cap oldest-first. Bounded work per pass. Compaction reclaims space — without it the file never shrinks and retention looks broken. `--dry-run` first; `usage` reports bytes per project and month.

### Project exclusion

```toml
[projects]
exclude      = ["/data/clients", "~/work/nda"]
include_only = []          # non-empty = allowlist mode
```

Prefix match on canonicalized paths, case-insensitive on macOS. A repo-local marker file excludes a repo so client work carries its own policy. **Excluded means never read** — not ingest-then-filter. **Enforced on both ingest and injection**; claude-mem honors it on write and ignores it on read.

---

## 12. Paths and configuration

| Purpose | Linux | macOS |
|---|---|---|
| Stable link to the active version | `~/.local/bin/terminus` | same |
| Stored versions | `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus` | same |
| Data | `$XDG_DATA_HOME/terminus` | `~/Library/Application Support/terminus` |
| Config | `$XDG_CONFIG_HOME/terminus` | same convention |

- `TERMINUS_DATA_DIR` overrides. Resolved **once** at startup and passed down explicitly — never re-derived in a child.
- A `location` pointer file in the config dir means nothing hardcodes the store path; `terminus data move <path>` relocates safely.
- Transcript roots: `$CLAUDE_CONFIG_DIR` → `~/.claude`, accepting a **list**. Canonicalize — symlinked config dirs are real and would otherwise ingest twice.
- Free-space guard before writing; refuse network mounts by default.
- Config written atomically, temp + rename.

**Supported systems:** Terminus supports Linux and macOS. Windows was dropped, as it is across all of the owner's products, and a build for any other system fails at compile time with "Terminus supports Linux and macOS only."

---

## 13. Install and packaging

**There is no npm channel.** The npm package was removed. `install.sh` is the one-command installer, run as `curl -fsSL https://raw.githubusercontent.com/crenshawdev/terminus/main/install.sh | sh`. It downloads from the GitHub releases on crenshawdev/terminus, and one `https://` source replaces them when the line ends in `sh -s -- https://<source>` instead of `sh`. It refuses any release whose signature does not verify against the crenshawdev release key pinned in the script. No release is published yet.

Secondary: Homebrew, deb/rpm.

**No Claude Code plugin in 0.x.** The plugin system installs into versioned directories, which is the root cause of claude-mem's inline path-resolution script and its version-check hook. Revisit at 1.0; if it ships, it stays a thin bootstrap.

### Binary layout

`~/.local/bin/terminus` is a symbolic link to `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, one folder per version. It is the layout Baley uses, which is why the path carries the `crenshawdev` folder. The hooks and the MCP registration name the link, so they are written once and the version behind them changes without touching them. No PATH modification.

The versions folder sits under HOME and outside the data directory. The data directory moves with `TERMINUS_DATA_DIR` and `terminus data move`, and `terminus uninstall --purge` deletes it, which would delete the file every hook runs.

A version gets there one of two ways:

- `install.sh` downloads `terminus-<os>-<arch>.manifest` and `terminus-<os>-<arch>.manifest.sig` for the machine, from the GitHub releases by default or from the given source, and verifies the signature before it reads the manifest (see Release signature below). Only then does it download `terminus-<os>-<arch>`, check the binary's SHA-256 in `verify_download`, stage it in the folder for the manifest's version through a temporary file and `mv`, and move a temporary link over the stable path with `mv -f`. It ends by running `~/.local/bin/terminus install`.
- `terminus install` run from anywhere else stores its own binary under its own version and points the link at it. A local build is installed with `cargo build --release && target/release/terminus install`. Run from the file the link already names, as `install.sh`'s last step is, it changes neither the link nor the file.

Both replace what is at the stable path only when nothing is there or it carries the Terminus marker, a fixed string every build holds, directly or through a link, so a plain-file install from before the link becomes the link. A link to nothing counts as nothing to `terminus install`, and to `install.sh` only when it points into the versions folder. Anything else is refused and left as it is.

A version folder that already holds the same bytes is left alone. One holding different bytes, which is what a rebuild at the same version finds, gets the new file renamed over it, and a process running the old file keeps its inode. Old version folders are never deleted and nothing prunes them, so a running session's MCP server and the ingest its hook spawned keep their file.

Uninstall removes the link and leaves the versions folder, printing its path.

### Release signature

A release's manifest carries an OpenSSH signature (SSHSIG) made with the crenshawdev release key, an Ed25519 key, under the namespace `terminus-release`. `install.sh` holds the key's public line once, as a literal. It writes the allowed-signers line `crenshawdev-release <pinned key>` from that literal into its `mktemp -d` folder, then runs `ssh-keygen -Y verify -f <that file> -I crenshawdev-release -n terminus-release -s <manifest.sig>` with the manifest on stdin. Nothing is read from HOME or `~/.ssh`, so a key listed in `~/.ssh/allowed_signers` counts for nothing. Any non-zero exit is a refusal, and the binary is never downloaded. The script does not check `ssh-keygen`'s version, so one too old for `-Y` fails closed.

`ssh-keygen`'s output is dropped, and the script prints its own reason on stderr. A missing signature is a download failure, `signature download failed: <url>`. A signature made by another key or under another namespace, a manifest changed after signing, and anything else `ssh-keygen` refuses all print `the manifest does not verify against the pinned crenshawdev release key`.

`ssh-keygen` is required. The script checks for it with `command -v` before any download, and when it is missing names `openssh-client` (Debian, Ubuntu), `openssh-clients` (Fedora) and `openssh` (Arch).

Nothing overrides the key. It is never fetched, and no argument, setting or environment variable changes it, so every source, the `https://` argument included, must serve a signature made with it. There is no unsigned mode. An update is a rerun of the same command, so it gets the same check.

### `terminus install`

Install asks nothing. It applies its defaults, never reads stdin and takes no arguments; any argument is misuse and exits 2.

1. Detect and canonicalize config roots; report if more than one resolves.
2. Merge the four hook entries into the existing `hooks` object of `settings.json`: atomic temp + rename, idempotent, with a one-time backup of the file as it was before install first changed it. A file not in the two-space format `JSON.stringify(value, null, 2)` writes is rewritten in full, and the backup holds the original bytes.
3. Place the MCP entry with `claude mcp add-json --scope user terminus '<JSON>'`: stdio, the stable path as `command`, and `args` of `["mcp"]`. Install reads `.mcpServers.terminus` in `~/.claude.json` and never writes that file, so Claude Code stays its only writer.
   - An entry already in that shape runs no `claude`. An entry in any other shape is removed with `claude mcp remove --scope user terminus` and then added again. Keys the user added beside the three, such as `env`, are carried into the new entry.
   - `claude` is found on `PATH` and run by absolute path, with no shell, stdin at null, a 30-second deadline and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, so wiring Terminus up opens no network connection.
   - When `claude` is missing, times out or exits non-zero, or `~/.claude.json` cannot be read, the install is incomplete. The binary, the hooks and the backfill still go ahead, the receipt prints the exact command that finishes the registration, and install exits non-zero.
4. Print the receipt: every file changed (the binary, `settings.json` and its backup, the MCP entry), where data lives, the config roots, and how to undo it with `terminus uninstall`. It also shows the current `cleanupPeriodDays`, says that transcripts older than it are already gone, and prints the line that raises it, then gives the auto-compact recommendation. Install changes neither setting.
5. Last, estimate the backfill (sessions, size, time), add the estimate and `terminus status`, where to watch it, to the end of the receipt, and run it detached, chunked, resumable, with bounded parallelism. Install returns immediately.

Hooks go in **user scope** — terminus is global, one store across all projects.

### Upgrade

Installing a new version, with `install.sh` or with the new binary's `terminus install`, moves the link to that version's file. **Hooks are never rewritten** — the path doesn't change, so there is no version-check hook and nothing to repair. Store format bumps rebuild derived tables only. Config is additive.

### Uninstall

Removes only what it added, restores the settings backup if otherwise unchanged, **leaves the data** and prints where it is. `--purge` deletes it after showing size and confirming. Works even if the config is partly broken.

### `terminus doctor`

Read-only. Never repairs. Every problem it names carries the exact command that fixes it.

Checks: binary at the stable path and version; hook entries present and pointing there; MCP registered; config roots resolved; data dir writable, free space; store openable, format version, integrity; last ingest run and error; effective `cleanupPeriodDays`; effective `autoCompactEnabled` across env and all three settings scopes.

### Auto-compact

Recommend disabling; never change it. With terminus, compaction burns tokens to produce a lossy summary of context already stored losslessly — a fresh session plus targeted recall beats a self-summarized one. Surfaced in README, in `doctor`, and as one line in the install summary. Key is `autoCompactEnabled` (default `true`, any settings scope); also `DISABLE_AUTO_COMPACT=1` and `autoCompactWindow`.

---

## 14. CLI

| Command | Does |
|---|---|
| `terminus status` | sizes, counts, watermarks, last ingest run and error |
| `terminus search` / `show` / `sessions` | terminal recall |
| `terminus ingest` | manual catch-up |
| `terminus reindex` | rebuild derived tables from the block rows |
| `terminus verify` | check every block against its hash and each session checksum, name the session and block of a failure |
| `terminus compact` | reclaim freed space |
| `terminus usage` | bytes per project and month |
| `terminus data move <path>` | relocate the store safely |
| `terminus export` | portable output for backup or migration |
| `terminus doctor` | read-only health report |
| `terminus install` / `uninstall` | wiring |
| `terminus observations regenerate [--since]` | recompute stored observations from the archived block rows |
| `terminus mcp` | stdio MCP server |

Contract: human-readable by default, `--json` on every data command with a stable shape. Errors to stderr, data to stdout. Exit `0` success, `1` operational failure, `2` misuse — never non-zero for "no results". No colour when stdout isn't a TTY; honour `NO_COLOR`. Quiet unless attached to a TTY.

**No log file.** Ingest is detached, so its failures would otherwise be invisible — solved with the `runs` table, surfaced by `status`. A log file is debug-only, off by default, enabled with `--verbose`.

---

## 15. Testing

| Layer | What |
|---|---|
| Unit | tokenizer expansions, entity extraction, path canonicalization, block framing, watermark math, retention evaluation, egress redaction |
| Real-corpus parse | all 1,896 transcripts: zero panics, zero unparseable lines, snapshot extracted structure. Catches Anthropic format drift the day it ships |
| Property | round-trip compress/decompress, block reads at arbitrary offsets, watermark resume from arbitrary truncation, supersede ordering |
| Crash / durability | kill ingest at random points; assert consistency and convergence on rerun; truncate transcripts mid-line |
| Concurrency | N processes race the lock — exactly one works, the rest exit 0 in ms; readers during writes |
| Hook contract | feed the exact JSON Claude Code sends; assert exit 0, bounded wall time, clean stdout, fail-open when the DB is missing |
| Retrieval eval | labeled (query → expected turn) set; precision@k as a regression suite via the replay harness |
| Platform CI | Linux / macOS matrix actually exercising hooks and locking |
| Startup benchmark | assert hook cold start under budget. Startup time *is* the product |

Fixture data can't be committed — transcripts are private. Small synthetic/redacted set in-repo for CI, plus a local-only corpus test pointed at the real tree via env var.

**Claude-mem's issue tracker is a free test plan.** One regression test per failure class designed out: no shell spawn in the hook, no inherited handles on the detached child, exclusions honored on the read path, settings reaching the query, path canonicalization across symlinks.

No mocking SQLite — real temp databases throughout.

---

## 16. Versioning, licensing, repo

- **0.0.1 onward. 1.0 only when the full feature set has landed.** Agile increments.
- **Store format version is a separate integer** in `meta`. The binary refuses a store whose archive format differs from its own, newer or older, and a derived schema that differs from its own rebuilds the derived tables. A product bump never implies a store rebuild.
- **Apache-2.0.** Source must be public — nobody installs a closed binary that reads every keystroke of their work. Apache over MIT for the explicit patent grant. AGPL is a documented negative signal in this category and its network clause protects nothing here. All dependencies are permissive (SQLite public domain, rusqlite MIT, zstd BSD, rayon MIT/Apache).
- **Public OSS** — an explicit override of the private-by-default rule, scoped to this project.
- `origin` is the self-hosted repo and the only remote. Publishing elsewhere is handled outside this project.
- crates.io `terminus` is taken. Skip crates.io; how Terminus is delivered is still open.

---

## 17. Open items

| Item | Status |
|---|---|
| Auto-tuner for injection thresholds | Hypothesis. Ship logging + replay; gate the tuner behind evidence it converges |
| Import of `/data/verbatim-legacy` (801 MB) | Deferred |
| Other harnesses — Codex, Cursor, OpenCode | Out of scope for 0.x |
| Multi-machine sync | Out of scope, stated so it doesn't creep in |
