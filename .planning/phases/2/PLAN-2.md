---
phase: 2
plan: 2
requirements:
  - PRIV-01
  - PRIV-03
files:
  - crates/verbatim-core/src/observe/provider.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/egress.rs
  - tests/fixtures/session-secrets.jsonl
  - tests/fixtures/README.md
---

# Phase 2: Egress Filter Sees What Is Sent - Plan 2

## Goal

The redaction on the remote provider path runs over each message's content
before `json!` builds the document, and the egress tests assert over the exact
bytes `complete` hands to `net::post` - obtained by driving the real call
against a loopback stub - rather than over hand-written samples.

## Must be true when done

- With `local` absent from the config, a secret planted in an ingested
  transcript turn is absent from the request body the stub recorded. Putting the
  whole-body `egress::for_destination` call back makes that test fail.
- Each of the seven shapes planted in that fixture - `Authorization: Bearer`, a
  JSON `"password"` pair, a space-separated `--token` flag, a `Cookie` header, a
  bare JWT, a GitHub token and a connection URL carrying userinfo - is absent
  from the recorded body, and the body carries that rule's named marker.
- The recorded body still parses as JSON, with the same message count, the same
  roles in the same order and the same top-level key sequence as the same call
  makes with redaction off, and the system instruction turn is byte-identical
  between the two.
- A turn carrying `Cookie: <secret>` mid-line still shows the text that followed
  the value, under an intact `turn_id=` anchor.
- With `local = true`, every planted secret is still in the recorded body, and
  `a_local_destination_sends_the_body_byte_identical` passes unmodified.
- `cargo test -p verbatim-core --features testkit --test egress` reports a
  NON-ZERO test count and `nothing_in_the_ingest_path_names_the_egress_filter`
  is one of the tests that ran.

## Context

