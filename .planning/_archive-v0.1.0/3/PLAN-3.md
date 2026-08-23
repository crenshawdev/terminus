---
phase: 3
plan: 3
requirements:
  - RCL-05
  - RCL-06
  - RCL-09
files:
  - Cargo.lock
  - crates/verbatim/Cargo.toml
  - crates/verbatim/src/main.rs
  - crates/verbatim/src/cmd/mod.rs
  - crates/verbatim/src/cmd/json.rs
  - crates/verbatim/src/cmd/read.rs
  - crates/verbatim/src/cmd/search.rs
  - crates/verbatim/src/cmd/show.rs
  - crates/verbatim/src/cmd/sessions.rs
  - crates/verbatim/src/cmd/status.rs
  - crates/verbatim/src/cmd/verify.rs
  - crates/verbatim/src/cmd/reindex.rs
  - crates/verbatim-core/src/recall/get.rs
  - crates/verbatim-core/src/recall/mod.rs
  - crates/verbatim-core/tests/recall.rs
  - crates/verbatim/tests/cli.rs
  - crates/verbatim/tests/recall_cli.rs
  - crates/verbatim/tests/status.rs
  - docs/json-shapes.md
---

# Phase 3: Recall - Plan 3 of 4 (terminal recall and the JSON contract)

**SEQUENTIAL: PLAN-2 must complete before this plan, and this plan before
PLAN-4.** This plan is built on the `recall` module PLAN-2 creates and adds to
it; PLAN-4 shares `crates/verbatim/src/main.rs` and
`crates/verbatim/src/cmd/mod.rs` with this plan. Do not run them in parallel.

## Goal

Recall from the terminal: `verbatim search`, `verbatim show` and `verbatim
sessions` find and print a past turn, and every data command speaks one JSON
contract - stable shape on stdout, diagnostics on stderr, exit 0 on success
including an empty result, 1 on operational failure, 2 on misuse.

## Must be true when done

- `verbatim search src/worker/S.ts` exits 0 and prints hits; a query matching
  nothing exits 0 with an empty result set; an unknown flag exits 2.
- Each of `search`, `show`, `sessions`, `status`, `verify` and `reindex`
  accepts `--json`, writes JSON and only JSON to stdout, and writes every
  diagnostic to stderr.
- `verbatim show` on a turn whose session is marked evicted prints the record
  flagged as evicted rather than failing.
- Running a read command against a data directory holding no store exits 0 with
  an empty result and a reason, and creates no file there.
- A read command against a store whose derived tables predate this build says
  so instead of silently returning fewer hits.

## Context

- D-24 puts the `--json` retrofit onto `status`, `verify` and `reindex` in this
  phase, not phase 4. `crates/verbatim/src/main.rs:83-90` currently rejects
  `verify --json` with exit 2 through `no_more_arguments`, whose comment names
  that as deliberate until this contract lands.
- D-25 emits JSON with `serde_json` named directly in the `verbatim` binary
  crate, which adds no newly compiled crate because `verbatim-core` already
  depends on it. Shapes are never hand-formatted: escaping an excerpt holding
  quotes, backslashes or control bytes is exactly where `format!` fails, and it
  fails on the turns most worth reading.
- D-10 keeps `search`, `show` and `sessions` on PLAN-2's read-only open, and
  D-18 requires a read command against a store predating the bump to say so
  rather than repair it - `reindex::open_up_to_date` stays the only caller that
  rebuilds.
- D-08 sources `body_evicted` from `session_meta.is_evicted` and never from a
  failed blob read: retention is phase 8 and the column stays null until then,
  so the test for it sets the column directly. D-20 groups requested ids by
  `session_key` so each blob decompresses once.
- Existing shape to follow: `cmd::Failure` already splits `Operational` (exit
  1), `Silent` (exit 1, already reported) and `Misuse` (exit 2), and
  `cmd::status` shows the house style - `rusqlite` is deliberately not named in
  the binary crate, and every number comes through `verbatim-core`.
