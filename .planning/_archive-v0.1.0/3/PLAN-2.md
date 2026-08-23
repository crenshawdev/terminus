---
phase: 3
plan: 2
requirements:
  - RCL-04
  - RCL-05
  - RCL-07
  - RCL-08
files:
  - crates/verbatim-core/src/lib.rs
  - crates/verbatim-core/src/error.rs
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/src/store/open.rs
  - crates/verbatim-core/src/store/schema.rs
  - crates/verbatim-core/src/recall/mod.rs
  - crates/verbatim-core/src/recall/query.rs
  - crates/verbatim-core/src/recall/scope.rs
  - crates/verbatim-core/src/recall/search.rs
  - crates/verbatim-core/src/recall/excerpt.rs
  - crates/verbatim-core/src/recall/context.rs
  - crates/verbatim-core/tests/recall.rs
  - crates/verbatim-core/tests/store_open.rs
  - crates/verbatim-core/tests/exclusion.rs
  - crates/verbatim-core/tests/schema.rs
---

# Phase 3: Recall - Plan 2 of 4 (the query layer)

**SEQUENTIAL: PLAN-1 must complete before this plan, and this plan before
PLAN-3 and PLAN-4.** This plan shares `crates/verbatim-core/src/lib.rs` and
`crates/verbatim-core/src/store/open.rs` with PLAN-1, and PLAN-3 and PLAN-4 are
both built on the `recall` module this plan creates. Do not run them in
parallel.

## Goal

One query layer in `verbatim-core` that both the terminal commands and the MCP
server sit on: a user's raw string becomes ranked turns scoped to the project
they are standing in, with excluded projects invisible, subagent turns present
but ranked below the turns the user watched, an excerpt that shows why each hit
matched, and a chronological window around any hit.

## Must be true when done

- A raw query containing `/`, `.`, an unbalanced quote or a bare `AND` returns
  results or an empty set, never an FTS5 syntax error.
- A search returns only turns from the project the process is standing in
  unless cross-project is asked for explicitly, and never returns a turn from
  an excluded project under either of its project keys.
- A subagent turn and a top-level turn with identical text rank in that order,
  top-level first.
- Every hit carries an excerpt cut from the session blob showing the matched
  text, not an empty string.
- A turn that carries an entity matching the query outranks one that matches
  only as free text, and a common entity value moves a hit less than a rare one.
- A context window around a hit returns turns in `turn_seq` order, stops at the
  session boundary, and says which end it stopped at.
- Opening the store for a read creates no file, runs no DDL and writes no
  pragma; a store that is not there is an empty result with a reason, not an
  error.

## Context

- D-10 locks reads to a read-only open. `crates/verbatim-core/src/store/open.rs`
  `Store::open` currently does `create_dir_all`, `initialize()` on a fresh
  path, `bring_forward` (which issues `CREATE TABLE` / `ALTER TABLE`) and
  `pragma_update(journal_mode=wal)`, none of which a `readOnlyHint` server may
  do on connect.
- D-09 locks the query path: the user's string is tokenized and rebuilt in Rust
  before it reaches `MATCH`. Measured on sqlite 3.53.4, `MATCH
  'src/worker/S.ts'` fails with `fts5: syntax error near "/"`, exit 1.
- D-21 locks exclusion on read to `config::visible`'s module but with a
  project-only projection: the existing `visible::sessions()` query costs
  6.2-6.8 ms because of its per-session `count(*)`, against 0.11 ms for the
  distinct-project projection, and the whole phase-5 budget is single-digit
  milliseconds.
- D-12 locks current-project auto-scoping to a longest-prefix match of the
  process `cwd` against the `project` and `project_pre_worktree` values already
  in `session_meta` - no git subprocess, because `project::Resolver` shells out
  with a 2 s budget and process spawn is 10-30 ms.