CONTEXT locks: D-01 (per-message redaction REPLACES the whole-body call;
`for_destination` keeps its `(&str) -> Cow<str>` shape), D-02 (content strings
are edited, the finished body is never re-parsed and re-serialized), D-07 (only
message content - `model`, `response_format` and the caller's `json_schema` are
left alone), D-08 (the system turn is redacted too and today's instructions
survive it byte-for-byte), D-14 (drive `provider::complete` or `judgment::judge`
against `testkit::HttpStub` and read `stub.requests()`; never export a
body-builder for tests), D-15 (a new fixture JSONL ingested through the real
`ingest::run` and judged, following `tests/judgment.rs`'s `Bench`), D-16
(per-test `#[cfg(feature = "testkit")]`; `tests/egress.rs` gets NO file-level
`#![cfg(feature = "testkit")]`, because that makes `cargo test --test egress`
compile an empty binary and report green - a recorded open item at
`.planning/CAPTURE.md:149`).

The two-arm claim tasks 3 and 4 assert - filtered with `local` absent or false,
unfiltered with `local = true` - is the one v0.1.0 phase 7's UAT already signed
off (`_archive-v0.1.0/7/UAT.md`); this phase changes only WHERE the
filter runs, so that arm must still read the same.

Runs AFTER plan 1: the marker constants task 4 asserts on are created there.

## Tasks

### Task 1: Filter each message's content instead of the finished body

- **Files:** crates/verbatim-core/src/observe/provider.rs (start at `complete`;
  the module header's two-boundaries section changes too)
- **Action:** `complete` builds `request` with `json!`, calls
  `request.to_string()`, and then hands that string to
  `egress::for_destination`. By then every quote and newline in the transcript
  is escaped, which is why `redact_header_lines` sees one line with no `'\n'`
  and `redact_json_pairs` walks past every `\"` and consumes the whole
  transcript as one candidate name. Apply `egress::for_destination` to each
  `Message`'s `content` inside the `messages` map instead, before `json!` builds
  the document, and DELETE the call on the serialized body - a second scan over
  escaped bytes is the exact scan this phase calls inert, and leaving both means
  the assignment rule fires twice, once over content already carrying a marker,
  so no test can attribute a catch to a rule (D-01). `config.provider_local()`
  and `credential` are the same two arguments the deleted call passed;
  `for_destination` keeps its signature. Do not parse and re-serialize the
  finished body (D-02): key order is alphabetical here whatever happens, because
  this workspace's `serde_json` has no `preserve_order` feature and no
  `indexmap` in `Cargo.lock`, so re-encoding buys nothing and puts the whole
  document back through the escaping-sensitive path. Map over EVERY message, the
  `system` instruction turn included (D-08), and over nothing else: `model`,
  `response_format` and the caller's `json_schema` payload stay untouched,
  because a schema property name coming back `[redacted]` inside
  `response_format` is a 400 from a strict endpoint - the failure class the
  2026-08-21 DeepSeek probe recorded in this file's own doc comment (D-07).
  Update the module header's "Two boundaries every request crosses" so it says
  what now goes through the filter.
- **Verify:** `cargo test --workspace --all-features` passes, in particular
  `crates/verbatim-core/tests/provider.rs::the_body_on_the_wire_is_filtered_for_remote_and_whole_for_local`
  and `crates/verbatim-core/tests/egress.rs::a_local_destination_sends_the_body_byte_identical`,
  both unmodified; and `grep -n for_destination
  crates/verbatim-core/src/observe/provider.rs` shows exactly one call site,
  inside the messages map, with nothing applied to `request.to_string()`'s
  result.

### Task 2: A fixture transcript carrying all seven shapes

- **Files:** tests/fixtures/session-secrets.jsonl,
  crates/verbatim-core/src/testkit.rs (`TRANSCRIPT_FIXTURES` and
  `ROOTED_FIXTURES`), tests/fixtures/README.md
- **Action:** Add one rooted transcript fixture whose turns carry all seven
  shapes AC2 names, and register it in `TRANSCRIPT_FIXTURES` and in
  `ROOTED_FIXTURES` under a FOURTH project key, with a row in the README's file
  table saying what it exists to exercise. Each secret is a short unrealistic
  sentinel in this repo's existing convention (`ghp_abc123XYZ`,
  `sk-VERBATIMEGRESS-71c4a8-do-not-log`), distinct from every other so a test
  can assert on one at a time, and short on purpose: realistic-length values in
  a public repo risk GitHub push protection (D-12). Four of the seven - the bare
  JWT, the GitHub token, the userinfo URL and the space-separated `--token`
  flag - must sit with NO name-keyed text beside them, so that the rule under
  test is the only thing that could have caught them; a `GITHUB_TOKEN=ghp_...`
  spelling would be caught by the assignment rule and the assertion would be
  about nothing. The `Cookie` shape goes MID-LINE inside a turn with ordinary
  prose after the value on the same line, which is what AC5 reads. The fixture
  needs at least six turns that project non-empty text, because
  `observe::cost::MIN_TURNS` is 6 and a shorter session is skipped unjudged.
  Follow the constraints `crates/verbatim-core/tests/fixtures.rs` already
  enforces: UTF-8, no `\r` byte, a trailing `\n`, every record carrying `cwd`
  exactly `{{ROOT}}/<project>` plus `sessionId`, `uuid`, `timestamp` and `type`,
  and no occurrence of `/data/code/verbatim`. Make it INERT for every existing
  assertion the way `session-envelope.jsonl` was (commit 875ad04, which touched
  only these same three files): a project key nothing else scopes against, and a
  vocabulary sharing none of the corpus's counted tokens - no `SearchManager`,
  no `brillig`, no `cargo`, none of the paths the entity tests match on.
- **Verify:** `cargo test --workspace --all-features` passes with no test file
  edited, in particular
  `crates/verbatim-core/tests/fixtures.rs::line_endings_match_the_measured_corpus`
  and `::the_rooted_fixtures_name_a_test_owned_root_and_two_projects`, and the
  corpus-wide benches in `tests/index.rs`, `tests/recall.rs`, `tests/reindex.rs`
  and `crates/verbatim/tests/{cli,mcp,recall_cli}.rs`, which all ingest every
  member of `TRANSCRIPT_FIXTURES`; and `grep -c` over the new fixture shows one
  occurrence of each of the seven sentinels.

### Task 3: Assert over the bytes the real call puts on the wire

- **Files:** crates/verbatim-core/tests/egress.rs
- **Action:** Add the wire harness and the body-fidelity assertions to this
  file. Each new item carries its own `#[cfg(feature = "testkit")]`; the file
  does NOT get a file-level `#![cfg(feature = "testkit")]`, because that makes
  `cargo test --test egress` compile an empty binary and report green, which
  would silently retire `nothing_in_the_ingest_path_names_the_egress_filter` -
  the one guard between this widened detector and the ingest path (D-16). The
  harness follows `crates/verbatim-core/tests/judgment.rs`'s `Bench` (its
  `bench`, `ingest`, `observed` and `config` helpers): copy the new fixture in
  with `testkit::copy_rooted_fixture_into`, archive it through the real
  `ingest::run`, mark `session_meta.is_final`, run `observe::observe_new`, then
  drive `judgment::judge` against a `testkit::HttpStub` serving one
  `testkit::chat_completion` whose content is a valid judgment - `topic`,
  an `outcome` from `judgment::OUTCOMES`, and three empty claim arrays - so one
  request is enough and the stub is not asked for a retry it has no response
  for. Read the request back with `stub.requests()` and take the body after the
  first `\r\n\r\n`, the way `tests/provider.rs::body_of` and
  `tests/judgment.rs::the_request_shows_the_model_each_turn_beside_its_real_turn_id`
  already do. Build no body-building helper for the tests to call: a body the
  product does not build is the same failure this phase exists to fix (D-14).
  Two configs, differing only in the `local` key: one that OMITS it entirely,
  which is AC1's arm and the arm a forgotten declaration reaches, and one with
  `local = true`, which is the unredacted build to compare against. Assert: a
  secret whose shape the assignment rule cannot catch - a JSON `"password"` pair
  or the mid-line `Authorization:` header - is absent from the `local`-absent
  body and present in the `local = true` body; the `local`-absent body parses as
  JSON; its `messages` array has the same length and the same `role` values in
  the same order as the `local = true` one; its top-level key sequence, read off
  the body TEXT in order of appearance rather than off a parsed map (which is a
  `BTreeMap` and would sort both sides into agreement without testing
  anything), equals the other's; and `messages[0].content`, the system
  instruction turn, is byte-identical between the two, which is D-08's claim
  that today's instructions survive redaction.
