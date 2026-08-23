---
phase: 5
plan: 3
requirements:
  - INJ-03
files:
  - crates/verbatim-core/src/recall/search.rs
  - crates/verbatim-core/src/recall/mod.rs
  - crates/verbatim-core/src/inject/mod.rs
  - crates/verbatim-core/src/inject/prompt.rs
  - crates/verbatim-core/src/testkit.rs
  - crates/verbatim-core/tests/recall.rs
  - crates/verbatim-core/tests/fixtures.rs
  - crates/verbatim-core/tests/inject_prompt.rs
  - crates/verbatim/tests/prompt.rs
  - tests/fixtures/session-edits.jsonl
  - tests/fixtures/README.md
---

# Phase 5: Context Injection - Plan 3 of 4 (the prompt path)

**ORDER: after PLAN-1. May run in parallel with PLAN-2** - that plan writes
`inject/brief.rs` and the brief tests and shares no file with this one. PLAN-4
comes after both and edits this plan's `inject/prompt.rs`, so it must not run
beside it.

## Goal

A prompt that names something the archive has seen gets back the one to three
past turns that actually match it, and a prompt that names nothing gets silence
- with the decision made on structure rather than on a score, and without
paying for a single decompressed session on the prompts that stay silent.

## Must be true when done

- A `Hit` says how its entities matched - whether the query asked for a whole
  stored value and nothing else, or asked for more besides - and how many
  distinct entities on that turn the query matched.
- A prompt naming a path relative to the payload's `cwd` matches the absolute
  path a past session stored, and the same prompt with the path removed matches
  nothing (AC3).
- A prompt whose only match is free text emits nothing, whatever its BM25
  relevance (AC7).
- At most three turns are ever injected, and the injected text stays inside the
  configured prompt budget.
- A prompt that injects nothing decompresses no session blob.

## Context

- D-04 binds the shape: the structural threshold cannot be read off a `Hit` as
  it stands, because `weight_by_entities` collapses the `EntityMatch` kind and
  the number of matched entities into the scalar `entity_score`. Approximating
  INJ-03's two conditions with a score cutoff is what `DESIGN-BRIEF.md:245`
  forbids outright - BM25 scores are not comparable across queries.
- D-05 binds the path case: stored path entities are overwhelmingly absolute -
  1,029 absolute against 2 relative over 120 sampled real transcripts - and
  `Query::matches_entity` requires EVERY token of the stored value to be present
  in the query, so `/root/project/crates/x.rs` cannot match a query of
  `crates/x.rs`. Without resolution AC3 is permanent silence that every test
  written against an absolute-path fixture still passes.
- D-12 binds the scope: `Scope::Directory` built from the payload's `cwd`.
- D-13 binds the cost: `search::run` calls `excerpt::attach` unconditionally,
  and one excerpt materializes a whole compressed session - p50 273 KB, p90
  1.03 MB, p99 2.6 MB, max 10.1 MB uncompressed over the real corpus.
- `recall::search`'s `TAIL` and `rank` are a total order on purpose, and its doc
  says why in this phase's own terms: "phase 5 reads rank 1..3 and a
  nondeterministic tie would make its threshold fire on a different turn each
  time". Nothing in this plan changes that order.
- Out of scope: the resume brief (PLAN-2); every suppression rule, the
  per-session state file and the compaction candidate pool (PLAN-4). This plan
  injects a turn it has already injected once this session, because nothing here
  remembers; PLAN-4 is what makes that stop.

## Tasks

### Task 1: A hit says how it matched, and how much

- **Files:** crates/verbatim-core/src/recall/search.rs (`Hit`,
  `weight_by_entities`), crates/verbatim-core/src/recall/mod.rs
