# `--json` shapes and the exit-code contract

Every data command accepts `--json`. This file is the contract.
`crates/terminus/tests/cli.rs` holds every command but `doctor` to it, one test
per property across all of them, so a command added later is one line in that
test's command list rather than a new file. `crates/terminus/tests/doctor.rs`
holds `doctor`'s shape.

## The envelope

Every `--json` run writes exactly one JSON document on one line to stdout, and
nothing else to stdout, ever.

```json
{"command": "search", "ok": true, "reason": null, "data": {}}
```

| Field | Type | Meaning |
|---|---|---|
| `command` | string | The subcommand that produced this document. |
| `ok` | bool | `true` when the process exits 0, `false` when it exits 1. |
| `reason` | string or null | Why a result is empty, or why the command failed. Always present; null when there is nothing to say. |
| `data` | object | The command's own answer. The only part whose shape differs between commands. |

`reason` is prose rather than a code. In-process callers already have
`recall::Reason` as a value they can match on; across the CLI boundary the
consumer is a person reading a terminal or a script checking `ok`, and a second
vocabulary of reason codes would be a contract to keep stable for nobody.

Every value is built and serialized by `serde_json`, never assembled with
`format!`. A transcript excerpt carries quotes, backslashes and control bytes,
and a hand-rolled writer fails on exactly the turns most worth reading.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success, **including an empty result set**. A query that matched nothing is a successful answer to a question with no matches. |
| 1 | Operational failure: the work was attempted and did not succeed. `verify` found a corrupt block, `reindex` skipped a session, the store could not be read, the store is in an archive format this build refuses. |
| 2 | Misuse: the command line does not name work that could be attempted. An unknown flag, an unknown subcommand, a non-numeric turn id, a malformed `--since`. |

A caller that parses `ok` and a caller that checks the exit code never
disagree. `verify --json` on a damaged store writes its document **and** exits
1; the document is the evidence and the code is the verdict.

**One case writes no document:** a failure raised before the command reached its
own answer - a store in an archive format this build refuses, an unresolvable
data directory - is reported on stderr with exit 1 and stdout stays empty. The
command never got far enough to know `--json` had been asked for. Empty stdout with a non-zero exit is
therefore part of the contract, and a caller must check the exit code before it
parses.

## Streams

Data on stdout, every diagnostic on stderr, on every path. That is what lets
`terminus search --json | jq` work while a warning is still printed.

With `--json` the document **is** the answer, so routine commentary moves into
it rather than being printed twice: `verify`'s "N session(s) checked" is the
document's `checked`, and a clean `--json` run writes nothing at all to stderr.
Two accounts of one walk that can disagree is exactly what one shape exists to
prevent.

stderr is left for warnings and irregularities, which are still printed in JSON
mode - a store whose derived tables predate this build says so on stderr and
still writes only the document to stdout.

Without `--json` the split is unchanged from what phase 1 and 2 shipped: the
data on stdout, the counts and the warnings on stderr.

## `data` by command

### `search`

```json
{"query": "src/worker/S.ts", "truncated": false, "hits": [
  {"turn_id": 42, "session_key": "/abs/path/session.jsonl", "project": "/abs/path/repo",
   "record_type": "assistant", "ts": "2026-08-12T21:00:00.000Z", "sidechain": false,
   "relevance": 1.83, "entity_score": 0.0,
   "matched_on": [{"kind": "path", "value": "src/worker/S.ts"}],
   "excerpt": "the retry budget lives in ..."}]}
```

- `query` is the string as typed, not the tokenization of it.
- `truncated` is true when the query carried more than `MAX_QUERY_TOKENS`
  distinct tokens. The search still ran; the tokens are conjoined, so a
  truncated query is broader than the one asked for rather than wrong in a
  direction the caller cannot see.
- `ts` and `project` are null for a turn or a session that has none.
- `relevance` is higher-is-better. `entity_score` is the part of it that came
  from an exact structural match rather than from free text.
- `matched_on` names the distinct stored `(kind, value)` entity pairs this query
  matched on this turn, in sorted order: the facts behind `entity_score`, since
  a score alone cannot say whether a path, a command or a symbol produced it. It
  is `[]` for a hit reached only through free text, and its length is the entity
  count the injection threshold reads. `value` is the normalized stored
  spelling, not the query's.
- `excerpt` is cut from the session's block rows, never from FTS5, and may be
  empty when the archive would not give the bytes up.
