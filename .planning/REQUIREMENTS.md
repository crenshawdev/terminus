# Requirements: Verbatim

**Defined:** 2026-08-12
**Core Value:** The verbatim transcript is the record: every session archived losslessly and permanently, everything else derived, rebuildable, and measurable against it.

Scope comes from `DESIGN-BRIEF.md`, where every decision is settled unless marked OPEN. v1 here means 1.0 — the brief's own rule is "0.0.1 onward, 1.0 only when the full feature set has landed", so the Active set is the full feature set and the increments are phases, not scope cuts.

## Active

Committed scope for the open cycle. Each maps to exactly one roadmap phase.
Every v0.1.0 requirement shipped and was verified — see `## Shipped` below for
all 64 ids with the phase that delivered each.

### Injection (INJ)

- **INJ-07**: The resume brief's quoted prompt is a turn the user typed. A `user` record carrying a tool result is not eligible to be quoted under "It last asked", the typed/tool-result discriminator is stored at ingest rather than derived by decompressing turns on the cold-start path, and a store written before this build acquires it by backfill from blobs alone. Closes the gap between `DESIGN-BRIEF.md:230`'s "last prompt" and `inject/brief.rs`'s `record_type = 'user'`, which is the transcript's own `type` field and therefore matches both.

## Shipped

Delivered and verified. Kept as rows for shipped-scope trace; git history
holds the full requirement text. Archived out of `## Traceability` so a new
milestone's audit starts clean (the audit seam parses only the Traceability
table).