- Out of scope: the MCP server (PLAN-4), and hooks, install and MCP
  registration (phase 4).

## Tasks

### Task 1: Add the JSON emitter and the shared `--json` flag

- **Files:** crates/verbatim/Cargo.toml, Cargo.lock,
  crates/verbatim/src/cmd/json.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs
- **Action:** Name `serde_json` in the `verbatim` binary crate's manifest as a
  workspace dependency (D-25) and add the module every command emits through.
  One envelope shape shared by every data command so a caller can tell success
  from an empty result without parsing prose, carrying the command's data plus
  any reason a result is empty; values are built as `serde_json` values and
  serialized, never assembled with `format!`. Add `--json` parsing where every
  subcommand can reach it - `main.rs`'s `dispatch` hands the `lexopt::Parser`
  on to each subcommand, so the flag is parsed by the subcommand, not by
  `run()`. `no_more_arguments` currently rejects every argument for `verify`,
  `reindex` and `status`; it stays the rule for everything except the flags
  this phase adds. Update `USAGE` to list the new commands and the flag. Keep
  the split absolute: JSON goes to stdout, every diagnostic to stderr, which is
  what lets `verbatim search --json | jq` work while a warning is still
  printed.
- **Verify:** `cargo build --workspace` succeeds and `cargo tree -p verbatim`
  shows `serde_json` already present through `verbatim-core` with no new
  transitive crate; a unit test in `cmd/json.rs` shows a string containing a
  double quote, a backslash and a control byte round-tripping through the
  emitter and back through `serde_json::from_str` unchanged.

### Task 2: Give read commands one entry point

- **Files:** crates/verbatim/src/cmd/read.rs, crates/verbatim/src/cmd/mod.rs
- **Action:** Add the single place `search`, `show` and `sessions` open the
  store, so the three cannot drift on the two behaviours D-10 and D-18 fix.
  It opens through PLAN-2's read-only constructor - never `Store::open`, which
  would create a store as the side effect of a read - and turns "no store
  there" into an empty result carrying a reason and exit 0, because a machine
  that has never ingested is not a machine with a broken command. When the
  opened store reports `rebuild_required`, it prints one line to stderr saying
  the derived tables predate this build and that the next `verbatim ingest`
  rebuilds them, and then queries the old-shape index anyway (D-18): a read
  that silently rewrote four tables would not be a read, and a user who
  upgraded and searched before the next hook fired must not read a degraded
  result as a search bug.
- **Verify:** `cargo test -p verbatim --features testkit --test recall_cli`
  shows each of the three commands run against an empty data directory exiting
  0, printing an empty result with a reason, and leaving that directory with no
  `verbatim.db` in it; and each run against a store whose `meta.derived_schema`
  was rolled back by one printing the staleness line on stderr, exiting 0, and
  leaving `meta.derived_schema` unchanged.

### Task 3: Read verbatim records for a list of turn ids

- **Files:** crates/verbatim-core/src/recall/get.rs,
  crates/verbatim-core/src/recall/mod.rs, crates/verbatim-core/tests/recall.rs
- **Action:** Add RCL-09's core: given turn ids, return each turn's full
  verbatim text with its session, timestamp and project. Group the ids by
  `session_key` and read each session's blob at most once per request (D-20) -
  rusqlite's incremental blob I/O is behind a feature this workspace does not
  enable, so a naive per-id read materializes a whole 10 MB blob per turn while
  the block counter at `blob/reader.rs:44-53` still reports one block
  decompressed and phase 1's AC1 stays green. The bytes returned are the
  record's own, read at the stored `stream_offset` and `byte_len` - the
  verbatim line, not the projection, because this is the command that answers
  "what exactly was said". A turn whose session has `session_meta.is_evicted`
  set returns the record flagged as evicted with no blob read attempted at all,
  and that flag is read from that column only (D-08): a corrupt blob is what
  `verbatim verify` reports, and letting a failed read look like an eviction
  would report silent data loss as retention working correctly. An id that
  names no turn, and a turn in an excluded project, are absent from the result
  with a reason rather than an error.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  shows ids from three sessions returning three records with each blob selected
  once, the returned bytes equal to the fixture's own line for that turn, a
  turn whose session has `is_evicted` set to 1 by the test returning the
  evicted flag and no body, and an unknown id returning a reason rather than an
  error.