- `--kind` takes `user`, `assistant`, `system` or `attachment`. `observation`
  is refused, as misuse (exit 2) from `terminus search` and as `isError` from
  `recall_search`, because Terminus stores no session summaries.

### `show`

```json
{"records": [
  {"turn_id": 42, "session_key": "...", "turn_seq": 1, "record_type": "assistant",
   "tool_name": null, "ts": "...", "project": "...",
   "body": "{\"type\":\"assistant\",...}", "body_evicted": false, "context": null}],
 "absent": [{"turn_id": 99, "reason": "no turn 99 is archived"}]}
```

- `body` is the record's own archived line. It is the only lossy rendering
  terminus performs: a JSON string is text by definition, so a byte that is not
  UTF-8 is replaced here. The human mode writes the bytes themselves. With
  `[privacy] redact_recall = true` in `terminus.toml` the line is put through the
  egress filter first, so `body` carries `[redacted]` markers where a credential
  stood; the archive itself is unchanged, and no key is added or removed in
  either setting.
- `body_evicted` comes from `session_meta.is_evicted` and from nothing else. A
  block that will not decompress is archive damage, which `terminus verify`
  reports; it leaves `body` null with `body_evicted` false.
- `context` is null unless `--before` or `--after` was asked for. When present:

```json
{"turns": [{"turn_id": 41, "turn_seq": 0, "record_type": "user", "tool_name": null,
            "ts": "...", "text": "...", "is_anchor": false}],
 "at_session_start": true, "at_session_end": false,
 "continues_from": null, "reason": null}
```

  `at_session_start` / `at_session_end` say which end the window stopped at. The
  window never crosses a session boundary, and `continues_from` is handed back
  unfollowed so a caller can decide to make a second call.
- `absent` carries every requested id that named no record this caller may see:
  an unknown id, a turn of an excluded project, or a turn outside the scope.

### `sessions`

```json
{"sessions": [
  {"session_key": "...", "session_no": 3, "project": "...", "project_pre_worktree": null,
   "branch": "main", "first_turn_at": "...", "last_turn_at": "...", "turns": 47,
   "watermark": 12034, "sidecar": false, "evicted": false}]}
```

- `sidecar` is true when the session has a `parent_session_key`: it is a
  subagent transcript.
- An excluded project's sessions are absent, including sessions archived before
  the exclusion was configured.

### `status`

```json
{"store": "/abs/path/terminus.db", "size_bytes": 106496, "sessions": 11, "turns": 47,
 "watermarks": 11, "watermark_bytes": 34012, "excluded": ["/abs/path/private"],
 "last_run": {"started_at": "...", "duration_ms": 12, "files_seen": 11,
              "files_committed": 11, "files_failed": 0, "bytes_read": 34012,
              "turns_added": 47, "error": null, "notes": null}}
```

- The same numbers the human output prints. `size_bytes` counts the database and
  its WAL sidecars together.
- `last_run` is null on a store no pass has ever written to.
- `error` holds what went wrong on that pass, one line each: a file skipped, a
  watermark repaired, a step that could not run. `notes` holds what a healthy
  pass did worth saying, such as labels written and sessions retention evicted.
  Each is null when there is nothing to say, and `doctor` flags only `error`.

### `verify`

```json
{"checked": 11, "failures": [{"session_key": "...", "block": 3, "detail": "..."}]}
```

- Each failure carries `block`: the number of the failing block, or `null` for a
  failure about the whole session, such as a session checksum that no longer
  matches the block hashes while every block matches its own hash.
- Without `--json`, each failure is one line on stdout,
  `<session_key>: block N: <detail>`, or `<session_key>: <detail>` when `block`
  is `null`.

Exits 1 when `failures` is non-empty, with the document still written.

### `reindex`

```json
{"sessions": 11, "turns": 47, "skipped": [{"session_key": "...", "reason": "..."}]}
```

Exits 1 when `skipped` is non-empty - every undamaged session was still rebuilt,
and the exit code is what tells a script the store is not whole. A run refused
because another process holds the ingest lock is `ok: false` with the lock named
in `reason`, not an empty stdout a caller could not tell from a crash.

Without `--json`, `reindex` writes nothing at all to stdout. That is deliberate
and unchanged.

### `stats`

```json
{"decisions": 12, "injected_turns": 9, "hits": 5, "false_positives": 4,
 "precision": 0.56, "misses": 0, "wasted_budget": 2,
 "chars_injected": 1840, "chars_referenced": 1020}
```

