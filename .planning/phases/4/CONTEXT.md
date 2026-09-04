# Phase 4: Redacted Recall - Context

Gathered: 2026-09-04
Feeds: /cad-plan 4

## Scope boundary

In: One new `verbatim.toml` table carrying a single optional boolean, default
off, consulted at the four projection entry points that already take `&Config`
- `recall::search::run`, `recall::context::window`, `recall::get::records` and
`inject::brief::session_start` - and routing their output through phase 2's
widened rule set; the same filter applied to successful judgment `topic`,
`outcome` and claim `text` before `judgment::store` writes them to SQLite, plus
a read-side filter on `recall_search`'s observation branch so claim rows
written before the knob flipped are covered too; and a per-invocation raw flag
on the terminal read commands so the owner can still read their own archive
unfiltered.

Out: Any redaction at ingest (a project-level Out of Scope, unchanged - the
`nothing_in_the_ingest_path_names_the_egress_filter` guard stays); the rule set
itself, which phase 2 widened and D-06 already committed to keeping correct on
plain prose; `verbatim export`, which stays unfiltered with the notice it has;
`verbatim observations` and `observations --json`, which print what the column
holds; the `--json` shapes, which gain no field in either setting; the
`provider.local` declaration's meaning on the provider path, frozen by D-13.

Deferred: None.