### Task 4: Ship `verbatim search`

- **Files:** crates/verbatim/src/cmd/search.rs,
  crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs,
  crates/verbatim/tests/recall_cli.rs
- **Action:** Add the command RCL-05 is named for, dispatched from `main.rs`
  beside `ingest`, `verify`, `reindex` and `status`. It takes the query as
  positional words, and flags for project scope (including the literal `*` for
  every project), tool, kind, one or more paths, `--since` / `--until`,
  `--limit` and `--json`, each mapping onto the filters PLAN-2 built - the
  command parses and validates, and the query layer decides. Human output is
  one hit per entry with its id, session, day-resolution timestamp, project and
  excerpt, readable in a terminal; `--json` emits the same hits through task
  1's emitter. A query that matches nothing exits 0 with an empty result set,
  never non-zero (RCL-06); an unknown flag is `Failure::Misuse` and exits 2; an
  unparseable `--since` is misuse too, not an empty result. Never pass the
  user's string through to SQLite - PLAN-2's tokenizer is the only way a query
  reaches `MATCH`, and `verbatim search src/worker/S.ts` is both the most
  natural command in the product and the string that makes a raw `MATCH` exit
  1 with an fts5 parser message.
- **Verify:** `cargo test -p verbatim --features testkit --test recall_cli`
  shows `verbatim search src/worker/S.ts` exiting 0 with at least one hit
  naming the fixture turn, `verbatim search SearchManager` and `verbatim search
  manager` returning the same turn, a query for a token in no fixture exiting 0
  with an empty result on stdout, `verbatim search --nope x` exiting 2, and
  `verbatim search --json x` producing stdout that parses as JSON with nothing
  else in it.

### Task 5: Ship `verbatim show`

- **Files:** crates/verbatim/src/cmd/show.rs, crates/verbatim/src/cmd/mod.rs,
  crates/verbatim/src/main.rs, crates/verbatim/tests/recall_cli.rs
- **Action:** Add the command that prints what a turn actually said: it takes
  one or more turn ids as printed by `search`, reads them through task 3, and
  prints each record's verbatim text with its session, timestamp and project.
  Flags for a chronological window before and after each id, served by PLAN-2's
  context window - so `show` is where a user follows a hit into the
  conversation around it - and `--json`. A window that hit the session boundary
  says so in the output rather than just returning a shorter list (D-06). A
  turn flagged evicted prints as evicted and the command still exits 0. An id
  that is not a number is misuse (exit 2); an id that names no turn is an empty
  result with a reason (exit 0).
- **Verify:** `cargo test -p verbatim --features testkit --test recall_cli`
  shows `verbatim show <id>` printing bytes equal to the fixture's own line for
  that turn, `--before`/`--after` returning the neighbouring turns in
  `turn_seq` order and flagging the boundary at the first turn of a session, a
  turn whose session has `is_evicted` set printing as evicted with exit 0, and
  `verbatim show notanumber` exiting 2.

### Task 6: Ship `verbatim sessions`

- **Files:** crates/verbatim/src/cmd/sessions.rs,
  crates/verbatim/src/cmd/mod.rs, crates/verbatim/src/main.rs,
  crates/verbatim/tests/recall_cli.rs