- **Action:** D-04. `weight_by_entities` already computes, per candidate turn,
  which stored `(kind, value_norm)` pairs the query matched and whether each
  matched as `EntityMatch::Exact` or `EntityMatch::Covered`, and then throws
  both facts away into a summed `entity_score`. Carry them onto the `Hit`
  instead: the strongest kind matched on that turn, with `Exact` outranking
  `Covered` and neither present when nothing matched, and the count of DISTINCT
  matched `(kind, value_norm)` pairs on it. Distinct is load-bearing - one path
  named by three tool calls of one turn is one piece of evidence, and INJ-03's
  second condition is about independent entities co-occurring, so counting rows
  would let a single repeated path pass a threshold meant for two different
  facts. `entity_score` keeps its meaning and its name untouched: it is the
  ranking contribution, `docs/json-shapes.md` documents it and both `--json`
  front ends emit it. Nothing about the ordering changes - `rank`, `TAIL`,
  `CANDIDATE_POOL` and the `EXACT_WEIGHT`/`COVERED_WEIGHT` arithmetic stay
  exactly as they are - because a re-ranking hidden inside a reporting change is
  the kind of regression every existing test still passes. Re-export whatever a
  caller outside the crate needs to read the kind, beside the existing
  `pub use search::{Filters, Hit, ...}`.
- **Verify:** `cargo test -p verbatim-core --features testkit --test recall`
  passes, unchanged where it was already passing, with new cases over the
  existing `session-recall.jsonl` fixture showing: a query that is exactly the
  stored `src/worker/S.ts` value reports the exact kind on that hit; the same
  path inside a longer question reports the covered kind; a turn carrying two
  different matched entities reports 2; a turn naming one path three times
  reports 1; and a hit that matched only as free text reports no kind and 0.

### Task 2: A fixture that stores an absolute path the way real transcripts do

- **Files:** tests/fixtures/session-edits.jsonl,
  crates/verbatim-core/src/testkit.rs (`TRANSCRIPT_FIXTURES`,
  `ROOTED_FIXTURES`), crates/verbatim-core/tests/fixtures.rs,
  tests/fixtures/README.md
- **Action:** nothing in the fixture corpus stores an absolute path: the one
  `Read` in `session-recall.jsonl` names `docs/RETRY.md` relative, which is the
  spelling the real corpus almost never uses and the one AC3 is not about. Add a
  rooted transcript fixture under `project-alpha` whose every record carries
  `"cwd": "{{ROOT}}/project-alpha"` - the `FIXTURE_ROOT_TOKEN` rule, so a
  project key is whatever the test built and nothing else - holding a `tool_use`
  whose input names a file by absolute path beneath that same root, and enough
  content that `index::entity::extract` emits at least one more distinct entity
  for that same turn, so the co-occurrence half of INJ-03 has something to fire
  on. Give it a second turn that names the same file only in prose, so
  "structural match" and "mentions the words" stay distinguishable - the pair
  `session-recall.jsonl` already carries for the relative case. Register it in
  `TRANSCRIPT_FIXTURES` and in `ROOTED_FIXTURES`, and hold it to the properties
  `crates/verbatim-core/tests/fixtures.rs` already asserts over the whole set:
  UTF-8, one JSON object per line, a trailing newline, no `\r`, no occurrence of
  `testkit::UNIQUE_TOKEN`, and no literal `/data/code/verbatim`. Add its row to
  `tests/fixtures/README.md`'s table saying what it exists to exercise.
- **Verify:** `cargo test -p verbatim-core --features testkit --test fixtures`
  passes with the new fixture in the set, plus a new case there showing the
  fixture's tool record yielding a `path` entity whose value is absolute and at
  least two distinct entities in total for that turn when run through
  `index::entities`.

### Task 3: A relative path in a prompt finds the absolute one in the archive

- **Files:** crates/verbatim-core/src/inject/prompt.rs
- **Action:** D-05. Before the prompt becomes a `Query`, find its path-shaped
  tokens and resolve each one against the payload's `cwd`, carrying the resolved
  spelling into the query text so that every token of the stored absolute value
  is present and `Query::matches_entity` can return a match at all. A
  path-shaped token is one that survives `index::entity::normalize_path` - which
  already trims quotes and a trailing `:line:col`, the two decorations a user
  pastes - and that carries a path separator; an already-absolute one is left
  alone. Put the resolved spellings ahead of the prose in the string handed to
  `Query::parse`, because `MAX_QUERY_TOKENS` drops the thirty-third distinct
  token onward and a pasted stack trace ahead of the path would otherwise drop
  exactly the tokens this task exists to add. Touch no filesystem: 52% of the
  real corpus's `cwd` directories no longer exist (phase 2 D-05), so a path
  canonicalized today would key differently tomorrow and the same file would key
  two ways across one archive - this is string joining, not resolution. A
  payload with no `cwd` skips the step and asks what the user typed.