- D-07 keeps sidechain turns searchable and sorts them below top-level turns at
  equal score, distinguished by a non-null `session_meta.parent_session_key`;
  39.4% of sampled turns are sidechain. D-22 makes "chronological" mean
  `turn_seq`, never timestamp - 31 of 62 real transcripts carry a backwards
  timestamp. D-06 stops a context window at the session boundary and returns
  the boundary explicitly. D-05 cuts excerpts from the blob, because
  `snippet()` returns an empty string on a contentless FTS5 table. D-23
  compares `turns.ts` as text.
- Out of scope: every CLI surface and the MCP server (PLAN-3, PLAN-4), and
  `recall_get`'s by-id read, which PLAN-3 builds where `verbatim show` needs it.

## Tasks

### Task 1: Open the store read-only for reads

- **Files:** crates/verbatim-core/src/store/open.rs,
  crates/verbatim-core/src/error.rs, crates/verbatim-core/tests/store_open.rs
- **Action:** Add a second constructor beside `Store::open` that opens an
  existing store for reading and nothing else (D-10): `SQLITE_OPEN_READ_ONLY`,
  no `create_dir_all`, no `initialize`, no `bring_forward`, no
  `pragma_update`. It still runs the existing `gate` function - `gate` is
  already read-only and already populates what `Store::rebuild_required`
  reports, which is what lets PLAN-3's read commands say a store predates this
  build instead of silently querying an old-shape index (D-18). Dropping
  `bring_forward` means the columns it adds are not guaranteed present:
  `session_meta.project_pre_worktree`, `agent_meta` and `transcript_diverged`
  are in `BRING_FORWARD_COLUMNS` (`store/schema.rs:255-262`), so a store last
  written by a phase-1 binary lacks them and any read query naming one fails
  with `no such column` on a connection that cannot ALTER. The read path
  detects their absence and reports the store as predating this build - the
  D-18 degraded read - rather than surfacing a SQLite error. A data
  directory or database file that is not there must be distinguishable by the
  caller without matching on message text, the way `Error::TranscriptDiverged`
  is a variant rather than a formatted string, so a caller can render it as an
  empty result with a reason. Any other failure to open - including a WAL
  database on a medium where the shared-memory index cannot be created - is
  reportable the same way rather than a panic. Leave `Store::open` untouched:
  ingest, `verify`, `reindex` and `status` keep creating and bringing forward
  the store they are entitled to write.
- **Verify:** `cargo test -p verbatim-core --test store_open` shows that the
  read-only open against a directory holding no store creates no file and no
  directory (assert the path still does not exist afterwards), that it against
  a store written by an older `DERIVED_SCHEMA` reports `rebuild_required` and
  leaves `verbatim.db` and its sidecars byte-identical, and that a write
  attempted on its connection fails.

### Task 2: Turn a user's query string into an FTS5 MATCH expression

- **Files:** crates/verbatim-core/src/recall/mod.rs,
  crates/verbatim-core/src/recall/query.rs, crates/verbatim-core/src/lib.rs,
  crates/verbatim-core/tests/recall.rs
- **Action:** Create the `recall` module and give it the tokenizer D-09
  requires: the user's string is split on the same separators the index
  tokenizes on, each token is emitted as a quoted FTS5 string, and the tokens
  are combined so that all of them must appear. A raw query never reaches
  `MATCH` - `src/worker/S.ts`, `foo AND (bar` and `"unbalanced` are all fts5
  syntax errors, and RCL-06 forbids a non-zero exit for a query that simply
  found nothing. Do not expand query tokens the way `index::expand` expands
  indexed text: the expansion already happened at index time, so a query for
  `manager` matches the stored `Manager` token directly, and expanding again
  would widen a specific query into its own components. A query that reduces to
  no tokens at all yields no `MATCH` call and an empty result, because `MATCH
  ''` is itself an fts5 error. Bound the token count so a pasted stack trace
  cannot build a thousand-term expression.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  shows that `src/worker/S.ts`, `foo AND (bar`, `"unbalanced`, `*`, an empty
  string and a string of punctuation each produce either hits or an empty
  result and never a `rusqlite` error, and that `src/worker/S.ts` returns the
  fixture turn that contains it.