- **Action:** Add the listing command: every session a read path may see, with
  its key, project, branch, first and last turn timestamps, turn count, whether
  it is a sidecar (a non-null `session_meta.parent_session_key`) and whether it
  is evicted. Flags for project scope, `--since` / `--until`, `--limit` and
  `--json`. This is a listing rather than a hot query path, so it goes through
  the existing `config::visible::sessions` - the module every read path is
  required to use - rather than PLAN-2's projection, and an excluded project's
  sessions are absent from it including the ones archived before the exclusion
  was configured. Report the count of listed sessions on stderr the way
  `status` reports its commentary, so stdout stays parseable.
- **Verify:** `cargo test -p verbatim --features testkit --test recall_cli`
  shows `verbatim sessions` listing every ingested fixture session with its
  project, the sidecar fixtures flagged as sidecars, a project excluded through
  `verbatim.toml` absent from both the human and the `--json` output while its
  turns are still in the store, and `--json` output parsing as JSON.

### Task 7: Retrofit `--json` onto status, verify and reindex

- **Files:** crates/verbatim/src/cmd/status.rs,
  crates/verbatim/src/cmd/verify.rs, crates/verbatim/src/cmd/reindex.rs,
  crates/verbatim/src/main.rs, crates/verbatim/tests/status.rs,
  crates/verbatim/tests/cli.rs
- **Action:** D-24: the three commands phase 1 and 2 shipped join the contract
  in the phase that owns it, so a shape change never lands beside install in
  phase 4 where something is already scripted against it. `status --json`
  emits the store path, size, visible session, turn and watermark counts, the
  exclusion list and the last run's row including its error, the same numbers
  the human output prints. `verify --json` emits the checked count and the
  failing session ids - the ids are the data and already go to stdout, the
  count is commentary and already goes to stderr, so JSON mode puts both in one
  document on stdout. `reindex --json` emits the rebuilt session and turn
  counts and the skipped sessions with their reasons; it currently prints
  nothing at all to stdout by design, and that stays true without `--json`.
  Exit codes do not change: `verify` and `reindex` still exit 1 when something
  failed, with the JSON document still written, because a caller that parses
  the document and a caller that checks the code must not disagree about
  whether the store is whole.
- **Verify:** `cargo test -p verbatim --features testkit --test cli` and
  `--test status` show each of the three commands with `--json` writing a
  parseable JSON document to stdout and nothing else, the same commands without
  `--json` producing byte-identical output to what they produce today, and
  `verify --json` on a store with a flipped byte exiting 1 with the failing
  session id present in the JSON.

### Task 8: Pin the exit-code and shape contract

- **Files:** docs/json-shapes.md, crates/verbatim/tests/cli.rs,
  crates/verbatim/src/cmd/mod.rs
- **Action:** Document the JSON shape each data command emits - the shared
  envelope and each command's own fields - in `docs/json-shapes.md`, and write
  the sweep that holds every command to it: for each of `search`, `show`,
  `sessions`, `status`, `verify` and `reindex`, `--json` output parses, matches
  the documented shape field for field, and stdout carries no non-JSON byte
  while diagnostics land on stderr. Assert the exit-code contract in the same
  sweep: 0 for success including an empty result set, 1 for an operational
  failure, 2 for misuse, with an unknown flag and an unknown subcommand both
  exit 2. Replace the phase-scoping note at the top of
  `crates/verbatim/src/cmd/mod.rs`, which says the contract is phase 3's, with
  what the contract now is. The sweep is one test per property across all
  commands rather than one test per command, so a seventh command added in a
  later phase is one line rather than a new file.
- **Verify:** `cargo test -p verbatim --features testkit` passes, and
  `verbatim search --json somethingthatmatchesnothing; echo $?` prints a JSON
  document followed by `0`.

## Notes

- `verbatim show` is the terminal's `recall_get` plus `recall_context`, which
  is why task 3 builds the by-id read here rather than in PLAN-4: PLAN-4's
  `recall_get` tool then wraps a function that already has terminal coverage.
- Task 7 is the first time `verify` and `reindex` accept any argument at all.
  `no_more_arguments` in `main.rs` is what rejects arguments today, and its
  doc comment names `verify --json` as the exact case it was written to keep
  from looking supported before it was.