- **Verify:** `cargo test -p verbatim-core --features testkit --test egress`
  reports a non-zero test count with every test passing, and
  `cargo test -p verbatim-core --test egress` (no feature) still compiles and
  runs `nothing_in_the_ingest_path_names_the_egress_filter`; reverting task 1 -
  putting `egress::for_destination` back on `request.to_string()`'s result -
  makes the AC1 assertion fail, and that revert is undone before the task is
  done.

### Task 4: Assert all seven shapes are gone and each rule said so

- **Files:** crates/verbatim-core/tests/egress.rs
- **Action:** Add the per-shape assertion over the same recorded body the task 3
  harness produces, one gated test driving the same fixture through
  `judgment::judge` with `local` absent. For each of the seven shapes - the
  `Authorization: Bearer` header, the JSON `"password"` pair, the
  space-separated `--token` flag, the `Cookie` header, the bare JWT, the GitHub
  token and the connection URL carrying userinfo - assert that its sentinel's
  characters do not appear anywhere in the body, and that the body contains the
  marker constant `verbatim_core::observe::egress` exports for the rule that
  catches that shape (plan 1 added one per new rule; the header rule keeps
  `config::REDACTED`). Assert the same seven sentinels are ALL present in the
  `local = true` body, so a fixture that failed to plant one fails loudly rather
  than passing as a catch. For AC5, on the `Cookie` turn specifically: the prose
  that followed the cookie value on that same line is still in the body, and
  that turn's `turn_id=` anchor is still there - a rule that ran to end of line
  would leave the anchor standing over a turn the model was shown a fragment of,
  and it could still hang a claim on it. Read the anchors from the store the way
  `tests/judgment.rs`'s `turn_ids` helper does rather than hardcoding ids.
- **Verify:** `cargo test -p verbatim-core --features testkit --test egress`
  passes with the new test present, and removing any one of plan 1's rules from
  `egress::redact` makes exactly that shape's assertion fail while the other six
  still pass.

## Notes

Sequential after plan 1, not parallel: the two plans declare no file in common,
but task 4 asserts on marker constants plan 1 creates. The CONTEXT `Plan shape`
directive asked for two plans and this is two plans; the ordering is the part it
did not say.

`tests/fixtures/session-secrets.jsonl` is the planner's name for the fixture and
`project-delta` the planner's choice of fourth project key; either may be spelled
differently as long as the registration and README row follow it.

`crates/verbatim/tests/hook.rs` fails nondeterministically under full-workspace
parallel load - a different subset each run, all passing in isolation. It is
pre-existing (`.planning/CAPTURE.md`, phase 7) and untouched by this phase; a
`cargo test --workspace --all-features` run that fails only there is not this
plan's regression.
