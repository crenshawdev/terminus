# `--json` shapes and the exit-code contract

Every data command accepts `--json`. This file is the contract, and
`crates/verbatim/tests/cli.rs` holds it to it: one test per property across all
commands, so a seventh command added later is one line rather than a new file.

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
| 1 | Operational failure: the work was attempted and did not succeed. `verify` found a corrupt blob, `reindex` skipped a session, the store could not be read. |
| 2 | Misuse: the command line does not name work that could be attempted. An unknown flag, an unknown subcommand, a non-numeric turn id, a malformed `--since`. |

A caller that parses `ok` and a caller that checks the exit code never
disagree. `verify --json` on a damaged store writes its document **and** exits
1; the document is the evidence and the code is the verdict.

**One case writes no document:** a failure raised before the command reached its
own answer - a store from the future, an unresolvable data directory - is
reported on stderr with exit 1 and stdout stays empty. The command never got far
enough to know `--json` had been asked for. Empty stdout with a non-zero exit is
therefore part of the contract, and a caller must check the exit code before it
parses.

## Streams

Data on stdout, every diagnostic on stderr, on every path. That is what lets
`verbatim search --json | jq` work while a warning is still printed.

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
- `excerpt` is cut from the session blob, never from FTS5, and may be empty when
  the archive would not give the bytes up.
- `--kind observation` searches stored session summaries instead of turns, and
  is the whole of how observations are reachable from the model - there is no
  fourth MCP tool. Each hit is one CLAIM: `turn_id` is the turn that claim is
  anchored to, `excerpt` is the claim's own text, `record_type` is
  `observation`, and `ts` is when the observation was written rather than when
  the turn happened. `relevance` is flat at 1.0, because there is no FTS index
  over these to score against and every hit matched every token; the order is
  newest observation first and is total. Scope and exclusion apply exactly as
  they do to turns. On a store written before the observations table exists, the
  answer is empty with that named in `reason` - never zero hits in silence.

### `show`

```json
{"records": [
  {"turn_id": 42, "session_key": "...", "turn_seq": 1, "record_type": "assistant",
   "tool_name": null, "ts": "...", "project": "...",
   "body": "{\"type\":\"assistant\",...}", "body_evicted": false, "context": null}],
 "absent": [{"turn_id": 99, "reason": "no turn 99 is archived"}]}
```

- `body` is the record's own archived line. It is the only lossy rendering
  verbatim performs: a JSON string is text by definition, so a byte that is not
  UTF-8 is replaced here. The human mode writes the bytes themselves.
- `body_evicted` comes from `session_meta.is_evicted` and from nothing else. A
  blob that will not decompress is archive damage, which `verbatim verify`
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
{"store": "/abs/path/verbatim.db", "size_bytes": 106496, "sessions": 11, "turns": 47,
 "watermarks": 11, "watermark_bytes": 34012, "excluded": ["/abs/path/private"],
 "last_run": {"started_at": "...", "duration_ms": 12, "files_seen": 11,
              "files_committed": 11, "files_failed": 0, "bytes_read": 34012,
              "turns_added": 47, "error": null}}
```

- The same numbers the human output prints. `size_bytes` counts the database and
  its WAL sidecars together.
- `last_run` is null on a store no pass has ever written to.

### `verify`

```json
{"checked": 11, "failures": [{"session_key": "...", "detail": "..."}]}
```

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
{"thresholds": {"ranked": 10, "compacted_ranked": 50, "entity_rank": 5, "co_occurring": 2,
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
- `labels` is one entry per label, always all four and always in that order.
  `old` is what the store holds, `new` is what the replayed rules produce.
- `changed` names the decision ids whose label set moved, in id order, so a diff
  can be followed back into the prompts behind it.
- `replay` never writes: the store is opened read-only, `verbatim.db` is byte
  for byte what it was, and the WAL carries no frame afterwards. A first
  read-only open of a WAL database does materialize SQLite's shared-memory
  index (`verbatim.db-shm`) and an empty `verbatim.db-wal`; those hold no page
  of the database, every reader needs them, and a read-only connection cannot
  unlink them on close.

### `observations`

```json
{"observations": [
  {"session_key": "/home/you/.claude/projects/-code-verbatim/1111....jsonl",
   "session_id": "1111...", "generated_at": "2026-08-21T18:31:07.412Z",
   "mechanical": {"files_read": ["..."], "files_modified": ["..."],
                  "tools": ["Bash", "Edit"],
                  "commands": ["cargo test -p verbatim-core --features testkit"],
                  "errors": ["..."], "commits": ["wire the observation step up"],
                  "branch": "phase-7", "turns": 47,
                  "first_turn_at": "...", "last_turn_at": "...",
                  "duration_seconds": 5421, "compactions": 1, "truncated": []},
   "status": null, "model": null, "prompt_version": null, "topic": null,
   "outcome": null, "decisions": null, "learned": null, "unresolved": null,
   "raw": null, "tokens": null}]}
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
- The judgment columns - `status`, `model`, `prompt_version`, `topic`,
  `outcome`, `decisions`, `learned`, `unresolved`, `raw`, `tokens` - are the
  optional LLM half. They are **present and null** until a provider is
  configured and answers, so a consumer reads one shape either way.
  `observations.decisions` here is the column and not the `decisions` table.