- **Verify:** `cargo test -p verbatim-core --features testkit --test inject_prompt`
  passes with cases showing: over a store holding task 2's fixture, a prompt of
  "what changed in <the relative path>" with the fixture's project as `cwd`
  produces a query whose `matches_entity` on the stored absolute value returns a
  match, and the same prompt built without the resolution step returns none; an
  already-absolute path in a prompt still matches; and a prompt of thirty words
  of prose followed by the path still matches, proving the ordering against
  `MAX_QUERY_TOKENS`.

### Task 4: The structural threshold, and the excerpt nobody pays for

- **Files:** crates/verbatim-core/src/inject/prompt.rs,
  crates/verbatim-core/src/recall/search.rs
- **Action:** INJ-03's decision, made on structure. Run one project-scoped
  search from the payload's `cwd` (D-12) with a small bounded limit - use 10, so
  the ranked window is wider than the cap of three without widening the pool
  `CANDIDATE_POOL` already fixes - and then apply the threshold over the ranked
  hits using the two facts task 1 put on `Hit`. A hit is eligible when it sits
  within the top three of that order AND matched at least one entity, or when it
  matched two or more distinct entities wherever it ranks: the first is INJ-03's
  rank-1-to-3 exact-entity condition, the second is its co-occurrence condition,
  which is not a subset of the first because corroboration by two independent
  entities is evidence that BM25's ordering may have got wrong. A hit that
  matched only as free text - no matched entity at all - is never eligible,
  whatever its relevance: a score cutoff is exactly the approximation
  `DESIGN-BRIEF.md:245` forbids, and free-text-only matches are the weakest
  signal in the hierarchy and the one that fires the most false positives. At
  most three turns survive, in the ranked order, and zero is the ordinary
  answer. Then the cost half, D-13: `search::run` attaches an excerpt to every
  hit it returns, and an excerpt materializes a whole compressed session, so
  give the search a way for this caller to skip that step, run the threshold
  over the hits without excerpts, and attach them afterwards through the
  already-public `excerpt::attach` for the at-most-three turns that fired.
  Nothing about the excerpt path itself changes for `verbatim search` or
  `recall_search`, which must keep attaching excerpts exactly as they do now.
- **Verify:** `cargo test -p verbatim-core --features testkit --test inject_prompt`
  passes with cases showing: a prompt naming the fixture's path fires on the
  turn that stored it; a prompt of prose that matches only free text fires on
  nothing (AC7); a store where four turns qualify yields three; a hit outside
  the top three carrying two distinct matched entities fires while a hit outside
  the top three carrying one does not; and an instrumented run over a
  non-firing prompt reports `excerpt::Reads` of zero blobs, while a firing one
  reports no more than the number of distinct sessions it injected from.

### Task 5: The injected turns, and the prompt that gets nothing

- **Files:** crates/verbatim-core/src/inject/prompt.rs,
  crates/verbatim-core/src/inject/mod.rs, crates/verbatim/tests/prompt.rs
- **Action:** render the turns that fired into the text PLAN-1's seam emits as
  `hookSpecificOutput.additionalContext`: each one identified by the turn id
  `verbatim show` and `recall_get` take, its session's date at day resolution -
  the first ten characters of the stored timestamp, guarded on the character
  boundary the way `cmd::search::day` is - and its text, inside the
  `prompt_chars` budget PLAN-1 put in `Config`, counted in characters (D-16) and
  cut with `recall::excerpt::ELISION`. The order is the ranked one, so two runs
  of one prompt against an unchanged store render the same bytes. When nothing
  fired the arm returns nothing at all and the hook writes nothing, which is the
  common case and the one the phase exists to protect: three near-misses are
  worse than silence.
- **Verify:** AC3 and AC7, end to end. `cargo test -p verbatim --test prompt`
  passes with a temp data directory seeded from task 2's rooted fixture and
  ingested to completion, showing: `verbatim hook UserPromptSubmit` with a
  prompt naming the fixture's file relative to the payload's `cwd` writes
  exactly one JSON object whose `hookSpecificOutput.hookEventName` is
  `UserPromptSubmit` and whose `additionalContext` names the turn that stored
  that path; the same prompt with the path removed writes nothing and exits 0;
  a prompt of generic prose that only matches free text writes nothing and exits
  0; and no emitted `additionalContext` exceeds the configured budget.
