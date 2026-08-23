# Phase 1: Prompt-True Resume Brief - Context

Gathered: 2026-08-23
Feeds: /cad-plan 1

## Scope boundary

In: A stored per-turn discriminator that says whether a `user` record was
authored by the person, written at ingest and rebuilt from blobs alone; the
`SessionStart` brief's "It last asked" line reading it so the quoted prompt is
always a turn the person typed; the schema bump and bring-forward that let a
store from the previous build acquire the column without breaking future
ingests.

Out: Any change to what the brief renders besides the quoted-prompt line;
`recall` search / get / context, which name `record_type` but not this column;
the observation and egress paths (phases 2 and 4); retention.

Deferred: None.

Plan shape: multiple plans, same phase - the schema/migration and
bring-forward, the ingest-side classifier, and the brief render plus fixtures
are separable leases over different files.

## Durable decisions

- D-01 (Discriminator): The classifier is a `tool_result` block in the record's
  own `message.content[]`, read from the same bytes with no cross-record join,
  NOT the top-level `toolUseResult` key. Measured 2026-08-23 over 400 sampled
  transcripts (13,756 `user` turn records): `toolUseResult` present implies a
  `tool_result` block in 12,234/12,234 cases, and the block appears without the
  key in a further 282 records, all in `agent-*.jsonl` sidecars - the block rule
  is a strict superset and the key rule adds nothing. Evidence:
  `crates/verbatim-core/src/parse/record.rs:298-313` (`tool_name` already walks
  `message.content` and names itself the extension point),
  `crates/verbatim-core/src/index/text.rs:143-155`; carries forward v0.1.0
  phase 3 D-02 (no cross-record join). If wrong: 282 of 12,516 tool-result
  records classify as typed and a sidecar brief quotes a file listing as a
  prompt.

- D-02 (Discriminator): "Typed" means authored by the person, not merely "not a
  tool result". The not-typed side folds in `isMeta: true` records and the
  harness envelope shapes alongside tool results. Still exactly two states.
  Chosen against the strict tool-result reading because measured 2026-08-23
  over 400 top-level transcripts, the record a strict rule would quote is
  harness-authored in 49 of 400 sessions (12.3%): 21 task-notification
  envelopes, 21 `isMeta: true` command caveats, 4 local-command-stdout, 3
  command-name records. `promptSource` cannot serve as the discriminator - it is
  present on only 57 of 400 and never on a tool-result record (0 of 6,822).
  Accepted cost: the column's name stops meaning `tool_result`, and the harness
  shape list is something Claude Code can change under us. If wrong: the phase
  ships and one session in eight still renders the defect INJ-07 exists to
  close. REQUIREMENTS.md's INJ-07 row was corrected to this wording.

- D-03 (Storage): The column lives on `turns` (a derived table) and the upgrade
  is a `DERIVED_SCHEMA` bump 3 -> 4, which makes `reindex::reindex` the
  "backfill from blobs alone" the acceptance criteria name. Note the naming
  collision: `crates/verbatim-core/src/ingest/backfill.rs` is the parallel
  transcript-reading pass and is NOT this mechanism. Evidence:
  `crates/verbatim-core/src/store/open.rs:33` (phase 3 bumped to 3 for a
  `turns_fts.body` shape change), `crates/verbatim-core/src/reindex.rs:92-203`
  (reads only `sessions.blob`), `crates/verbatim-core/src/store/schema.rs:385-391`.
  If wrong: without the bump, `derive::derive_turn`'s explicit INSERT column
  list fails with `no such column` on the next ingest.

- D-04 (Storage): The column is nullable and written for `record_type = 'user'`
  rows; "no third state" is asserted as a count query returning zero, never as a
  `NOT NULL` declaration. Preserved evicted-session rows
  (`crates/verbatim-core/src/reindex.rs:146-152`) are skipped by reindex and
  keep NULL whatever the declaration says. Precedent: `capture_mode`
  (`schema.rs:110-121`), a nullable added column whose null has a documented
  meaning. If wrong: a `NOT NULL DEFAULT` makes every preserved evicted row read
  as one class silently, and an evicted session that is the project's most
  recent quotes on a value nothing derived.

## Decisions

- D-05 (Discriminator): A `user` record carrying both a `text` block and a
  `tool_result` block classifies as not-typed. Measured: 2 such records in a
  180-file sample, both `isSidechain: true` fork boilerplate in `agent-*.jsonl`;
  zero in a 300-file top-level sample. Evidence:
  `crates/verbatim-core/src/index/text.rs:125-158`.

- D-06 (Discriminator): No check may read `toolUseResult` through
  `Value::as_object`. Measured over the 300-file top-level sample it is a dict
  on 6,265 records, a string on 522 and a list on 35 - 8.2% would misclassify.
  This contradicts v0.1.0 phase 3 D-02's "present as a dict on 4,858 records"
  as a general shape claim. `crates/verbatim-core/src/capture.rs:175-181`
  already returns `None` for the string form.

- D-07 (Discriminator): The rule survives `[capture]` elision, so a `lean` or
  `minimal` ingest and a later blob-only rebuild agree. Elision replaces only
  the top-level `toolUseResult` and `attachment` and leaves `message.content`
  untouched. Evidence: `crates/verbatim-core/src/capture.rs:82,157`,
  `crates/verbatim-core/src/ingest/mod.rs:487-507`.