- `decisions` counts **every** logged prompt, including the ones that injected
  nothing and the ones that never opened the store. That is the denominator the
  whole report is for: a precision computed only over the prompts that fired
  would be a number about a subset that flatters itself.
- `precision` is `hits / (hits + false_positives)`, and it is **null**, not
  zero, when no injected turn has been labelled yet - an archive whose sessions
  are all still open has no precision, and zero would read as "injection never
  helps".
- `misses` counts decisions where the model went to recall for something the
  prompt had named and the injector declined to hand over. It reads zero on real
  history today: no `recall_search` call exists in any measured transcript.
- `chars_injected` is what was spent; `chars_referenced` is the part of it
  carried by turns that turned out to be hits. **Characters, never tokens** -
  the same proxy the injection budget is spent in.
- A label is only ever written for a session the idle rule has closed, so every
  count here lags the live sessions by that threshold.

### `replay`

```json
{"thresholds": {"ranked": 10, "compacted_ranked": 50, "entity_rank": 3, "co_occurring": 2,
                "max_turns": 3, "max_candidates": 8},
 "decisions": 12,
 "labels": [{"label": "hit", "old": 3, "new": 5},
            {"label": "false positive", "old": 4, "new": 2},
            {"label": "miss", "old": 0, "new": 0},
            {"label": "wasted budget", "old": 2, "new": 1}],
 "changed": [17, 41]}
```

- `thresholds` is the six numbers the replay ran under: the compiled-in values
  with whatever `--ranked`, `--compacted-ranked`, `--entity-rank`,
  `--co-occurring`, `--max-turns` and `--max-candidates` moved. A diff without
  them is unreadable, so they are present whether or not a flag was passed.
  **They reach this report and nothing else** - the live injection path compiles
  its thresholds in and no config key or flag detunes it.
- `decisions` counts the rows that were scored: the decisions of sessions the
  idle rule has closed, which is exactly the set the labeller was allowed to
  judge. A decision of a live session has no stored label to diff against.
- `labels` is one entry per label, all four in that order, whenever replay ran
  against the decision log, even one with no decision to score. With no store,
  or a store with no decision log, `labels` is `[]`, and it is `[]` in the
  failure document for a damaged log too. `old` is what the store holds, `new`
  is what the replayed rules produce.
- `changed` names the decision ids whose label set moved, in id order, so a diff
  can be followed back into the prompts behind it.
- `replay` never writes: the store is opened read-only, `terminus.db` is byte
  for byte what it was, and the WAL carries no frame afterwards. A first
  read-only open of a WAL database does materialize SQLite's shared-memory
  index (`terminus.db-shm`) and an empty `terminus.db-wal`; those hold no page
  of the database, every reader needs them, and a read-only connection cannot
  unlink them on close.

### `observations`

```json
{"observations": [
  {"session_key": "/home/you/.claude/projects/-code-verbatim/1111....jsonl",
   "session_id": "1111...", "generated_at": "2026-08-21T18:31:07.412Z",
   "mechanical": {"files_read": ["..."], "files_modified": ["..."],
                  "tools": ["Bash", "Edit"],
                  "commands": ["cargo test -p terminus-core --features testkit"],
                  "errors": ["..."], "commits": ["wire the observation step up"],
                  "branch": "phase-7", "turns": 47,
                  "first_turn_at": "...", "last_turn_at": "...",
                  "duration_seconds": 5421, "compactions": 1, "truncated": []}}]}
```

- One entry per **finalized** session that has been observed, in ingest order.
  A session the idle rule has not closed yet has no row, and neither does an
  excluded project's session - including one archived before the exclusion was
  configured.
- `mechanical` is the OBS-01 fact set, and every one of its values is
  parser-derived: **no model is called and no network connection is opened** by
  the ingest step that wrote it or by this command that reads it back.
  `commands` carries whole command lines with their arguments, which is what the
  `entities` table cannot answer - it holds basenames.
- `truncated` names the fact lists a cap cut, by field name. A list that is
  present and not named there is the whole of what the session did.
- `duration_seconds` is the wall clock between the first and the last turn, and
  it is null on a session carrying no timestamp to measure from.
- A store written before this build carries no `observations` table. That is an
  empty answer with a reason naming the table and exit 0, never a SQLite
  message: the next `terminus ingest` creates it.

### `observations regenerate`

```json
{"since": "2026-08-14T00:00:00.000Z", "selected": 2, "regenerated": 2, "notes": []}
```