- `status` has three shapes and only two of them are answers. `"ok"` is a
  validated judgment; `"parse_failed"` is the OBS-04 arm, with the raw response
  in `raw` and the claim columns null; and `"judging <timestamp>"` is a
  **reservation**, written before the request goes out so that two overlapping
  runs cannot both pay for the same session - the second reads it, skips, and
  asks nothing. A reservation is transient: the run that took it replaces it
  with one of the two answers, or puts the column back to what it was if the
  provider never answered. One left standing by a killed process is treated as
  abandoned 15 minutes after its timestamp and the next ingest takes the
  session, so a crash mid-request costs a delay and never a permanently
  unjudgeable session.
- A store written before this build carries no `observations` table. That is an
  empty answer with a reason naming the table and exit 0, never a SQLite
  message: the next `verbatim ingest` creates it.

### `observations regenerate`

```json
{"since": "2026-08-14T00:00:00.000Z", "prompt_version": null,
 "selected": 2, "regenerated": 2, "notes": []}
```

- The **only** rebuild path for the observations table. `verbatim reindex` never
  touches it: the judgment half is a paid model call no blob replay reproduces,
  so a rebuild that ran on a schema bump would delete summaries a user bought.
- `since` and `prompt_version` are the selector echoed back, each null when it
  was not given. "Two rows were rebuilt" is unreadable without the bound that
  chose them.
- `--since` takes `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS.mmmZ` and selects
  sessions whose `session_meta.last_turn_at` is at or after it. A session
  carrying no `last_turn_at` is outside every bound. `--prompt-version` selects
  rows carrying exactly that value in the `prompt_version` column. Given both,
  they narrow conjunctively; given neither, every visible session is selected.
- `selected` counts the rows the selector named **and** the exclusion gate
  allows; `regenerated` counts the ones actually rewritten, and `notes` names
  the difference, one line per row that could not be recomputed.
- The mechanical half writes the `mechanical` column and only that, on the
  selected rows. `generated_at` dates the row as a whole - the judgment half
  included - so a mechanical recompute leaves it alone, and every unselected row
  stays byte for byte what it was.
- With `[provider] enabled = true`, the same selected rows are then re-asked of
  the model, which is what makes `--prompt-version` mean anything: the reason to
  pay for a session twice is that the prompt changed. This is the **only** path
  allowed to ask about a session that already carries a judgment status; every
  other cost control still applies, so a session under the minimum turn count is
  still not bought, and a session an ingest pass is asking about right now is
  skipped with a note rather than asked a second time in parallel. With no provider, the stored judgment columns are left
  exactly as they were and no request is made at all - a user without a model can
  still rebuild facts.
- `notes` carries the judgment half's report too: one `re-judged N of M selected
  session(s)` line whenever a provider was asked, one line per session whose
  answer could not be used, and - when the daily token budget runs out part way
  through the set - one line naming how many selected sessions were left
  unasked. A run that exhausts the budget stops asking, keeps every row it
  already rebuilt, and still exits 0.
- This command writes, so it opens the store for writing and takes the ingest
  lock. A run refused because another process holds that lock is `ok: false`
  with the lock named in `reason`, exit 1 - the same answer `reindex` gives. The
  lock covers the mechanical rewrite only: provider calls are made after it is
  released, because an HTTP call held inside the ingest lock would make every
  hook-spawned pass in that window archive nothing.

### `doctor`

```json
{"binary": {"state": "ok",
            "finding": "/home/you/.local/bin/verbatim is this build (0.1.0)", "fix": null},
 "hook_SessionStart": {"state": "problem",
                       "finding": "/home/you/.claude/settings.json has no verbatim entry for SessionStart",
                       "fix": "/home/you/.local/bin/verbatim install"},
 "cleanup_period_days": {"state": "note",
                         "finding": "cleanupPeriodDays is 7, set in /home/you/.claude/settings.json. ...",
                         "fix": "set \"cleanupPeriodDays\": 3650 in /home/you/.claude/settings.json"}}
```

`data`'s keys are the check names, and there is no list: `data` is the checks,
one object per check.

| Field | Type | Meaning |
|---|---|---|
| `state` | string | One of `ok`, `note`, `unknown`, `problem`. Never null. |
| `finding` | string | One line, always present, never empty. Names the file or path it is about, so a check reads on its own. |
| `fix` | string or null | What to run, or the settings edit to make. Null whenever there is nothing to do - which is every `ok`, and also a `problem` whose repair is a hand edit (a duplicated hook entry is the one that is). |

- `note` is true, worth saying, and not a failure: a machine before its first
  ingest, or a setting verbatim would choose differently and never changes
  itself. `unknown` is doctor declining to guess - no `claude` on `PATH`, a
  settings file that will not parse - and, like `note`, never reaches the exit
  code.
- **Exit 0 unless some check is `problem`**, and `ok` is that same answer. An
  advisory does not make `doctor` report failure; `reason` on a failure names
  the count and every problem check by name.
- The key set does not depend on what doctor found. A check it could not run
  reports `unknown` rather than going missing, so a caller reads a state instead
  of testing for a key.
- `fix` is a shell command wherever one exists. Where the repair is a value in
  the user's own settings file, it is that edit written out literally, because
  verbatim never changes either of the two Claude Code settings it reports
  (INST-06).
- `doctor` writes nothing anywhere: no file, no directory, on any path,
  including the data directory it reports as absent.

`install` and `uninstall` deliberately have no `--json` (D-24). They show a diff
and ask a question, and a single JSON document on stdout cannot be both of
those things.