### Task 3: Rank matching turns

- **Files:** crates/verbatim-core/src/recall/search.rs,
  crates/verbatim-core/src/recall/mod.rs, crates/verbatim-core/tests/recall.rs
- **Action:** Build the search this phase exists for: `turns_fts MATCH` joined
  to `turns` and LEFT-joined to `session_meta` - an inner join drops every turn
  of a session with no meta row, the damage `config::visible::sessions`
  deliberately keeps visible (`config.rs:528-533`) - returning per hit the turn id, session key,
  timestamp, project and record type, ordered by relevance. Relevance is
  derived from `bm25(turns_fts)`, which SQLite returns as a NEGATIVE number
  where more negative is a better match - negate it once, in one place, and
  say so in a comment, because a sign error here ranks the worst match first
  and every test that only checks "hits came back" still passes. Sort by
  relevance descending, then by whether the hit is a sidechain turn - a
  non-null `session_meta.parent_session_key` sorts last (D-07) - then by
  `turns.ts` descending and `turns.id` ascending, so the order is total and two
  runs over an unchanged store agree. Sidechain turns are included by default:
  39.4% of turns are sidechain and excluding them would hide a long research
  subagent's whole output. Enforce a result limit with a named maximum that a
  caller may lower but not raise, which is the server-side budget RCL-10 needs
  in PLAN-4.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  shows that a query matching turns in several fixtures returns them in
  descending relevance, that the sidechain turn from
  `subagents/agent-echo.jsonl` and the identical-text top-level turn from
  `session-recall.jsonl` both come back with the top-level one first, and that
  asking for more results than the maximum returns the maximum.

### Task 4: Scope a search to a project and hide excluded ones

- **Files:** crates/verbatim-core/src/recall/scope.rs,
  crates/verbatim-core/src/recall/search.rs,
  crates/verbatim-core/src/config.rs,
  crates/verbatim-core/src/store/schema.rs,
  crates/verbatim-core/tests/exclusion.rs,
  crates/verbatim-core/tests/schema.rs, crates/verbatim-core/tests/recall.rs
- **Action:** Two scoping rules over one projection. Add to the
  `config::visible` module - the module every read path is required to go
  through - a function returning the distinct `project` and
  `project_pre_worktree` values in `session_meta` and which of them the config
  excludes, and use that rather than `visible::sessions()`, whose per-session
  `count(*)` costs 6.2-6.8 ms on a store shaped like the real one against 0.11
  ms for the projection (D-21). Exclusion is applied to BOTH columns, because
  either alone leaks a worktree session, and a session whose `project` is null
  stays visible because nothing can say it is excluded; `Config::
  excludes_everything()` short-circuits to an empty result. Current-project
  scoping is a longest-prefix match, on path components, of the process's own
  working directory against those same distinct stored keys (D-12) - never a
  git call, and never a fresh derivation through `project::Resolver`, because a
  rule that disagrees with what ingest wrote returns zero hits inside a
  worktree. A caller may pass a literal `*` to mean every project, which
  removes the project filter and leaves exclusion in force; a caller may name a
  project explicitly; and when the process stands in a directory no stored key
  is a prefix of, the default scope matches nothing and the reason says so
  rather than silently searching everything. Add `CREATE INDEX IF NOT EXISTS
  idx_session_meta_project ON session_meta(project)` to `schema::CREATE_SQL`
  (D-19) and note in a comment that it reaches an existing store only because
  `reindex` runs the whole `CREATE_SQL` batch and PLAN-1's `DERIVED_SCHEMA`
  bump forces that reindex - `Store::open` creates indexes only when a whole
  table is missing. Do not bump `DERIVED_SCHEMA` again; PLAN-1 already moved it
  for this phase.