- The **only** rebuild path for the observations table. `terminus reindex` never
  touches it: the facts come off the session's block rows, and retention
  eviction deletes those rows, so a rebuild that dropped the table could not
  recompute an evicted session's observation.
- `since` is the selector echoed back, null when it was not given. "Two rows
  were rebuilt" is unreadable without the bound that chose them.
- `--since` takes `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS.mmmZ` and selects
  sessions whose `session_meta.last_turn_at` is at or after it. A session
  carrying no `last_turn_at` is outside every bound. Without it, every visible
  session is selected.
- `selected` counts the rows the selector named **and** the exclusion gate
  allows; `regenerated` counts the ones actually rewritten, and `notes` names
  the difference, one line per row that could not be recomputed.
- It writes the `mechanical` column and only that, on the selected rows.
  `generated_at` dates the row as a whole, so a recompute leaves it alone, and
  every unselected row stays byte for byte what it was.
- This command writes, so it opens the store for writing and takes the ingest
  lock. A run refused because another process holds that lock is `ok: false`
  with the lock named in `reason`, exit 1 - the same answer `reindex` gives.

### `retention`

```json
{"cutoff": "2026-08-22T22:33:40.115Z",
 "evict": ["/home/you/.claude/projects/-data-code-app/9f2c....jsonl"],
 "delete": ["/home/you/.claude/projects/-data-code-app/1a04....jsonl"],
 "over": 0, "excluded": 3}
```

`terminus retention --dry-run` describes what the **next ingest pass** would do
under the current `terminus.toml`. It never applies anything: retention runs as
a bounded step at the end of every pass, that is the only place it acts, and
this command and that step share one evaluation function so the two cannot
disagree about a `'now'`-relative rule.

- `cutoff` is the instant every age in this report was measured from, in the
  same ISO-8601 UTC spelling every stored timestamp uses - **not** `now` less
  some number of days. There is no single such number: the age is configured per
  project, so what one report can honestly state is the one clock reading all of
  them were measured against. It is null only when there is no store to read a
  clock from. A pass running later reads its own instant, so a session sitting
  on an age boundary can legitimately fall the other side of it; that is why the
  instant is reported rather than implied.
- `evict` and `delete` are session keys - the canonical transcript paths - oldest
  first, and capped at what one pass will act on per action.
- `over` is how many more eligible sessions that cap left for the pass after
  next. The hook spawn is the scheduler, so "the next pass" is the next prompt.
- `excluded` counts sessions in projects the config excludes, which retention
  passed over and did not name. Exclusion means never read, on the ingest and
  the read paths both, and a delete is the most extreme thing that could be done
  to bytes a user said not to look at.

What the two verbs mean for everything else you can ask this store:

- An **evicted** session keeps its row, its metadata and every derived row; only
  its archived bytes go. It is still listed by `terminus sessions` (with
  `evicted: true`), a search that matched one of its turns still returns that
  turn, and `terminus show` reports `body_evicted: true` with a null `body`. Its
  `excerpt` in a search hit is empty, because the bytes an excerpt is cut from
  are the ones that were reclaimed.
- A **deleted** session is gone from both: no listing, no search hit, no turn.
  `delete` acts only on sessions whose transcript file Claude Code's own
  `cleanupPeriodDays` has already removed, so a session still on disk is never
  deleted however old it is - it would simply be re-ingested on the next hook
  spawn.

Empty is the ordinary answer and always exit 0, with `reason` saying which
empty: no store yet, no `[retention]` table selecting anything, or a configured
policy with nothing due. Retention is off by default.

### `compact`

```json
{"before": {"db": 15065088, "wal": 0, "shm": 32768, "total": 15097856},
 "after": {"db": 7540736, "wal": 0, "shm": 32768, "total": 7573504},
 "reclaimed_bytes": 7524352}
```

`terminus compact` reclaims the space retention already freed. It deletes
nothing and has no opinion about what should be kept: what to keep is
retention's decision, made inside the ingest pass, and this is the command that
gives the pages back to the filesystem afterwards.

- `before` and `after` are **the same three files `status` reports as
  `size_bytes`** - `terminus.db`, `terminus.db-wal` and `terminus.db-shm` - each
  measured with the connection open, so the two are the same measurement of the
  same store in two states. `total` is their sum.
- The three components are carried apart because the total alone cannot be read.
  `compact` is `VACUUM` followed by `PRAGMA wal_checkpoint(TRUNCATE)`, and both
  halves are load-bearing: a bare `VACUUM` on a WAL store writes the whole
  rebuilt database THROUGH the WAL, so `terminus.db` shrinks while the WAL grows
  by as much or more and the store on disk gets *bigger*. An `after` whose `wal`
  is not 0 is that failure, visible.