- D-08 (Storage): `turns` must also gain an entry in `BRING_FORWARD_COLUMNS`.
  `crates/verbatim-core/src/reindex.rs:122-144` skips the DROP loop when a
  session is preserved and only runs `CREATE TABLE IF NOT EXISTS`, a no-op on an
  existing table, so the column is never created; `derive::derive_turn`
  (`crates/verbatim-core/src/derive.rs:70-92`) then names it and errors, and
  `open_up_to_date` (`reindex.rs:261-273`) runs at the top of every pass, so the
  failure is permanent. Current list: `schema.rs:414-431` (only `session_meta`
  and `runs`); only ALTER site: `store/open.rs:356-398`.

- D-09 (Storage): `verbatim verify` needs no work - its single query reads
  `sessions` and `session_meta` and never touches `turns`.
  `crates/verbatim-core/src/verify.rs:101-147`.

- D-10 (Storage): The read-only degraded path needs no new handling.
  `crates/verbatim-core/src/inject/mod.rs:127-137` returns `None` on
  `predates_this_build()` and `store/open.rs:320-322` makes a `DERIVED_SCHEMA`
  mismatch enough, so the brief goes silent rather than hitting `no such column`
  between upgrade and first ingest. `recall/search.rs`, `recall/get.rs` and
  `recall/context.rs` name `record_type` but not this column.

- D-11 (Query): One extra `AND` on the existing `last_turn` statement, no new
  index. Measured 2026-08-23 against the live 1.16 GB store (450,834 turns,
  121,338 `user` rows, 0 with `tool_name`), 50 iterations on the largest session
  (2,465 turns): current query 0.002 ms, a variant forced to scan that session's
  turns backwards without matching 0.362 ms, both planned as `SEARCH turns USING
  INDEX sqlite_autoindex_turns_1`. Real scan depth p50 27, p90 181, p99 404, max
  520 rows. The wall asserted at `crates/verbatim/tests/brief.rs:217` and
  `crates/verbatim/tests/hook.rs:376` is 10 ms. Evidence:
  `crates/verbatim-core/src/inject/brief.rs:504-515`.

- D-12 (Render): The session state file needs no change - the ids already travel
  on the `Quote` that was actually assembled, so INJ-04's suppression follows the
  new rule for free. Evidence: `crates/verbatim-core/src/inject/brief.rs:186-195,
  110-119`, `crates/verbatim-core/src/inject/state.rs:193-197`.

- D-13 (Render): The "no typed prompt at all" arm is fixture-only - 0 of 300 and
  0 of 400 real transcripts have zero person-authored `user` records. It is
  directly constructible from `tests/fixtures/session-errors-a.jsonl` and
  `session-errors-b.jsonl`, both rooted at `project-beta`
  (`crates/verbatim-core/src/testkit.rs:54-60`).

- D-14 (Tests): The corpus measurement uses the existing `VERBATIM_TEST_CORPUS`
  gate and its loud-skip convention, not a new mechanism, and must keep
  `VERBATIM_CONFIG_DIR` temporary or the developer's real `roots` win. Evidence:
  `crates/verbatim-core/src/testkit.rs:178-198`,
  `crates/verbatim/tests/corpus.rs:1-40,445-452`.

- D-15 (Tests): The byte-identity assertion needs a new rooted fixture.
  `crates/verbatim/tests/brief.rs:36` seeds from `session-recall.jsonl`, whose
  only `user` record is a text block, so today's test cannot observe the change.
  `session-capture.jsonl` has the right record order (`text`, `tool_result`,
  `tool_result`) but is not among the five rooted fixtures.

- D-16 (Reach): `derive::derive_turn` and `parse::Record::parse` are the only
  production reach points. `derive_turn` has two non-test callers,
  `ingest/mod.rs:495` and `reindex.rs:183`, and `ingest/backfill.rs:365` reaches
  the store through `crate::ingest::apply`, the same function `pass.rs` uses, so
  all three ingest entry points converge on `ingest/mod.rs:487-508`. Phase 8's
  three-way `mod.rs`/`pass.rs`/`backfill.rs` split does not apply here.

## Acceptance criteria

- [ ] AC1: In a freshly ingested store,
      `SELECT count(*) FROM turns WHERE record_type='user' AND <col> IS NULL`
      returns 0, and `brief.rs`'s last-turn query differs from today's by exactly
      one `AND` with no additional blob opened.
- [ ] AC2: On a store whose last `user` record is a tool result, `SessionStart`
      quotes the session's last typed prompt; on a store seeded only from
      `session-errors-a.jsonl` the output contains no "It last asked" line.
- [ ] AC3: A session whose last `user` record is a task-notification envelope or
      an `isMeta: true` record quotes the preceding typed prompt, not the
      envelope.
- [ ] AC4: `crates/verbatim/tests/hook.rs`'s existing p99 assertion passes over
      100 runs of every event with `VERBATIM_TEST_CORPUS` set.
- [ ] AC5: A store written by the previous build gains the column on open, is
      populated from `sessions.blob` with no transcript read, then completes a
      further `verbatim ingest` without error when it contains an `is_evicted`
      session, and `verbatim verify` passes over it.
- [ ] AC6: Two `SessionStart` renders against an unchanged store are
      byte-identical, asserted on a rooted fixture whose quoted turn moves under
      the new rule.

## Flagged assumptions

- The harness envelope shapes D-02 folds into the not-typed side are Claude
  Code's own and can change without notice - Likely; if wrong: a new envelope
  shape appears and starts being quoted as a prompt, undetected until someone
  reads a brief.