Plan shape: multiple plans, same phase - three separable leases over different
files, the shape phases 1-3 all used. (a) the config table plus the four core
call sites (`config.rs`, `recall/{search,context,get}.rs`, `inject/brief.rs`);
(b) the judgment write filter plus the observation read filter
(`observe/judgment.rs`, `recall/search.rs`'s observation branch); (c) the
terminal raw flag plus the golden and round-trip test harness
(`cmd/{search,show}.rs`, `tests/`).

## Durable decisions

- D-01 (Placement): The knob is consulted at the four entry points that
  already carry `&Config` - `recall::search::run`, `recall::context::window`,
  `recall::get::records` and `inject::brief::session_start` - and NOT inside
  the shared projection `recall::excerpt::of_record` / `excerpt::attach`.
  Evidence: `crates/verbatim-core/src/recall/excerpt.rs:66` (`attach(conn,
  query, hits)`) and `:143` (`of_record(query, record)`) take no config;
  `crates/verbatim-core/src/recall/search.rs:340`,
  `crates/verbatim-core/src/recall/context.rs:78`,
  `crates/verbatim-core/src/recall/get.rs:100` and
  `crates/verbatim-core/src/inject/brief.rs:70` already do. If wrong:
  filtering inside `of_record`/`attach` also filters a fifth surface the
  criteria never name - `inject/prompt.rs:427` builds the per-prompt
  injection's excerpts through `excerpt::attach`, and `verbatim replay`
  re-scores through `search::run` - so AC2's byte-identity is asserted over
  four outputs while a fifth changes with no test on it. [corrected by plan-1
  deviation: the fifth surface writes to hook stdout, which is the same
  model-context egress boundary, so it is filtered too - five entry points
  resolve a `Redaction`, not four; the placement half of this decision stands]

- D-02 (Placement): The filter runs over the FULL projection BEFORE the
  240-character excerpt window is cut, never over the window afterwards.
  Evidence: `crates/verbatim-core/src/recall/excerpt.rs:149`
  (`window(query, &flatten(&text::project(&value)))`), `:178-201` (the window
  centres on the first query token), `:112` (`EXCERPT_CHARS = 240`);
  `crates/verbatim-core/src/observe/egress.rs:358-432` - the header rule needs
  the secret's NAME and its VALUE in the same string. Measured 2026-09-04 over
  the live store at `~/.local/share/verbatim/verbatim.db` (683,255 turns),
  scrubbing the real projection of every turn of the 60 most recent sessions
  (5,412 records, 12.9 MB): 0.18% of projections change, against 2.49% of raw
  record bodies. If wrong: a secret whose `Authorization:` or `--token` name
  sits outside the cut keeps its raw value inside the excerpt, because the rule
  that would have caught it never sees the name - and AC1 can pass anyway when
  the planted secret happens to land whole inside the window.

- D-03 (Destination): The recall filter does NOT go through
  `egress::for_destination`'s `local` gate; it calls an unconditional entry
  point, because the destination is the model's context and not the configured
  provider. Evidence: `crates/verbatim-core/src/observe/egress.rs:193-208`
  (`local == true` returns `Cow::Borrowed`, unfiltered) against `:6-8`, whose
  module doc already lists "a recall excerpt on its way into an injected brief"
  as a `for_destination` caller; `DESIGN-BRIEF.md:355` keys redaction on the
  PROVIDER destination, and this phase reclassifies hook and MCP output as its
  own egress boundary. Phase 2 D-13 kept `local` a declaration rather than an
  observation. If wrong: every user with `provider.local = true` turns the knob
  on and gets raw text in all four outputs - the knob is silently inert for
  exactly the user who runs a local model for privacy reasons - and AC1 passes
  only because its fixture config leaves `local` unset.

- D-04 (Placement): With the knob on, `recall::get::Record::body` carries
  filtered bytes, so the one path documented as byte-exact goes through a
  `String` round trip; the conversion happens once in the core, not separately
  in each renderer. Evidence: `crates/verbatim-core/src/recall/get.rs:47-60`
  (the "`Vec<u8>` and not `String` on purpose ... the archive is verbatim"
  doc); `crates/verbatim/src/cmd/show.rs:9-14` and `:92-97` (`write_all`, not
  `println!`); `docs/json-shapes.md:120-122`. Measured 2026-09-04 over 400 real
  sessions / 47,157 archived records read out of their blobs: 0 are not valid
  UTF-8, so the lossy step costs nothing observable on today's corpus. If
  wrong: `verbatim show`'s human mode stops being the byte-exact path its own
  module doc claims, or the conversion is duplicated per renderer and the MCP
  `body` and the terminal `body` diverge.

- D-05 (Observations): The observation branch of `recall_search` filters its
  `excerpt` on READ as well as at write time, so claim rows written before the
  knob was turned on are covered with no regeneration and no user action.
  Evidence: `crates/verbatim-core/src/recall/search.rs:472-478` ("**No excerpt
  read.** `excerpt::attach` is deliberately not called: the text IS the claim")
  and `:559` (`excerpt: row.get(5)?`, straight off the `observations` claim
  column via `json_each`); markers are whitespace-free and idempotent by
  `crates/verbatim-core/src/observe/egress.rs:129-138`, so a second scrub over
  an already-filtered new row is a no-op. Chosen over write-only filtering with
  a `verbatim observations regenerate` instruction in `doctor`. If wrong: with
  the knob on, `recall_search --kind observation` hands the model raw claim
  text from every row generated before the flip, and AC1 passes only because
  its observation was generated after the knob was set - a green test over a
  live leak.

- D-06 (Injection): With the knob on, the brief filters BEFORE the
  `brief_chars` budget is spent, so a marker consumes budget a quotation would
  otherwise have had. Evidence:
  `crates/verbatim-core/src/inject/brief.rs:154-186` (`render` measures
  `chars(&full)` against the budget), `:281-310` (`shares`/`clip` cut the
  quotes), `:509` (quote text comes from `excerpt::of_record`). Measured
  2026-09-04 over the last user and assistant turn of 400 real sessions (776
  turns): `egress::scrub` over the full projection costs a worst case of
  0.563 ms on the largest projection (50,325 characters) and a mean of
  0.014 ms - roughly 1.1 ms worst case for the brief's two quotes against the
  10 ms release p99 `crates/verbatim/tests/hook.rs` asserts. If wrong:
  filtering after `clip` can cut a marker in half, and a secret truncated at
  the clip boundary is left as a nameless partial value the shape rules cannot
  catch, so the brief is the one of the four outputs where AC1 fails on a long
  turn.

- D-07 (Escape hatch): The raw escape hatch is a per-invocation CLI flag
  parsed in each terminal read command's own lexopt loop - never an environment
  variable, never a second config key. Evidence:
  `crates/verbatim/src/cmd/search.rs:186-230` and
  `crates/verbatim/src/cmd/show.rs:270-280` (each command matches its own
  `Long(...)` arms); `crates/verbatim/src/cmd/mod.rs:173-186` (`JSON_FLAG`,
  with the note that a shared-parser shortcut is "how `verify --json` came to
  look supported before it was"). If wrong: an environment variable is
  inherited by the hook and the MCP server, both of which load `Config`
  in-process (`crates/verbatim/src/cmd/hook.rs:255`,
  `crates/verbatim/src/cmd/mcp/tools.rs:606`), so the owner's own escape hatch
  silently unfilters the model-facing paths - the exact thing AC4 must not
  enable.

- D-08 (Shape): The filtered outputs announce themselves ONLY through the
  in-band markers; no `--json` shape gains a field in either setting. Evidence:
  `crates/verbatim/tests/mcp.rs:880-895` asserts the exact key set of a
  `recall_get` record; `docs/json-shapes.md:71-140`;
  `crates/verbatim-core/src/observe/egress.rs:61-63` ("Every one of them leaves
  a marker naming what went. A payload that came back silently shorter would
  leave a reader unable to tell filtering from a provider that returned
  less."); RCL-06 pins the shapes. If wrong: adding a `redacted` boolean breaks
  the pinned key-set assertion and changes a documented stable shape in BOTH
  settings, violating AC2 for the `--json` path.

## Decisions

- D-09 (Config): The knob is a new `verbatim.toml` TABLE with one optional
  boolean, resolved key-by-key in `Config::resolve` like every other table -
  not a key on `[provider]` and not a bare top-level key. Evidence:
  `crates/verbatim-core/src/config.rs:80-115` (`FileConfig` holds `injection`,
  `provider`, `retention`, `snapshot`, `capture`, each a table of `Option` keys
  with unknown keys ignored), `:637-690` (`resolve`), `:527-546` (`Config`
  fields); no `[privacy]` or `[recall]` table exists in the repo or in
  `DESIGN-BRIEF.md`. If wrong: placing it under `[provider]` reads as gated on
  a provider being configured - the same confusion `provider.local` already
  causes on this exact path - and a user with judgment disabled would
  reasonably conclude the knob does nothing.

- D-10 (Detector): Rule 1, the exact-credential rule, is fed
  `config.provider_api_key()` on the recall path, so on the common machine (no
  provider configured, OBS-02 default off) the knob rests on the eight shape
  rules alone. Evidence:
  `crates/verbatim-core/src/observe/egress.rs:215` (`redact(credential,
  text)`), `:232-243` (`replace_credential`, `MIN_CREDENTIAL_CHARS = 4`);
  `crates/verbatim-core/src/config.rs:730` (`provider_api_key`), `:686-694`
  (`provider.enabled` defaults false). If wrong: the only exact rule in the set
  never fires on the recall path for most users, and AC1 is satisfied entirely
  by shape guesses whose length floors are sized against this repo's sentinels
  (phase 2 D-12) rather than against real credentials.

- D-11 (Judgment): AC5's filter runs inside `judgment::store`, or on the
  `Judgment` immediately before it, using the `credential` and `&Config`
  already in scope at that call site. Evidence:
  `crates/verbatim-core/src/observe/judgment.rs:359-362` (`store(conn,
  session_key, config, &judgment, tokens, reservation).map_err(|e|
  note(credential, e))`), `:791-830` (`store` writes `topic`, `outcome` and the
  three claim lists), `:375-377` (the failed path already scrubs), `:838-841`.
  If wrong: n/a - the roadmap names both line numbers and the parameters are
  already in scope.

- D-12 (Out): `verbatim export` stays entirely unfiltered with the knob on,
  and its manifest notice keeps its current wording. Evidence:
  `crates/verbatim/src/cmd/export.rs:11-12`, `:52-56`;
  `docs/json-shapes.md:494-495`; `crates/verbatim/src/main.rs:131-132`. If
  wrong: either the export starts filtering while its own notice says it does
  not, or the notice needs a clause about a knob it ignores - and the notice
  text is asserted in tests.

- D-13 (Out): `verbatim observations` and `observations --json` are NOT among
  the filtered outputs and print whatever the column holds. Evidence:
  `crates/verbatim/src/cmd/observations.rs:278-301` (`topic`, `outcome`,
  `decisions`, `learned`, `unresolved` rendered straight from the row); the
  roadmap names four outputs and no fifth. If wrong: a user who set the knob
  sees filtered text in `recall_search --kind observation` and raw text in
  `verbatim observations` for the same claim, with nothing explaining the
  difference.

## Acceptance criteria

- [ ] AC1: A secret planted in a fixture transcript and ingested is returned
      with its characters intact by `recall_search`'s excerpt,
      `recall_context`'s window, `recall_get`'s body and the `SessionStart`
      brief's quote when the config table's boolean is absent; with that
      boolean set true and nothing else changed, none of the four contains the
      secret's characters and each contains that rule's named marker constant.
- [ ] AC2: With the boolean absent, a golden test over a fixture store finds
      all four outputs byte-identical to the same four captured from the
      pre-phase build, and every existing test in
      `crates/verbatim/tests/mcp.rs`, `crates/verbatim/tests/hook.rs` and the
      recall test files passes unmodified.
- [ ] AC3: Over one fixture store ingested once, the stored blob bytes and
      `verbatim verify --json`'s digests are equal with the boolean on and off,
      and `nothing_in_the_ingest_path_names_the_egress_filter` still passes.
- [ ] AC4: With the boolean on, `verbatim show <id>` prints the marker and
      `verbatim show <id>` with the raw flag prints the secret's bytes; a
      `verbatim hook SessionStart` run and an MCP `recall_get` call both emit
      filtered output no matter what environment variables are set.
- [ ] AC5: With the boolean on, a provider response whose `topic`, `outcome`
      and one claim `text` each carry a planted secret produces `observations`
      rows in which a direct SQL read of all three columns finds the marker and
      none of the secret's characters.
- [ ] AC6: An observation row stored while the boolean was absent, then
      queried through `recall_search` with `kind: "observation"` and the
      boolean on, returns the marker rather than the secret's characters.
- [ ] AC7: `crates/verbatim/tests/hook.rs` passes its p99 wall budget over 100
      runs of every event with the boolean on.

## Flagged assumptions

- The shape rules' length floors were sized in phase 2 against this repo's
  short sentinels (phase 2 D-12), and this phase points those same rules at
  free prose rather than at a request body - Likely; if wrong: a floor low
  enough to fire on a sentinel also fires on ordinary prose, and the
  over-matching direction phase 2 stated as tolerable now costs the owner
  detail in their own recall output rather than detail in one provider message.
  Measured baseline for the planner: 0.18% of real projections change under
  `egress::scrub` today (D-02).
- Rule 1 is inert for the common no-provider machine (D-10), so AC1 and AC6
  are carried by shape rules alone unless the fixture config sets a provider
  key - Likely; if wrong: a test that configures a provider key proves a rule
  most users never run. The planner should make each round-trip fixture plant
  its shape with NO surrounding name-keyed context, carrying forward phase 2's
  own flagged note, so only the rule under test can catch it.
- AC2's "pre-phase build" goldens have no in-repo source of truth until the
  phase generates them - Likely; if wrong: goldens captured after the first
  code change bake the change in and AC2 asserts nothing. The planner should
  capture them from the phase's base commit before any edit lands.