- `reclaimed_bytes` is `before.total - after.total` and is **signed**. A
  compaction that reclaimed nothing and left the store fractionally larger
  reports a negative number rather than a floor of zero.
- **Exit 1 when the ingest lock is held**, with the document still written and
  `ok` false, as `reindex` and `observations regenerate` also answer a held
  lock. The "0 including an empty result" rule does not apply here, and
  deliberately: a contended `ingest` exits 0 because the next hook spawn
  catches what it skipped, but a compaction was asked for explicitly
  and one that did not happen is an operational failure. `before` and `after`
  are the same unchanged measurement in that document, because nothing moved.
- A machine with no store is an ordinary empty answer: exit 0, a `reason`, all
  numbers zero, and no data directory created by the asking.

### `usage`

```json
{"sessions": 3416, "archive_bytes": 465985120,
 "by_project": [{"project": "/data/code/verbatim", "sessions": 812, "bytes": 141203456},
                {"project": null, "sessions": 1, "bytes": 4096}],
 "by_month": [{"month": "2026-08", "sessions": 3415, "bytes": 465981024},
              {"month": null, "sessions": 1, "bytes": 4096}],
 "file_bytes": 1103396864,
 "footprint": [{"name": "sessions", "bytes": 469413888},
               {"name": "turns_fts_data", "bytes": 350846976},
               {"name": "(free pages)", "bytes": 1126400},
               {"name": "(lock page)", "bytes": 4096}]}
```

**Two independent tables, and they never mix.** Each reconciles to its own total
and to nothing else. That is the whole design, and reading one against the
other's total is the mistake it exists to prevent.

- The ARCHIVE table is `by_project` and `by_month`. The bytes in each add up to
  `archive_bytes`, which is the sum of each session's block byte lengths
  (`length(bytes)` over its `blocks` rows) over the sessions a read path may
  see - and **not** to `file_bytes`. On the live store measured 2026-08-22 the
  file is 1,103,396,864 bytes against 465,985,120 of archive, 2.4x everything
  the projects account for; the rest is the derived tables, and attributing them
  to projects pro-rata would make every per-project number an estimate rather
  than a measurement.
- A session with no `project` and a session with no `first_turn_at` each get a
  bucket whose key is `null`. They are counted and their bytes are inside the
  totals: a null quietly dropped is a total that no longer reconciles, and the
  live store has exactly one of each. Buckets are biggest first, with the `null`
  one last whatever its size.
- A session is filed under one month, by `substr(first_turn_at, 1, 7)`.
  `first_turn_at` is established by byte order and never revised by a tail pass,
  and 0 of 3,416 live sessions have a `last_turn_at` in a different month.
- The FOOTPRINT table is `footprint`, per-table bytes out of SQLite's own
  `dbstat`. Its rows add up **exactly** to `file_bytes`, which is
  `page_count * page_size`. Two rows are not tables: `(free pages)` is
  `freelist_count * page_size`, which is the space `compact` is about to
  reclaim, and `(lock page)` is the single page SQLite reserves at byte
  `0x40000000` and uses for nothing, present only on a database that has grown
  past 1 GiB. Without those two the sum is short and the reconciliation is a
  claim rather than a property.
- Exclusion is retroactive (ING-08), so a project excluded after its sessions
  were archived appears in neither archive total. `footprint` is a property of
  the database file and is not filtered by anything.
- `usage` is a read: it opens read-only and creates no store.

### `export`

```json
{"destination": "/home/you/terminus-export",
 "manifest": "/home/you/terminus-export/manifest.json",
 "sessions": 3416, "turns": 432242, "evicted": 2, "bytes": 1556598685,
 "notice": "This export is the verbatim, unredacted transcripts: ..."}
```

`terminus export <dir>` writes one `.jsonl` file per session, holding that
session's uncompressed stream exactly as the archive stores it, plus a
`manifest.json` beside them. The transcript is the portable form for this
product: it is the form the data arrived in, the form any future importer would
read, and it opens without a terminus build. A copy of the store file is a
snapshot, not an export.

- **`export`'s output is the unredacted transcripts.** Nothing is filtered on
  the way out. Redaction is keyed on destination and never on operation
  (PRIV-01), and a directory the user named is not egress - so `notice` is the
  answer instead: the same words go into the manifest, onto the terminal ahead
  of the counts, and into this document, because a caller scripting an export is
  the one most likely to be writing somewhere shared.