- **Verify:** `cargo test -p verbatim-core --features testkit --test exclusion`
  and `--test recall` show that a search from inside a project returns only
  that project's turns, that `*` returns turns from several projects, that
  excluding a project after its sessions were archived removes its turns from
  both the default and the `*` result, that excluding the pre-worktree path
  alone also hides them, and that
  `SELECT count(*) FROM sqlite_master WHERE type='index' AND
  name='idx_session_meta_project'` is 1 on a fresh store and 1 after a reindex
  of a store created without it.

<!-- scoping note: a longest-prefix hit on `project_pre_worktree` resolves back
to that row's `project` before scoping. Phase 2 D-06 folded a repo and its
worktree into ONE key on purpose; scoping to the worktree key would show only
sessions run from the worktree and hide the repo's own, which is the
fragmentation D-12 exists to prevent. -->

### Task 5: Filter by tool, kind, path and time window

- **Files:** crates/verbatim-core/src/recall/search.rs,
  crates/verbatim-core/tests/recall.rs
- **Action:** Add RCL-07's filters to the search: `tool` against
  `turns.tool_name` (D-16 measured exactly one `tool_use` block per record, so
  the column is a complete filter), `kind` against `turns.record_type`, one or
  more paths against the `paths` table PLAN-1 now fills, and a `since` /
  `until` window against `turns.ts`. The time window is a lexicographic text
  comparison with no date parsing on the stored side (D-23): all 22,412
  sampled turns carry exactly the shape `NNNN-NN-NNTNN:NN:NN.NNNZ`, one format,
  UTC only, and `turns(ts)` is already indexed. Accept both a full timestamp
  and a bare `YYYY-MM-DD`, extending a bare date to the first instant of that
  day for `since` and the last for `until` so a one-day window includes its own
  day; reject a value of any other shape as a caller error rather than
  comparing it and returning a plausible wrong answer. Filters combine
  conjunctively and each one is independently optional.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  shows a query filtered to `tool = "Bash"` returning only Bash turns, a
  `kind = "user"` filter returning only user turns, a path filter returning
  only turns whose `paths` rows match, a `since`/`until` window that includes
  and excludes fixture turns by their known timestamps, and a malformed date
  reported as a caller error rather than as an empty result.

### Task 6: Weight entity matches by inverse document frequency

- **Files:** crates/verbatim-core/src/recall/search.rs,
  crates/verbatim-core/src/recall/query.rs, crates/verbatim-core/tests/recall.rs
- **Action:** Close RCL-04's query-time half. The match is between the query and
  a stored `entities.value_norm`, NOT between a single query token and a whole
  value: `path` and `error` values are multi-token by construction (PLAN-1 tasks
  4 and 5 store `src/worker/S.ts` and a whole normalized stderr line), so a
  token-equality rule can never fire for the two kinds this phase headlines.
  Match a value whose own tokenization is covered by the query's token set, and
  keep exact whole-string equality as the strongest case. For each matched
  value, compute its document frequency - the number
  of distinct `turn_id` carrying that `(kind, value_norm)`, which
  `idx_entities_lookup` already covers - and add an inverse-document-frequency
  contribution to the relevance of every candidate turn carrying that entity,
  so an exact structural match outranks a free-text match and a value carried
  by half the corpus moves a hit far less than a rare one. Nothing is rejected
  and no stop-list exists: "too common" changes as the corpus grows, which is
  why RCL-04 puts this at query time and forbids it at index time. Keep the
  computation deterministic and dependent only on the stored rows, so two runs
  over an unchanged store produce the same order - phase 5's structural
  threshold reads rank 1-3 and a nondeterministic tie makes that threshold
  fire on different turns each time.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  shows that for a query naming a path, the turn whose `tool_use` carried that
  path as an entity outranks a turn that merely mentions it in text, and that
  seeding a store where one entity value appears in many turns and another in
  one leaves the rare one contributing the larger score increase.

