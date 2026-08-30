# Phase 2: Egress Filter Sees What Is Sent - Context

Gathered: 2026-08-30
Feeds: /cad-plan 2

## Scope boundary

In: Moving redaction on the remote provider path from the already-serialized
JSON body to each `Message::content` before `json!` builds the document;
widening the rule set so the seven shapes the roadmap names are actually
caught in transcript text (mid-line header matching, four new value-shape
rules, `cookie` added to `SECRET_NAMES`); and a test harness that asserts over
the exact bytes `complete` hands to `net::post` rather than over hand-written
samples.

Out: Any redaction at ingest (a project-level Out of Scope, unchanged); the
derived recall projections and the injected brief, which are phase 4 and reuse
this widened rule set; the `local = true` path's behaviour, which is frozen by
D-13 and criterion AC4; `response_format` and the caller's `json_schema`
payload, which are not message content; the file-permission surface, which is
phase 3.

Deferred: None.

Plan shape: multiple plans, same phase - the rule-set widening inside
`egress.rs` and the wire-test harness plus fixture across `provider.rs` and
`tests/` are separable leases over different files.

## Durable decisions

- D-01 (Placement): The per-message redaction REPLACES the whole-body
  `egress::for_destination` call in `complete` rather than running in addition
  to it. `for_destination` keeps its `(&str) -> Cow<str>` shape and is applied
  to each `Message::content` before `json!` builds the document. Evidence:
  `crates/verbatim-core/src/observe/provider.rs:255-272`,
  `crates/verbatim-core/src/observe/egress.rs:88-98`,
  `crates/verbatim-core/tests/egress.rs:44-56` (the D-13 borrow test AC4
  freezes). If wrong: a second scan runs over escaped bytes on the wire path -
  the exact scan this phase calls inert - and rule 5 fires twice, once over
  content already carrying `[redacted]`, so no test can attribute a catch to a
  rule.

- D-02 (Placement): Redaction edits content STRINGS and never re-parses and
  re-serializes the finished body. This is what makes AC3's "same key order"
  free rather than earned: `serde_json` here has no `preserve_order` (no
  `indexmap` in `Cargo.lock:442-450`, no features enabled in either
  `Cargo.toml`), so the object map is a `BTreeMap` and key order is
  alphabetical regardless. Evidence:
  `crates/verbatim-core/src/observe/provider.rs:255-268`. If wrong: a
  parse-then-walk-then-reserialize implementation still passes the key-order
  criterion while re-encoding the whole document, which is the
  escaping-sensitive path this phase exists to leave.

- D-03 (Detector): Rule 3 must match header shapes MID-LINE. Unescaping alone
  does not make it fire on transcript text: `judgment.rs:619` prefixes every
  turn `turn_id=<id> <record_type>: ` and `:631-646` collapses each turn to one
  line, so the first colon on every line sits behind a prefix containing a
  space and an `=`, which `header_name` rejects. Evidence:
  `crates/verbatim-core/src/observe/egress.rs:210-218` (`line.find(':')`, first
  colon only). Measured 2026-08-30 over the 60 most recent transcripts under
  `~/.claude/projects` (21 MB, `grep -oE` occurrence counts): `Authorization:`
  36, `Cookie:` 7. If wrong: AC2's `Authorization: Bearer` and `Cookie` cases
  still fail after the serialization fix lands, and the phase closes with two
  of its seven shapes uncaught.

- D-04 (Detector): A mid-line header match stops at a BOUNDED terminator
  instead of running to end of line the way rule 3 does today. Evidence:
  `crates/verbatim-core/src/observe/egress.rs:196-202` (replaces from the colon
  through the whole line body),
  `crates/verbatim-core/src/observe/judgment.rs:631` (a whole turn is one
  line). If wrong: one `Cookie:` in a tool result erases the rest of that turn
  from the prompt, so the model is shown a truncated turn under an intact
  `turn_id=` anchor and can still anchor a claim to it.

- D-05 (Blast radius): Widening `redact` also widens `scrub` - one rule set,
  two entry points - so every provider error string, `runs.error` note and
  stored failed-answer blob passes through the new rules. This is accepted, not
  worked around: it is D-16's boundary doing its job. Evidence:
  `crates/verbatim-core/src/observe/egress.rs:110-116`,
  `crates/verbatim-core/src/observe/provider.rs:164-169`,
  `crates/verbatim-core/src/observe/judgment.rs:376-377` and `:556`,
  `crates/verbatim-core/src/observe/mod.rs:341-489`. If wrong: a new over-match
  changes wording stored in `runs.error` and `observations` raw text, breaking
  existing assertions in `crates/verbatim-core/tests/judgment.rs` and
  `crates/verbatim/tests/observations.rs` that read stored failure text.