- `bytes` is the total uncompressed stream written, which is what the files add
  up to - not the compressed size in the archive and not the size of the
  database.
- `evicted` counts sessions retention has emptied. Each is written as a
  **present, empty file** and named in the manifest with `"evicted": true`, so
  an export never silently drops a session it could not fill.
- A project the config excludes is in neither the manifest nor the directory.
  Export is a read path and ING-08 binds every read path alike.
- **A destination that already holds anything is refused**, exit 1, with nothing
  written - not merely one that already holds an export. A file this command did
  not write is never overwritten, and every refusal lands before the first byte.
  A missing destination on the command line is misuse (exit 2): there is no
  default place to put an unredacted copy of every transcript.
- The manifest carries `notice`, `archive_format` as the store itself reports
  it, the session / turn / evicted counts, the total bytes, the distinct
  projects, the `first_turn_at`..`last_turn_at` range, and a `files` array with
  one entry per session: `file`, `session_key`, `session_id`, `project`,
  `turns`, `bytes`, `evicted`, `first_turn_at` and `last_turn_at`. It is
  pretty-printed, unlike the one-line `--json` envelope: an envelope is piped,
  a manifest is opened and read.

### `doctor`

```json
{"binary": {"state": "ok",
            "finding": "/home/you/.local/bin/terminus is this build (0.1.0)", "fix": null},
 "hook_SessionStart": {"state": "problem",
                       "finding": "/home/you/.claude/settings.json has no terminus entry for SessionStart",
                       "fix": "/home/you/.local/bin/terminus install"},
 "cleanup_period_days": {"state": "note",
                         "finding": "cleanupPeriodDays is 7, set in /home/you/.claude/settings.json. ...",
                         "fix": "set \"cleanupPeriodDays\": 3650 in /home/you/.claude/settings.json"}}
```

`data` holds one object per check, keyed by the check's name. Every run carries
the same 14 keys: `binary`, `settings_file`, `hook_SessionStart`,
`hook_UserPromptSubmit`, `hook_SessionEnd`, `hook_PostCompact`, `mcp_server`,
`claude_code`, `config_roots`, `data_directory`, `store`, `last_run`,
`cleanup_period_days` and `auto_compact`.

| Field | Type | Meaning |
|---|---|---|
| `state` | string | One of `ok`, `note`, `unknown`, `problem`. Never null. |
| `finding` | string | One line, always present, never empty. Names the file or path it is about, so a check reads on its own. |
| `fix` | string or null | What to run, or the settings edit to make. Null wherever doctor has no one command or edit to give, as listed below. |

- `note` is true, worth saying, and not a failure: a machine before its first
  ingest, or a setting terminus would choose differently and never changes
  itself. `unknown` is doctor declining to guess - no `claude` on `PATH`, a
  settings file that will not parse - and, like `note`, never reaches the exit
  code.
- **Exit 0 unless some check is `problem`**, and `ok` is that same answer. An
  advisory does not make `doctor` report failure; `reason` on a failure names
  the count and every problem check by name.
- The key set does not depend on what doctor found. A check it could not run
  reports `unknown` rather than going missing, so a caller reads a state instead
  of testing for a key.
- `fix` is set only where doctor can name one command to run or one settings
  edit to make. It is null for every `ok` and every `unknown`. It is null for a
  `note` with nothing to run: a settings file, data directory or store not
  there yet, no ingest run yet, more than one config root, or a hook event
  another install's entry also runs. It is null for a `problem` doctor has no
  one command for: a `settings.json` or `.claude.json` that cannot be read or
  is not JSON, a duplicated hook entry, a config that does not load or resolves
  no root, a data directory that cannot be resolved or read or is not a
  directory, a store that cannot be opened or counted, a `runs` table that
  cannot be read, a `cleanupPeriodDays` that is not a number, and an
  `autoCompactEnabled` that is neither true nor false.
- `fix` is a shell command wherever one exists. Where the repair is a value in
  the user's own settings file, it is that edit written out literally, because
  terminus never changes either of the two Claude Code settings it reports
  (INST-06).
- `doctor` writes nothing anywhere: no file, no directory, on any path,
  including the data directory it reports as absent.

`install` and `uninstall` deliberately have no `--json` (D-24). Install prints a
receipt written for a person, and uninstall asks a question before `--purge`
deletes the archive, which a single JSON document on stdout cannot do.
