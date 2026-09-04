---
phase: 4
plan: 2
requirements:
  - PRIV-03
  - RCL-09
  - OBS-03
files:
  - crates/verbatim-core/src/recall/context.rs
  - crates/verbatim-core/tests/recall.rs
  - crates/verbatim-core/src/recall/get.rs
  - crates/verbatim/src/cmd/mcp/tools.rs
  - docs/json-shapes.md
  - crates/verbatim-core/src/inject/brief.rs
  - crates/verbatim-core/tests/inject_brief.rs
  - crates/verbatim-core/src/observe/judgment.rs
  - crates/verbatim-core/tests/judgment.rs
---

# Phase 4: Redacted Recall - Plan 2

## Goal

The other three derived projections - `recall_context`'s window, `recall_get`'s
body and the `SessionStart` brief's quotes - run through the same filter PLAN-1
built, and a successful judgment's `topic`, `outcome` and claim `text` are
filtered before they reach SQLite, closing the gap where only failed responses
were scrubbed.

## Must be true when done

- With the boolean on, a `recall_context` window over the
  `session-secrets.jsonl` session carries the named marker constant and none of
  the planted sentinels; with it absent, every turn's text is what it is today.
- With the boolean on, `recall_get`'s body for the `Authorization` turn carries
  the marker and no sentinel characters while the rest of that archived JSON
  line is still there; with it absent the body is the fixture's own line byte
  for byte.
- With the boolean on, a `SessionStart` brief for that project quotes the
  marker rather than the sentinel, and two renders against an unchanged store
  are still byte-identical.
- A provider answer whose `topic`, `outcome` and one claim `text` each carry a
  planted secret produces `observations` rows in which a direct SQL read of all
  three columns finds the marker and none of the secret's characters, while
  every claim's `turn_id` still names a real turn of that session.
- The four `--json` shapes gain no key in either setting, and every existing
  test in the files this plan touches passes unmodified.

## Context

Locked: D-04 (with the knob on `recall::get::Record::body` carries filtered
bytes through a `String` round trip done ONCE in the core, not per renderer),
D-06 (the brief filters BEFORE the `brief_chars` budget is spent), D-11 (the
judgment filter runs inside `store` or on the `Judgment` immediately before it,
using the `credential` and `&Config` already in scope at `judgment.rs`'s `store`
call site), D-03 (the unconditional entry point, never `for_destination` - the
judgment harness in `tests/judgment.rs` declares `local = true` and the write
filter must still fire), D-08 (no `--json` shape gains a field in either
setting: `tests/mcp.rs` pins the exact key set of a `recall_get` record). The
config accessor and the egress entry point come from PLAN-1 tasks 2 and 3; this
plan does not create them. Out and untouched: `verbatim export` (D-12) and
`verbatim observations` (D-13). Measured: 0 of 47,157 archived records read out
of their blobs are invalid UTF-8, so D-04's lossy step costs nothing observable
on today's corpus; `egress::scrub` over the largest real projection (50,325
chars) costs 0.563 ms against the brief's 10 ms p99.

## Tasks

### Task 1: Filter the context window's turn text

- **Files:** crates/verbatim-core/src/recall/context.rs (`window`, `fill_text`),
  crates/verbatim-core/tests/recall.rs
- **Action:** `context::window` already takes the `&Config`; derive PLAN-1's
  filter value there and thread it into `fill_text`, which is where each turn's
  text is cut through `excerpt::of_record` with an empty `Query`. The filter
  therefore lands on the full projection before the window cut, exactly as it
  does for a search excerpt, because it is the same `of_record` argument (D-01,
  D-02) - do not add a second scrub over `ContextTurn::text` afterwards, which
  would double-apply the rules and could take a marker's own words as a value.
  Rule 1 is fed `config.provider_api_key()` (D-10). Nothing about scoping,
  exclusion, `at_session_start`/`at_session_end` or `continues_from` changes.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --features testkit --test recall` passes a new test, added
  beside the existing context tests and reusing that file's `bench()`, showing
  that a window anchored on a turn of `session-secrets.jsonl` carries
  `sk-VBEGRESS-authz-9f2` with the knob absent and
  `verbatim_core::config::REDACTED` with none of that sentinel's characters
  when a `verbatim.toml` turning the knob on is loaded through
  `Config::load_from`; every existing test in that file passes unmodified.

### Task 2: Filter the archived body recall_get hands back

- **Files:** crates/verbatim-core/src/recall/get.rs (`records`, `Record::body`),
  crates/verbatim/src/cmd/mcp/tools.rs (`run_get`), docs/json-shapes.md
- **Action:** In `recall::get::records`, where a range read off the blob becomes
  `Record::body`, convert through a lossy `String`, put it through PLAN-1's
  entry point and store the result back as bytes - but ONLY when the knob is on.
  With the knob absent the bytes must be the blob's own with no conversion at
  all, because that is the byte-exactness AC2 and this module's own doc claim.
  Once in the core and not per renderer (D-04): `verbatim show` writes the bytes
  with `write_all` and the MCP tool renders them with `String::from_utf8_lossy`,
  and a conversion done separately in each would let the terminal `body` and the
  MCP `body` diverge. Update the three places that currently promise an
  unqualified verbatim body - the `Record::body` doc's "`Vec<u8>` and not
  `String` on purpose" paragraph, `run_get`'s "This is the one path that hands
  the truth back" doc, and the `body` bullet in `docs/json-shapes.md` - each
  with one sentence naming the knob and saying the archive itself is untouched.
  No key is added to any document and no rendering code changes (D-08): the
  filtered outputs announce themselves through the in-band markers alone.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --features testkit --test recall` passes a new test showing
  that with the knob absent the body of the `Authorization` turn equals that
  line of `tests/fixtures/session-secrets.jsonl` byte for byte, and with the
  knob on it contains `verbatim_core::config::REDACTED`, none of
  `sk-VBEGRESS-authz-9f2`, and still contains that record's `"uuid"` and
  `"type"` fields so the body is still the archived line rather than a stub.
  `cargo test --manifest-path /code/verbatim/Cargo.toml -p verbatim --features
  testkit --test mcp` passes with that file unmodified, including its record
  key-set assertion.

