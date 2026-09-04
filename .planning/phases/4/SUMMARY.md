---
phase: 4
status: complete
completed: 2026-09-04
---

# Phase 4: Redacted Recall - Summary

An opt-in `[privacy] redact_recall` boolean in `verbatim.toml` routes all five
derived recall projections plus the judgment write through
`egress::for_model_context`, with a per-invocation `--raw` escape hatch on the
two terminal read commands and no environment variable anywhere that can reach
past it.

## What shipped

- The knob - `[privacy] redact_recall`, resolved once in `Config::resolve` and
  read back through `Config::redact_recall()` - `crates/verbatim-core/src/config.rs`
- An unconditional egress entry point, `egress::for_model_context`, and the
  `Redaction<'a>` value that carries the decision to a projection without
  handing it a `Config` - `crates/verbatim-core/src/observe/egress.rs`
- Five filtered projections: `recall_search`'s excerpt and observation claims,
  `recall_context`'s window, `recall_get`'s body, the `SessionStart` brief's
  quotes, and the per-prompt injection -
  `recall/{search,excerpt,context,get}.rs`, `inject/{brief,prompt}.rs`
- The judgment write filter - a successful judgment's `topic`, `outcome` and
  every claim `text` filtered before they reach SQLite -
  `crates/verbatim-core/src/observe/judgment.rs`
- The owner's escape hatch - `--raw` on `verbatim search` and `verbatim show`
  only, rejected as misuse everywhere else -
  `crates/verbatim/src/cmd/{search,show}.rs`
- Pre-phase goldens pinning the four unfiltered projections byte-for-byte -
  `crates/verbatim-core/tests/goldens/recall-projections.txt`
- Process-boundary proofs for the MCP server's three tools and the
  `SessionStart` hook, including environment immunity and the p99 wall budget -
  `crates/verbatim/tests/{mcp,brief,hook}.rs`

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 6499e53 | Pin the four recall projections to a pre-phase golden |
| 1 | 2 | 3c8977e | Add the opt-in `[privacy] redact_recall` knob to `verbatim.toml` |
| 1 | 3 | 4261cb2 | Filter the search excerpt through an unconditional egress boundary |
| 1 | 4 | c5376ef | Filter `recall_search`'s observation claims on read |
| 1 | fix | f73b0af | Let the knob govern the per-prompt injection too |
| 2 | 1 | 40e413b | Filter the context window's turn text |
| 2 | 2 | c9491bb | Filter the archived body `recall_get` hands back |
| 2 | 3 | 40ba8e5 | Filter the brief's quotes before the budget is spent |
| 2 | 4 | 57fd11d | Filter a successful judgment before it reaches SQLite |
| 3 | 1 | 567846d | Give `search` and `show` a per-invocation `--raw` escape hatch |
| 3 | 2 | 34dd23f | Prove the terminal round trip at the process boundary |
| 3 | 3 | 88d2e5e | Prove the archive is untouched in both settings |
| 4 | 1 | d69ec41 | Prove the three MCP tools filter at the process boundary |
| 4 | 2 | d40ef74 | Prove the `SessionStart` brief filters at the hook boundary |
| 4 | 3 | 9412491 | Hold the hook wall budget with the redaction knob on |

## Deviations

- [deviation] PLAN-1 asserted the per-prompt injection stays byte-identical in
  both settings, and Task 3 instructed that call site to pass the no-filter
  value - both resting on the SCOPE half of CONTEXT D-01, which named
  `inject/prompt.rs` as a fifth surface deliberately left out. The blocking
  `risk_surface` review raised it as a blocker: that surface writes to hook
  stdout, the same model-context boundary `egress::for_model_context` exists
  for, so leaving it raw was a live leak under the very knob this phase adds.
  The user reversed the scope decision. `inject/prompt.rs` now resolves
  `Redaction::of(config)` (f73b0af), D-01's PLACEMENT half kept intact. The
  criterion holds in its knob-off direction only, asserted by
  `the_default_leaves_the_per_prompt_injection_exactly_as_it_was`; the on
  direction is a fifth filtered surface with its own test. AC2's four goldens
  are unaffected.