| Requirement | Phase | Status | Milestone |
|-------------|-------|--------|-----------|
| STOR-01 (A session's turns are stored as one block-framed zstd blob, and reading a single turn decompresses only the 64 KB blocks that turn occupies.) | 1 | Complete | v0.1.0 |
| STOR-05 (The binary refuses to open a store whose format version is newer than it knows, and rebuilds derived tables — never the archive table — when the format is older.) | 1 | Complete | v0.1.0 |
| STOR-02 (An ingest commits blob, turn rows, FTS rows, entity rows and the watermark in a single transaction, so a killed process leaves no partially indexed session.) | 1 | Complete | v0.1.0 |
| STOR-03 (`verbatim verify` walks every blob, checks its per-blob checksum, and names the session ids that fail rather than declaring the store bad.) | 1 | Complete | v0.1.0 |
| STOR-04 (`verbatim reindex` rebuilds every derived table from blobs alone and produces the same query results as before the rebuild.) | 1 | Complete | v0.1.0 |
| ING-01 (`verbatim ingest` tails each transcript from its stored byte offset to the last complete record, and resumes correctly when the previous run stopped mid-line.) | 1 | Complete | v0.1.0 |
| ING-02 (A second ingest launched while one is running exits 0 within milliseconds instead of waiting, and exactly one process does the work.) | 1 | Complete | v0.1.0 |
| ING-03 (Ingest performs its recovery at the top of every run, with no repair command and no external supervisor, and a rerun after a kill at any point converges to a consistent store.) | 2 | Complete | v0.1.0 |
| ING-04 (Ingest discovers nested subagent sidecar transcripts and links sessions that continue across files into one thread.) | 2 | Complete | v0.1.0 |
| ING-08 (An excluded project is never read rather than read-then-filtered, and the exclusion is honored on the ingest path and every read path alike.) | 2 | Complete | v0.1.0 |
| ING-09 (`verbatim status` reports sizes, counts, watermarks and the last ingest run with its error, sourced from the `runs` table rather than a log file.) | 2 | Complete | v0.1.0 |
| ING-05 (A session's project is the canonical git toplevel derived from the record's `cwd`, with worktrees mapped to the parent repo and both keys stored; the encoded directory name and `basename()` are never used for identity.) | 2 | Complete | v0.1.0 |
| ING-06 (A compaction appended to a live transcript is ingested as a boundary carrying the record's compaction metadata verbatim. The set of turns that fell out of the model's context is derived from that metadata at query time (INJ-05, Phase 5) - the metadata describes the preserved segment, so the complement is a query and not a stored column (Phase 2 D-08).) | 2 | Complete | v0.1.0 |
| RCL-01 (Turn text is expanded in Rust at ingest into its camel, snake, kebab and path components, so a search for `SearchManager` and a search for `manager` both find the same turn through plain `unicode61` FTS5.) | 3 | Complete | v0.1.0 |
| RCL-02 (Entities of kind path, command, error, symbol and tool are extracted from structured tool records, with per-kind normalization, rather than from prose.) | 3 | Complete | v0.1.0 |
| RCL-03 (Error entities are normalized by stripping line numbers, addresses, timestamps and UUIDs, so the same error recurring in a later session matches the earlier one.) | 3 | Complete | v0.1.0 |
| RCL-04 (No entity is rejected at index time; commonness is handled by IDF weighting at query time, and the number of entities emitted per turn is capped.) | 3 | Complete | v0.1.0 |
| RCL-05 (`verbatim search`, `verbatim show` and `verbatim sessions` give terminal recall over the archive with project scoping and filters.) | 3 | Complete | v0.1.0 |
| RCL-07 (`recall_search` returns ranked turns filtered by project, paths, tool, kind and time window, with id, session, timestamp, project and excerpt per hit.) | 3 | Complete | v0.1.0 |
| RCL-08 (`recall_context` returns the chronological turns around a given hit.) | 3 | Complete | v0.1.0 |
| RCL-06 (Every data command accepts `--json` and emits a stable shape, sends data to stdout and errors to stderr, and exits 0 on success, 1 on operational failure and 2 on misuse, never non-zero for an empty result.) | 3 | Complete | v0.1.0 |
| RCL-09 (`recall_get` returns full verbatim text for a list of ids and flags any body that was evicted.) | 3 | Complete | v0.1.0 |
| RCL-10 (All three MCP tools are marked read-only, auto-scope to the current project with `project: "*"` opting into cross-project, enforce a server-side result budget, and return empty results with a reason instead of throwing.) | 3 | Complete | v0.1.0 |
| RCL-11 (The MCP server is a short-lived stdio process reading through WAL, with no port, no daemon, no HTTP and no SSE.) | 3 | Complete | v0.1.0 |
| ING-10 (Each of the four hooks spawns a fully detached ingest process and returns 0 immediately, invoking the binary by absolute path with no shell and no inherited handles.) | 4 | Complete | v0.1.0 |
| INST-02 (Install copies the platform binary to the canonical stable path and points both hook entries and MCP registration at that copy, without modifying PATH.) | 4 | Complete | v0.1.0 |
| INST-03 (Install shows the exact diff for each file it changes (`~/.claude/settings.json` and `~/.claude.json`), backs each one up, confirms once, merges into the existing objects atomically, and produces no second entry when rerun.) | 4 | Complete | v0.1.0 |
| INST-04 (Install offers to raise `cleanupPeriodDays` when it is low and prints the auto-compact recommendation, without changing either setting itself.) | 4 | Complete | v0.1.0 |
| INST-05 (An upgrade replaces the binary and re-copies it to the stable path without rewriting a single hook entry.) | 4 | Complete | v0.1.0 |
| INST-08 (`--yes` accepts every default so install runs unattended in a script.) | 4 | Complete | v0.1.0 |
| INST-06 (`verbatim doctor` is read-only, never repairs, and prints the exact command that fixes each problem it reports.) | 4 | Complete | v0.1.0 |
| INST-07 (Uninstall removes only what install added, restores the settings backup when the file is otherwise unchanged, leaves the data and prints where it is, and deletes it under `--purge` only after showing its size and confirming.) | 4 | Complete | v0.1.0 |
| ING-11 (Backfill estimates sessions, size and time up front, then runs detached, chunked, resumable and with bounded parallelism.) | 4 | Complete | v0.1.0 |
| INST-01 (`npx verbatim install` runs from a thin npm package with per-platform optional dependencies and no postinstall script.) | 4 | Complete | v0.1.0 |
| INJ-01 (SessionStart emits a resume brief covering the last session in this project, the branch that session ended on, the index pointer and observations when enabled, inside its token budget and a single-digit-millisecond wall budget. The working-state delta is branch-only because `session_meta.branch` is the sole git fact the archive holds and a `git` subprocess costs 10-30 ms against a single-digit-millisecond budget (Phase 5 D-10).) | 5 | Complete | v0.1.0 |
| INJ-06 (Any injection failure emits nothing and exits 0 inside the deadline, so a missing, locked or corrupt store never blocks a prompt.) | 5 | Complete | v0.1.0 |
| INJ-02 (The resume brief contains no volatile text — stable ordering, dates rounded to the day — so unchanged state produces byte-identical output across runs and does not bust the prefix cache.) | 5 | Complete | v0.1.0 |
| INJ-03 (UserPromptSubmit injects between 0 and 3 turns, firing only on a structural threshold (a rank 1–3 exact entity match, or two or more independent entities co-occurring in one turn), and never on a free-text-only match.) | 5 | Complete | v0.1.0 |
| INJ-04 (A turn already injected this session, already visible in the session, or already carried by the resume brief is suppressed rather than injected again.) | 5 | Complete | v0.1.0 |
| INJ-05 (After a compaction, the next prompt draws its candidates from the turns that fell out of context, scoped and capped, rather than bulk re-injecting them.) | 5 | Complete | v0.1.0 |
| FEED-01 (Every injection decision is logged, non-fires included, with the entities extracted, candidates scored, turns injected, turns suppressed with reasons, thresholds used and tokens spent.) | 6 | Complete | v0.1.0 |
| FEED-02 (Ingest labels decisions from finalized sessions as hit, false positive, miss or wasted budget by joining them against the transcript that followed.) | 6 | Complete | v0.1.0 |
| FEED-03 (A replay harness re-runs every logged prompt against the index as it stood, so a change to extraction or thresholds is diffed against history offline instead of tuned by feel.) | 6 | Complete | v0.1.0 |
| FEED-04 (`verbatim stats` reports injection precision, misses, and tokens injected versus tokens referenced.) | 6 | Complete | v0.1.0 |
| OBS-01 (Mechanical observations — files read and modified, tools used, commands run, errors seen, branch, commits, turn count, duration, compactions — are parser-derived and always available.) | 7 | Complete | v0.1.0 |
| OBS-07 (`verbatim observations regenerate` rebuilds derived observations selected by `--since` or `--prompt-version`.) | 7 | Complete | v0.1.0 |
| OBS-05 (One provider block of base URL, model and key serves local, OpenRouter and any OpenAI-compatible endpoint through a single code path, with Anthropic subscription auth as a separate branch. - Phase 7 note (2026-08-22): the OpenAI-compatible half is delivered and the single code path is intact. Anthropic subscription OAuth - the "separate branch" - is DEFERRED out of phase 7 (CONTEXT D-05): no OAuth flow, refresh or storage is described anywhere in the repo and there is no Anthropic key on this machine to prove it against. Add it as its own phase via /cad-phase. A fourth key, `response_format`, exists for an endpoint whose structured-output support is narrower than `json_schema` (measured against `deepseek-chat`); it selects one field's value, not a second request shape, so "single code path" holds. See phase 7 AC4 as amended.) | 7 | Complete | v0.1.0 |
| PRIV-01 (Redaction happens at egress and is keyed on destination — a remote provider is filtered, a local provider is not egress at all — and never at ingest.) | 7 | Complete | v0.1.0 |
| PRIV-02 (Credentials load from the shared per-provider file with permissions enforced and load refused when they are too open, following precedence process env, then product config, then shared file, and their values never reach logs, errors or output. - Phase 7 note (2026-08-22): the permission check is a Unix mode-bit test via `PermissionsExt`. Windows ACL enforcement is DEFERRED (CONTEXT D-15); the Windows arm accepts with a caveat surfaced in `doctor`. Complete on Unix only.) | 7 | Complete | v0.1.0 |
| PRIV-03 (The binary opens no network connection except to the configured model provider, and emits no telemetry of any kind.) | 7 | Complete | v0.1.0 |
| OBS-02 (LLM judgment is opt-in and off by default, costs one call per finalized session, and returns strict JSON against a fixed schema.) | 7 | Complete | v0.1.0 |
| OBS-03 (Every generated claim carries a `turn_id` anchoring it to a verbatim turn in the archive.) | 7 | Complete | v0.1.0 |
| OBS-04 (A parse failure retries once and then stores the raw response with `status = parse_failed`; it is never dropped silently and never blocks ingest.) | 7 | Complete | v0.1.0 |
| OBS-06 (Cost controls hold: sessions under N turns are skipped, input is truncated with explicit elision markers, a daily token budget applies, and there is never more than one call per session.) | 7 | Complete | v0.1.0 |
| OBS-08 (Observations are reachable through the existing recall tools as a `kind` filter rather than through a fourth tool.) | 7 | Complete | v0.1.0 |
| RET-01 (Retention is off by default, and `keep`, `evict` and `delete` can be set globally or per project.) | 8 | Complete | v0.1.0 |
| RET-03 (Retention runs at the end of an ingest pass under the lock already held, does bounded work per pass, and `--dry-run` reports what it would do before anything is applied.) | 8 | Complete | v0.1.0 |
| RET-02 (An evicted session stays searchable and listed with its body flagged as evicted, while a deleted one is gone from both blob and index.) | 8 | Complete | v0.1.0 |
| RET-04 (`verbatim compact` reclaims freed space so the store actually shrinks after retention.) | 8 | Complete | v0.1.0 |
| RET-05 (`verbatim usage` reports bytes per project and per month.) | 8 | Complete | v0.1.0 |
| PRIV-04 (`verbatim export` produces portable output for backup or migration and states what that output contains.) | 8 | Complete | v0.1.0 |
| STOR-06 (Rolling snapshots run by default and produce a consistent copy of the store without stopping ingest.) | 8 | Complete | v0.1.0 |
| STOR-07 (`verbatim data move <path>` relocates the store and updates the location pointer, so no component holds a hardcoded store path.) | 8 | Complete | v0.1.0 |
| ING-07 (Capture mode (`full`, `lean`, `minimal`) controls how much of each record is stored, and every elision is marked in the stored record.) | 8 | Complete | v0.1.0 |

## v2 Requirements

Deferred. Tracked, not in the current roadmap.

### Retrieval

- **Auto-tuner for injection thresholds**: adjust thresholds automatically from the outcome labels. Hypothesis, not a plan — gated behind evidence from FEED-02/03 that it converges.
- **Trigram FTS index**: add only once substring search proves necessary in practice.

### Data

- **Import of `/data/verbatim-legacy`** (801 MB): deferred until the archive format has settled.

### Distribution

- **Secondary channels**: GitHub releases, Homebrew, Scoop/WinGet, deb/rpm. npm is the primary channel and ships first.
- **crates.io publication**: name `verbatim` is taken; would ship as `verbatim-cli` with `[[bin]] name = "verbatim"`. Real channels are npm and releases.

## Out of Scope

Explicit exclusions. The reason prevents scope creep later.

| Feature | Reason |
|---------|--------|
| Multi-machine sync, teams, auth, cloud | Local-first product; stated explicitly so it does not creep in |
| Harnesses other than Claude Code in 0.x (Codex, Cursor, OpenCode) | Each is a parser to add later, not an integration to design around now |
| Web viewer or HTTP server | No daemon, no port, no long-lived process is a design invariant |
| Telemetry of any kind | Privacy contract |
| Daemon or other long-lived process | The hook spawn is the scheduler; eliminates the orphaned-process failure class |
| Encryption at rest | Source transcripts already sit in plaintext in `~/.claude/projects`; at-rest scrubbing buys little |
| Ingest-time redaction | Makes the store lossy, defeats "verbatim", and destroys what it strips; redaction is egress-only |
| Custom FTS5 tokenizer | Normalizing in Rust at ingest keeps every expansion rule a unit-testable function |
| `repair` command or external supervisor | Recovery lives in the binary at the top of every ingest run; OS locks die with the process |
| Second index store (Tantivy and similar) | Second commit and divergence handling; FTS5 is adequate at this corpus size |
| Claude Code plugin in 0.x | Versioned plugin directories are the root cause of the incumbent's path-resolution and version-check hooks; revisit at 1.0 |
| `stats`, `usage`, `verify`, `reindex`, `export` as MCP tools | Every tool description sits in every session's context forever; these are CLI only |
| Meta-tool describing the other recall tools | Three tools with one-line descriptions; the workflow is implied by the return shapes |
| Log file on by default | Ingest is detached, so the `runs` table plus `status` is the visible record; a log file is debug-only behind `--verbose` |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|

Bare headers — `/cad-plan` seeds a row per requirement when its phase is planned.

---
*Last updated: 2026-08-12 after project initialization*
