---
phase: 4
plan: 1
requirements:
  - PRIV-03
  - OBS-03
files:
  - crates/verbatim-core/tests/recall_golden.rs
  - crates/verbatim-core/tests/goldens/recall-projections.txt
  - crates/verbatim-core/src/config.rs
  - crates/verbatim-core/tests/config.rs
  - crates/verbatim-core/src/observe/egress.rs
  - crates/verbatim-core/src/recall/excerpt.rs
  - crates/verbatim-core/src/recall/search.rs
  - crates/verbatim-core/src/inject/prompt.rs
  - crates/verbatim-core/tests/redacted_recall.rs
---

# Phase 4: Redacted Recall - Plan 1

## Goal

The opt-in knob exists in `verbatim.toml`, the filter it turns on has an
unconditional entry point of its own, and the first derived projection -
`recall_search`'s excerpt, turn hits and observation claims alike - runs through
it. Default stays today's raw behaviour, and the goldens that prove it are
captured before any product byte moves.

## Must be true when done

- A `verbatim.toml` whose new table sets the boolean true resolves to a config
  that says so; the table absent, the key absent, and the key `false` all
  resolve to today's behaviour, and an unknown key inside the table is ignored
  rather than refused.
- With the boolean absent, the four projections over a fixture store are
  byte-identical to goldens captured from this phase's base commit, and the
  golden file itself visibly carries an unfiltered sentinel.
- With the boolean true, a search over the `session-secrets.jsonl` project
  returns excerpts carrying the named marker constant and none of the planted
  sentinel's characters; with it absent the same search returns the sentinel
  whole.
- A turn whose projection is longer than 240 characters, with the secret's NAME
  sitting outside the window the excerpt would cut, still comes back with no
  sentinel characters in it.
- An observation claim stored while the boolean was absent comes back through
  `recall_search`'s `kind: "observation"` branch filtered once the boolean is
  on, with its `turn_id` unchanged.