- D-06 (Blast radius): The widened rules must stay correct on plain,
  non-JSON prose, because phase 4 reuses this exact rule set over recall
  excerpts, brief windows and `recall_get` output. No rule may assume its input
  is a JSON request body. Evidence: `.planning/ROADMAP.md:54` ("Depends on:
  Phase 2 (it reuses the widened rule set)"), `:57`,
  `crates/verbatim-core/src/observe/egress.rs:110-116`. If wrong: a rule
  written for request bodies mangles free prose and phase 4 inherits a rewrite
  instead of a knob.

## Decisions

- D-07 (Placement): Only message content is redacted. `model`,
  `response_format` and the caller's `json_schema` payload are left alone.
  Evidence: `crates/verbatim-core/src/observe/provider.rs:255-267`,
  `crates/verbatim-core/src/observe/judgment.rs:655-695` (`schema()`). If
  wrong: a schema property name containing a fragment comes back `[redacted]`
  inside `response_format`, and a strict remote endpoint answers 400 - the same
  failure class the 2026-08-21 DeepSeek probe recorded at `provider.rs:48-64`.

- D-08 (Placement): The system instruction turn is redacted along with the
  user turn, and today's instructions survive it byte-for-byte. Evidence:
  `crates/verbatim-core/src/observe/judgment.rs:323`, `:698-717`; the schema's
  key names (`name`, `schema`, `type`, `additionalProperties`, `required`,
  `properties`, `topic`, `outcome`, `decisions`, `learned`, `unresolved`,
  `description`, `items`, `maxItems`, `turn_id`, `text`) match no fragment in
  `egress.rs:54-66`. If wrong: a later prompt edit that introduces a matching
  word silently mutates the instruction turn and the model is asked to satisfy
  a schema that no longer says what it said.

- D-09 (Detector): Four of the seven required shapes are not name-keyed and
  need new value-shape rules - a bare JWT, a GitHub token, a connection URL
  carrying userinfo, and a space-separated `--token` flag (rule 5 keys on `=`
  only). `cookie` also joins `SECRET_NAMES`. Evidence:
  `crates/verbatim-core/src/observe/egress.rs:54-66` (no `cookie`), `:285-341`
  (rule 5 scans for `=`), `.planning/ROADMAP.md:34`. Measured 2026-08-30, same
  60-file / 21 MB sample: `--token[ =]` 5, `://user@`-shaped userinfo 8,
  `ghp_[A-Za-z0-9]{20,}` 0, `github_pat_` 0, full `eyJ....eyJ...` JWT 0, bare
  `eyJ[A-Za-z0-9_-]{8,}` 4. If wrong: the phase ships a serialization fix only
  and AC2 fails on four of seven shapes.

- D-10 (Detector): The GitHub-token rule keys on a PINNED prefix list -
  `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`, `github_pat_` - held in a named
  constant array beside `SECRET_NAMES` and commented as a snapshot that will go
  stale. Chosen over a generic `gh?_` shape, which would match unrelated text.
  Evidence: the codebase names no GitHub prefix anywhere (the analyzer's
  `needs_research`, settled by the user this pass). If wrong: a prefix GitHub
  invents later is missed until someone updates the list.

- D-11 (Detector): The new rules are hand-written byte scanners; no regex
  crate is added. Evidence: `crates/verbatim-core/Cargo.toml:17-25` (eight
  direct deps, no `regex`), `Cargo.lock` has no `regex` entry, and every
  existing rule in `egress.rs:132-341` is a manual scan over `as_bytes()`. If
  wrong: a dependency lands in the crate that ships as the single static
  binary, for a module whose whole design is index arithmetic.

- D-12 (Detector): Fixtures keep this repo's short unrealistic sentinel
  convention (`ghp_abc123XYZ`, `sk-VERBATIMEGRESS-71c4a8-do-not-log`), so the
  value-shape rules get LOOSE length floors and over-match more. Chosen over
  realistic-length values, which risk GitHub push protection on this public
  repo. Evidence: `crates/verbatim-core/tests/provider.rs:604-605`,
  `crates/verbatim-core/tests/egress.rs:17`,
  `crates/verbatim-core/src/observe/egress.rs:74-80` (`MIN_CREDENTIAL_CHARS`,
  the existing length-floor precedent). If wrong: a floor set too low fires on
  ordinary prose and AC7's tolerated-failure statement stops being a tolerable
  one.

- D-13 (Detector): Each new value-shape rule gets its own named marker
  constant, the way `REDACTED_PRIVATE_KEY` does, rather than reusing the bare
  `REDACTED`. Evidence:
  `crates/verbatim-core/src/observe/egress.rs:32-34` (the module's stated
  promise that every rule leaves a marker naming what went), `:68-72`;
  `crates/verbatim-core/tests/egress.rs:70` and
  `crates/verbatim-core/tests/provider.rs:634` already assert on those
  constants. If wrong: a JWT vanishing into an unlabelled `[redacted]` leaves a
  reader unable to tell which rule fired, and the module doc becomes false
  in-tree.

- D-14 (Testing): AC1's "same construction the real path uses" means driving
  `provider::complete` (or `judgment::judge`) against `testkit::HttpStub` and
  reading `stub.requests()`, NOT exporting a new body-builder function for
  tests. Evidence: `crates/verbatim-core/src/testkit.rs:602-752` (the stub
  records head and body as text),
  `crates/verbatim-core/tests/provider.rs:204-207` (`body_of`), `:596-647` (the
  existing on-the-wire D-13 test),
  `crates/verbatim-core/tests/judgment.rs:496-532`. If wrong: a test asserting
  over a body built by a helper the product does not call is the same failure
  this phase exists to fix - passing on a sample rather than on the wire.

- D-15 (Testing): "A secret planted in a transcript turn" means a new fixture
  JSONL ingested through the real `ingest::run` and judged, following the
  `Bench` pattern - not a `Message::user` string handed straight to `complete`.
  Evidence: `crates/verbatim-core/tests/judgment.rs:28-36`, `:43-105`,
  `tests/fixtures/session-*.jsonl`,
  `crates/verbatim-core/src/index/text.rs:55-70`. If wrong: the test never
  exercises `text::project` + `one_line`, which is exactly where the `turn_id=`
  prefix defeats rule 3, so the strongest failing-test-before-the-fix is never
  written.

- D-16 (Testing): The new wire tests carry `#[cfg(feature = "testkit")]`
  PER-TEST; `tests/egress.rs` does not get a file-level
  `#![cfg(feature = "testkit")]`. Evidence:
  `crates/verbatim-core/tests/egress.rs:231` (the ingest guard, currently
  ungated), `crates/verbatim-core/tests/provider.rs:12-14` and `:96-99` (the
  per-item gating pattern), `.planning/CAPTURE.md:149` (a recorded open item: a
  file-level gate makes `cargo test --test <name>` compile an empty binary and
  report green). If wrong: `cargo test --test egress` goes green running zero
  tests, and AC6's ingest guard - the one thing between this widened detector
  and the ingest path - stops executing unnoticed.

## Acceptance criteria

- [ ] AC1: A test drives `provider::complete` against `testkit::HttpStub` with
      `local` absent, reads the recorded body, and asserts a planted secret is
      absent from it. Reverting the per-message redaction to today's whole-body
      call makes that test fail.
- [ ] AC2: For each of seven shapes planted in an ingested fixture transcript -
      `Authorization: Bearer`, a JSON `"password"` pair, a space-separated
      `--token` flag, a `Cookie` header, a bare JWT, a GitHub token, and a
      connection URL carrying userinfo - the secret's characters do not appear
      in the body recorded by the stub, and the body contains that rule's named
      marker constant.
- [ ] AC3: The recorded body parses as JSON, and its message count, the `role`
      of each message in order, and its top-level key sequence equal those of a
      build with redaction removed.
- [ ] AC4: With `local = true`, the recorded body is byte-identical to the
      unredacted body, and the existing D-13 borrow test in
      `crates/verbatim-core/tests/egress.rs` passes unmodified.
- [ ] AC5: A turn containing `Cookie: <secret>` mid-line renders with the text
      following the redacted value still present in the recorded body, and that
      turn's `turn_id=` anchor intact.
- [ ] AC6: `nothing_in_the_ingest_path_names_the_egress_filter` passes, and
      `cargo test --test egress` reports a non-zero test count.
- [ ] AC7: `egress.rs`'s module docs state the over-matching direction as the
      tolerated failure: a name-substring false positive costs detail in one
      message.

## Flagged assumptions

- Fixture sentinels are short and unrealistic (D-12), so the value-shape rules'
  length floors are chosen against sentinels rather than against real
  credentials - Likely; if wrong: a rule requiring realistic length never fires
  on a sentinel and AC2's assertion passes because rule 5 caught the
  surrounding `NAME=` instead, which is an assertion about nothing. The planner
  should make each value-shape test plant its shape with NO surrounding
  name-keyed context, so only the rule under test can catch it.
- The pinned GitHub prefix list (D-10) is a 2026-08-30 snapshot with no
  in-repo source of truth - Likely; if wrong: a newer prefix is missed silently
  until someone notices.