### Task 3: Filter the brief's quotes before the budget is spent

- **Files:** crates/verbatim-core/src/inject/brief.rs (`session_start`,
  `render`, `last_exchange`), crates/verbatim-core/tests/inject_brief.rs
- **Action:** `session_start` has the `&Config`; derive PLAN-1's filter value
  there and carry it to `last_exchange`, where each quote's text is produced by
  `excerpt::of_record`, so the filter has run before `render` measures
  `chars(&full)` against the budget and before `shares`/`clip` cut the quotes
  (D-06). A marker consuming budget a quotation would otherwise have had is the
  decided behaviour, not a defect. Never filter after `clip`: a marker cut in
  half leaves a nameless partial value the shape rules cannot catch, which would
  make the brief the one of the four outputs where AC1 fails on a long turn. The
  brief's other blocks - the head line, the branch, the index pointer - are
  built by this file and not from archived text, and are not filtered. Nothing
  about INJ-02's byte identity, INJ-04's `remember` of quoted turn ids, or the
  `MAX_BRIEF_CHARS` clamp changes. Cost is inside budget by D-06's measurement:
  0.563 ms worst case per projection, roughly 1.1 ms for the two quotes against
  the 10 ms p99 `crates/verbatim/tests/hook.rs` asserts.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --features testkit --test inject_brief` passes a new test that
  archives `session-secrets.jsonl` through the bench's rooted-fixture helper and
  renders the brief for that project: with the knob absent the "It last asked"
  line carries `sid-VBEGRESS-qcrumb-1f9`; with a `verbatim.toml` turning the
  knob on, the same line carries `verbatim_core::config::REDACTED` and none of
  that sentinel's characters, and two renders against the unchanged store are
  byte-identical to each other. Every existing test in that file passes
  unmodified.

### Task 4: Filter a successful judgment before it reaches SQLite

- **Files:** crates/verbatim-core/src/observe/judgment.rs (`store` and its call
  site), crates/verbatim-core/tests/judgment.rs
- **Action:** With the knob on, put a validated `Judgment`'s `topic`, `outcome`
  and every claim `text` of the three lists through PLAN-1's entry point before
  `store` writes them, using the `credential` and `&Config` already in scope at
  the `store(conn, session_key, config, &judgment, tokens, reservation)` call
  (D-11). Through the unconditional entry point and never `for_destination`: a
  provider declared `local` still echoes text into a column `recall_search`
  hands the model and `verbatim observations` prints, so the declaration about
  where the request went says nothing about this write. Every claim's `turn_id`
  is untouched (OBS-03) - the anchor is what makes a claim auditable, and a
  filter that moved it would be worse than the leak. The failed path is not
  touched: `store_failure`'s `raw` already goes through `egress::scrub`
  unconditionally under D-16 and must keep doing so whatever the knob says, and
  the knob must not gate it. `MAX_CLAIMS`, the anchor validation, the
  reservation condition and the `raw = NULL` clearing all stay exactly as they
  are.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --features testkit --test judgment` passes a new test built on
  that file's `bench()`, `HttpStub` and `Row` helpers, whose stubbed answer
  carries a distinct planted sentinel in `topic`, in `outcome` and in one
  claim's `text`: with the knob on, a direct SQL read of `topic`, `outcome`,
  `decisions`, `learned` and `unresolved` finds
  `verbatim_core::config::REDACTED` and none of the three sentinels' characters
  while `Row::anchors()` still returns real `turns.id` values of that session;
  with the knob absent the same three columns hold the sentinels unchanged. The
  test runs against that file's existing config helper, which declares `local =
  true`, so a filter routed through `for_destination` fails it. Every existing
  test in that file passes unmodified.

## Notes

- Runs after PLAN-1: tasks 1-4 all call the egress entry point PLAN-1 task 3
  adds and read the config accessor PLAN-1 task 2 adds. No file is shared with
  PLAN-1.
- `crates/verbatim/src/cmd/mcp/tools.rs` and `docs/json-shapes.md` are declared
  for one doc sentence each in task 2 and for nothing else: both currently
  promise an unqualified verbatim body, and the knob makes that promise
  conditional.