- The per-prompt injection path (`inject::prompt`'s own `excerpt::attach` call)
  is byte-identical in both settings.

## Context

Locked: D-01 (the knob is consulted at the four entry points that already carry
`&Config`, never inside `excerpt::of_record`/`attach`), D-02 (filter the FULL
projection before the 240-character window is cut), D-03 (an unconditional
entry point, never `egress::for_destination`, whose `local = true` arm returns
`Cow::Borrowed`), D-05 (the observation branch filters on READ), D-09 (a new
`verbatim.toml` table with one optional boolean, resolved key by key), D-10
(rule 1 is fed `config.provider_api_key()`). Out and untouched here: `verbatim
export` (D-12), `verbatim observations` and `observations --json` (D-13), and
every `--json` key set (D-08). Ingest-time redaction stays barred:
`tests/egress.rs::nothing_in_the_ingest_path_names_the_egress_filter` scans
`src/ingest/*.rs` for the words `egress`, `scrub` and `redact`, and nothing this
plan writes may put one there. Plans 2, 3 and 4 consume the config accessor and
the egress entry point this plan creates, so the phase runs in numeric order.

## Tasks

### Task 1: Capture the pre-phase goldens for the four projections

- **Files:** crates/verbatim-core/tests/recall_golden.rs,
  crates/verbatim-core/tests/goldens/recall-projections.txt
- **Action:** THIS TASK LANDS BEFORE ANY PRODUCT EDIT OF THIS PHASE. AC2's
  "pre-phase build" has no in-repo source of truth until it is captured, and a
  golden generated after the first code change bakes that change in and asserts
  nothing (CONTEXT flagged assumption 3). Add a new `#![cfg(feature =
  "testkit")]` test file that builds a store the way
  `crates/verbatim-core/tests/recall.rs`'s `bench()` does - every
  `testkit::TRANSCRIPT_FIXTURES` entry through `ingest::run`, the
  `testkit::ROOTED_FIXTURES` members through `testkit::copy_rooted_fixture_into`
  under a root the test owns - and captures one deterministic document holding
  all four projections: `search::run` excerpts for a fixed query scoped to the
  `session-secrets.jsonl` project and a second query scoped to `project-alpha`;
  `context::window` turn texts around one anchor from each; `get::records`
  bodies for those same anchors; and `brief::session_start` for both projects.
  Every captured string has the tempdir root substituted back to
  `testkit::FIXTURE_ROOT_TOKEN` before it is written or compared - session keys
  and the brief's project label carry the temporary path, and a golden holding
  it would differ on every run for a reason that is not the product. Compare
  byte for byte against the checked-in file and fail with a readable diff;
  regenerate the file only when an environment variable the test names is set,
  so a regeneration is always a deliberate act visible in the diff. Call only
  what exists today: `recall::search::run`, `recall::context::window`,
  `recall::get::records` and `inject::brief::session_start` all take `&Config`
  already, and nothing in this task may reference the config key or the filter
  the later tasks add.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --features testkit --test recall_golden` passes, and passes a
  second time with no regeneration variable set. `grep -c VBEGRESS
  /code/verbatim/crates/verbatim-core/tests/goldens/recall-projections.txt`
  returns a non-zero count, so the captured document provably carries an
  unfiltered planted secret. `git -C /code/verbatim show --stat HEAD` for this
  task's commit lists no path under `crates/verbatim-core/src` or
  `crates/verbatim/src`.

### Task 2: The opt-in table in verbatim.toml

- **Files:** crates/verbatim-core/src/config.rs (`FileConfig`,
  `Config::resolve`, `Config`), crates/verbatim-core/tests/config.rs
- **Action:** Add one new table to `FileConfig` carrying a single optional
  boolean, built exactly the way `FileCapture` and `FileSnapshot` are: its own
  `#[derive(Debug, Clone, Default, Deserialize)]` struct of `Option` keys, a
  `#[serde(default)]` field on `FileConfig`, unknown keys ignored under the rule
  the file already states. Resolve it in `Config::resolve` key by key against
  the default the way `config.snapshot` and `config.capture` are resolved -
  absent means false - store it on `Config`, and read it back through an
  accessor beside `provider_local` and `capture_mode`. Add one further
  affordance whose only consumer is PLAN-3's terminal raw flag: a way for a
  caller holding a `Config` to obtain the equivalent value with this knob forced
  off, so a command can pass a config of its own to the query layer without
  re-reading the file. The spelling chosen is `[privacy]` with the key
  `redact_recall` - the planner's choice under D-09, taken over `[recall]`
  because the same boolean also gates the judgment WRITE filter (AC5), which is
  not a recall path. It is not a key on `[provider]` and not a bare top-level
  key (D-09): under `[provider]` it would read as gated on a provider being
  configured, which is the confusion `provider.local` already causes on this
  exact path, and a user with judgment disabled would reasonably conclude the
  knob does nothing. The doc comments carry that reasoning and state why the
  default is off: every byte in the archive already reached the provider once
  when it was typed, so redacting by default would delete the product. No new
  dependency and no lockfile change.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --test config` passes new tests showing that a `verbatim.toml`
  carrying the table with the boolean true resolves true; that the table absent,
  the key absent and the key written `false` all resolve false; that an
  unrecognized key inside the table is ignored rather than failing the load; and
  that the forced-off copy of a config which resolved true reports false while
  the original still reports true. `cargo build --manifest-path
  /code/verbatim/Cargo.toml --workspace` succeeds and `git -C /code/verbatim
  diff --quiet Cargo.lock` exits 0.

### Task 3: Route the search excerpt through an unconditional egress entry point

- **Files:** crates/verbatim-core/src/observe/egress.rs,
  crates/verbatim-core/src/recall/excerpt.rs (`of_record`, `attach`),
  crates/verbatim-core/src/recall/search.rs (`run`),
  crates/verbatim-core/src/inject/prompt.rs,
  crates/verbatim-core/tests/redacted_recall.rs
- **Action:** Four connected changes, and the end-to-end path they make is the
  phase's skeleton. First, `egress.rs` gains a third public entry point beside
  `for_destination` and `scrub`, taking the same credential-and-text pair and
  delegating to the same private `redact`, with NO `local` parameter (D-03): the
  destination here is the model's context and not the configured provider, and
  routing through `for_destination` would return `Cow::Borrowed` unfiltered for
  every user who declared `provider.local = true` - the knob silently inert for
  exactly the user running a local model for privacy reasons. Update the module
  doc's "Two entry points, one rule set" section to three and say what this one
  is for, keeping the existing list of `for_destination`'s callers honest.
  Second, `excerpt::of_record` and `excerpt::attach` apply a redaction the
  CALLER supplies and read no `Config` themselves (D-01), and apply it to the
  projected text - `text::project`'s output, which is what D-02 measured at
  0.18% of real projections changed - before `flatten` and before `window` cuts
  `EXCERPT_CHARS`. Never after the window: the header rule needs the secret's
  NAME and its VALUE in the same string, so a name that falls outside the cut is
  a value the rule never sees, and AC1 could still pass on a fixture whose
  secret happens to land whole inside the window. Third, `search::run` derives
  that value from the `&Config` it already takes, feeding rule 1 with
  `config.provider_api_key()` (D-10), and hands it to `attach`. Fourth,
  `inject::prompt`'s own `excerpt::attach` call passes the no-filter value: the
  per-prompt injection is the fifth surface D-01 names, this phase's criteria do
  not cover it, and it must stay byte-identical in both settings. That call is
  the ONLY place needing the no-filter value - `inject::prompt` and
  `feedback::replay` both build their `Request` with `.excerpts(false)`, so
  `search::run`'s `attach` never runs for either of them. Nothing in
  `src/ingest` is touched or reached.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --features testkit --test redacted_recall` passes new tests over
  a fixture store built the way `tests/recall.rs` builds one, with the knob
  turned on by writing a `verbatim.toml` and loading it through
  `Config::load_from` (the reason `tests/inject_brief.rs` gives for its own
  config helper: the key has to reach the query layer through the file the
  binary reads): with the knob absent, a search over the `session-secrets.jsonl`
  project returns an excerpt containing `sk-VBEGRESS-authz-9f2`; with the knob
  on and nothing else changed, the same search returns an excerpt containing
  `verbatim_core::config::REDACTED` and none of `sk-VBEGRESS-authz-9f2`,
  `VBEGRESS` or `authz`; and a turn the test archives itself, whose projection
  exceeds `recall::EXCERPT_CHARS` with the `Authorization:` name more than that
  many characters away from the query token, comes back with no sentinel
  characters in the excerpt (absence only - the marker itself may sit outside
  the window, which is the point of filtering first). `cargo test
  --manifest-path /code/verbatim/Cargo.toml -p verbatim-core --features testkit
  --test recall_golden`, `--test recall`, `--test inject_prompt` and `--test
  egress` all pass with no edit to any of those files.

### Task 4: Filter the observation branch on read

- **Files:** crates/verbatim-core/src/recall/search.rs (`observations`),
  crates/verbatim-core/tests/redacted_recall.rs
- **Action:** `search::run`'s observation branch reads `excerpt` straight off
  the claim column through `json_each` and deliberately calls no
  `excerpt::attach` - the text IS the claim. With the knob on, put that text
  through Task 3's entry point before it becomes a `Hit`, threading the decision
  down from `run`, which is where the `&Config` is (the branch takes none
  today). On READ and not write-only (D-05): a claim row written before the knob
  was flipped is then covered with no regeneration and no user action, and the
  markers are whitespace-free and idempotent by egress.rs's own
  `REDACTED_JWT` doc, so a second scrub over a row already filtered at write
  time (PLAN-2's half) is a no-op. `turn_id`, `session_key`, `ts`, `project` and
  the flat `relevance` are untouched - OBS-03's anchor is the only thing that
  makes a claim auditable and it has to survive the filter. `verbatim
  observations` and `observations --json` are NOT filtered (D-13) and no file
  behind them is touched.
- **Verify:** `cargo test --manifest-path /code/verbatim/Cargo.toml -p
  verbatim-core --features testkit --test redacted_recall` passes a new test
  that inserts one `observations` row directly the way `tests/recall.rs`'s
  `observe` helper does, with a claim whose text carries both a query token and
  `sk-VBEGRESS-authz-9f2`, anchored at a real turn of that session: queried with
  `kind: "observation"` and the knob absent the hit's excerpt holds the
  sentinel; with the knob on and the row unchanged on disk it holds
  `verbatim_core::config::REDACTED` and none of the sentinel's characters, and
  the hit's `turn_id` is the same id in both runs. `cargo test --manifest-path
  /code/verbatim/Cargo.toml -p verbatim --features testkit --test observations`
  and `-p verbatim-core --features testkit --test recall` pass with those files
  unmodified.

## Notes

- Ordering: this plan is first. Plans 2, 3 and 4 all consume the config
  accessor from Task 2 and the egress entry point from Task 3, so the phase runs
  in numeric order; plans 3 and 4 share no files with each other and may run in
  parallel once 1 and 2 have landed.
- Task 1 must be the phase's first commit. If any product edit of this phase has
  already landed when it runs, the goldens are worthless and the executor should
  say so rather than capture them anyway.
- The knob is turned on in tests by writing a `verbatim.toml` and calling
  `Config::load_from`. That is deliberate: PLAN-1 Task 2 adds a forced-OFF copy
  and no in-memory setter, so nothing can turn the filter on except the file the
  binary reads.