### Task 7: Cut each hit's excerpt from the session blob

- **Files:** crates/verbatim-core/src/recall/excerpt.rs,
  crates/verbatim-core/src/recall/search.rs, crates/verbatim-core/tests/recall.rs
- **Action:** Give every hit an excerpt read from the archive, never from FTS5:
  measured on sqlite 3.53.4 against the contentless table declared at
  `store/schema.rs:150-151`, `snippet()` returns an empty string with exit 0
  while `bm25()` still ranks, so an excerpt built from `snippet()` would
  validate against the documented shape and tell the reader nothing. Read the
  turn's bytes with `blob::BlobReader::read_range` at the `stream_offset` and
  `byte_len` already stored on the row - the working pattern is
  `testkit::read_turn` - project them through the same `index::text` function
  ingest used, so the excerpt is prose rather than JSON, and cut a bounded
  window around the first occurrence of a query token with an explicit
  elision marker, falling back to the head of the projection when no token
  occurs in the text (an entity-only hit). Read each session's blob at most
  once per search (D-20): rusqlite's incremental blob I/O is behind a `blob`
  feature this workspace deliberately does not enable, so the whole blob is
  materialized per read and sessions reach 10.1 MB uncompressed. A blob that
  will not decompress yields a hit with an empty excerpt and the rest of its
  fields intact rather than failing the search - `verbatim verify` is what
  reports that damage.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  shows every hit for a fixture query carrying a non-empty excerpt containing
  the queried token, no excerpt containing a JSON key such as `"type"` or
  `"message"`, and a search returning several hits from one session reading
  that session's blob once - assert on `BlobReader::blocks_decompressed` or on
  a counted blob query, the instrument phase 1 already uses for exactly this
  kind of claim.

### Task 8: Return the chronological window around a hit

- **Files:** crates/verbatim-core/src/recall/context.rs,
  crates/verbatim-core/src/recall/mod.rs, crates/verbatim-core/tests/recall.rs
- **Action:** Add RCL-08: given a turn id and a count before and after, return
  the surrounding turns of the SAME session in `turn_seq` order.
  `schema::split_turn_id` already recovers `(session_no, turn_seq)` from an id
  and `turns` carries `UNIQUE (session_key, turn_seq)`, the covering index this
  window needs. Order by `turn_seq` and never by timestamp (D-22): 31 of 62
  real transcripts carry a record whose timestamp goes backwards, so a
  timestamp sort would show a reply before the prompt it answers in half of all
  sessions. The window stops at the session boundary and does not follow
  `continues_from` or `parent_session_key` (D-06) - continuation is a fan-out
  with zero, one or many successors and 2 of 173 files name a predecessor that
  was never ingested, so a lineage walk here can cross into a fork that never
  happened. Report explicitly when the window was cut short at the start or the
  end of the session, so a caller can decide to issue a second call rather than
  guess from a short list. A turn in an excluded project returns an empty
  window with a reason, on the same scoping projection task 4 built.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  shows a window around a middle turn returning the requested count on each
  side in `turn_seq` order, a window around the first turn of
  `session-continuation.jsonl` (which carries a `continues_from` link)
  returning nothing before it and flagging the start boundary, and a window
  around a turn of an excluded project returning empty with a reason.

## Notes

- D-10 names `search`, `show`, `sessions` and `mcp` as the read-only openers
  and nothing else, so task 1 adds the constructor and changes no existing
  caller. `verbatim status` and `verbatim verify` still create a store as a
  side effect of being run against an empty data directory; that is outside
  this phase's locked decisions and is left for the human to weigh.
- The `blob` feature that would give `recall_get` and the excerpt reader
  incremental blob I/O is explicitly out of scope for this phase. Task 7 bounds
  the cost by reading each session once per request rather than eliminating it.