- [deviation] PLAN-2 Task 4 called for a planted sentinel in a judgment's
  `outcome` column. Unachievable: `judgment::read` validates `outcome` against
  the closed `OUTCOMES` enum before `store` is reached, so such an answer never
  becomes a `Judgment` at all. The filter still runs over `outcome` as D-11
  requires; the sentinels moved to `topic` and one claim `text` per list, and
  the `outcome` half is proved in the stronger direction by
  `an_outcome_carrying_a_credential_never_becomes_a_stored_judgment` (57fd11d).
- [deviation] PLAN-3 Task 1's `Verify` predicted `grep -rn '"raw"'` would name
  only the two parse loops; it also names a pre-existing hit at
  `crates/verbatim/src/cmd/observations.rs:302`, the unrelated `"raw"` JSON key
  of `observations --json`. The substance held - the flag string appears in
  exactly the two parse loops (567846d).

## Open items

- `recall_get`'s filtered arm does a lossy UTF-8 round-trip
  (`crates/verbatim-core/src/recall/get.rs:236`), so an archived line that is
  not valid UTF-8 comes back with U+FFFD replacement bytes once the knob is on,
  even carrying no secret. Raised `medium` by the plan-2 `risk_surface` review
  and recorded as confirmed-and-not-fixed in
  `ADJUDICATION-risk_surface-plan-2.json`. It could not be offered as a
  file-or-decline choice because `issue-filing.mjs` refused `no-forge`:
  `git.forge_provider` and `git.forge_repo` are unset on this repository.
- `cargo fmt` reformats eleven files outside phase 4's leases - pre-existing
  drift committed by earlier work, including an out-of-order `use` in
  `crates/verbatim-core/tests/retention.rs` and three diffs in
  `crates/verbatim/tests/hook.rs`. Left as found so none of it rode these
  commits. `cargo fmt` is not among the project's detected static-analysis
  commands.
- `crates/verbatim/tests/backfill.rs:493` is load-sensitive: its
  `interrupted >= 2` harness self-check can come up short under a fully
  parallel workspace suite. Passes run alone. Outside phase 4's leases and
  untouched by its commits; worth a look as a suite-stability item.

## Goal check

The phase goal is that an opt-in knob filters the derived recall projections,
because hook and MCP output is model context and therefore an egress boundary.
The commits deliver it. Every surface that renders archived text to a model now
resolves the decision at an entry point holding a `&Config` and cuts through
`egress::for_model_context`: the search excerpt (4261cb2), observation claims
(c5376ef), the context window (40e413b), the `recall_get` body (c9491bb), the
brief's quotes (40ba8e5) and the per-prompt injection (f73b0af). The two
surfaces the goal names by name are proved at the process boundary rather than
by library call - `crates/verbatim/tests/mcp.rs` spawns the server and asserts
all three tools filtered, `tests/brief.rs` spawns the hook and asserts the same
of `additionalContext` (d69ec41, d40ef74) - and both prove the filter survives
`RAW`, `VERBATIM_RAW` and `VERBATIM_REDACT_RECALL=false` on the spawn. That
environment immunity is structural, not enumerated:
`crates/verbatim-core/src/config.rs:724` is the only assignment to the resolved
field and reads the parsed TOML alone. The default is unchanged, held by
goldens captured before any product byte moved (6499e53) and still passing. The
archive itself is untouched in both settings, asserted over blob bytes and
`verbatim verify --json` (88d2e5e), and the hook stays inside its 10 ms p99
budget with the knob on - SessionStart 4.40 ms, the other three events under
1.2 ms (9412491). What is missing is narrow and named above: the lossy UTF-8
round-trip on `recall_get`'s filtered arm, which changes bytes for a neutral
non-UTF-8 input.
