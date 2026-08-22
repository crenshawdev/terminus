---
phase: 7
status: complete
completed: 2026-08-22
---

# Phase 7: Observations - Summary

Per-session observations in two halves: mechanical facts computed with no model call, and an opt-in LLM judgment bought through one OpenAI-compatible provider path, whose every claim is validated against a real `turns` row before it is stored.

## What shipped

- `observations` table - archival, never derived, absent from `DERIVED_TABLES`, no reference to `turns` (`crates/verbatim-core/src/store/schema.rs`)
- Mechanical facts per finalized session - files, tools, commands, errors, branch, commits, turn count, duration; lists capped at 64, values at 512 bytes, cuts named in the document (`crates/verbatim-core/src/observe/mechanical.rs`)
- `verbatim observations` and `verbatim observations regenerate` - listing with `--project`/`--json`, and a targeted rebuild narrowed by `--since` and `--prompt-version` (`crates/verbatim/src/cmd/observations.rs`)
- One HTTP client, behind one instrumented constructor - ureq 3.4 + rustls, redirects off, a testkit-gated attempt log recorded before the socket opens; `observe::net` is the sole namer and a source-level test asserts it (`crates/verbatim-core/src/observe/net.rs`)
- `[provider]` config plus a shared credentials loader with PRIV-02's refusal - any group or world bit refuses, and `config::Secret` renders redacted so `Config`'s derived `Debug` stays safe (`crates/verbatim-core/src/config.rs`, `credentials.rs`)
- A destination-keyed egress filter and an unconditional error scrubber over one rule set - local-only is a `Cow::Borrowed` no-op, so byte-identity is provable (`crates/verbatim-core/src/observe/egress.rs`)
- The judgment call and its anchoring contract - fixed versioned schema, one turn per line beside its real `turns.id`, anchors validated before any row is written (`crates/verbatim-core/src/observe/judgment.rs`)
- Four cost gates plus a fifth bound - `MIN_TURNS`, `TRUNCATION_BUDGET`, a daily token budget in one `meta` row, the one-call-per-session gate, and `JUDGED_PER_PASS = 1` (`crates/verbatim-core/src/observe/cost.rs`)
- The parse-failure arm - one retry, both requests charged, the second unusable answer stored under `status = parse_failed` with its raw text scrubbed and the claim columns cleared
- Judgment runs after the ingest pass and outside its lock (D-07), so a request in flight cannot make a second `verbatim ingest` exit `LockHeld`
- A reservation before the request - `observations.status` compare-and-swapped to `judging <instant>`, so two overlapping passes buy exactly one answer
- Observations reachable through the recall tools the model already has - a claims branch over `observations`, one hit per claim carrying its own `turn_id`, scoped and excluded like every other hit, still exactly three MCP tools

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 571393b | Declare the `observations` table, archival and never derived |
| 1 | 2 | 837c0ab | Compute the mechanical facts for one session |
| 1 | 3 | d936601 | Ingest observes every session it finalizes, once |
| 1 | - | e562684 | Checkpoint resolution: compare the walk, not the post-walk half |
| 1 | 4 | 8dffc43 | `verbatim observations` lists what every closed session did |
| 1 | 5 | a9f0fc6 | `observations regenerate` rebuilds exactly the set it was handed |
| 2 | 1 | 76fca89 | One HTTP client, behind one instrumented constructor |
| 2 | 2 | 949d041 | The provider config block |
| 2 | 3 | c03b720 | The shared credentials loader and PRIV-02's refusal |
| 2 | 4 | dcfe999 | The destination-keyed egress filter and the error scrubber |
| 2 | 5 | 6621162 | The `chat/completions` request and response |
| 2 | 6 | b9d3f0d | `doctor` reports the credentials state |
| 2 | - | 37b5834 | Correction: with judgment off, nothing resolves a credential or builds a request |
| 3 | 1 | 1ce8223 | One judgment call, every claim anchored to a real turn |
| 3 | 2 | b2c6d1a | Four gates on what a judgment run may cost |
| 3 | 3 | 0c826c0 | An answer that will not do is asked once more, then kept |
| 3 | 4 | db97524 | Judgment runs after the pass, outside the lock |
| 3 | 5 | 938c581 | `regenerate` re-asks the model for exactly the rows it chose |
| 3 | 6 | 5ae63b4 | Observations reachable through the tools the model already has |
| 3 | fix | 89098d5 | The row is claimed before the request, not after it |

`f61eca3` is a WIP pause commit carrying planning documents only.

## Deviations

None - all three plans executed as written. Two things worth reading as corrections rather than deviations: plan 1's checkpoint was a lease question the user resolved with an approved one-file extension (`crates/verbatim-core/tests/backfill.rs`, added to PLAN-1's `files:` so `lease-check` records the grant), and plan 2's `37b5834` corrected a plan claim that held only because no caller existed yet - `credentials::resolve` and `provider::complete` now both return before doing anything while judgment is off, so the attempt log stays empty and a test reads that as a number.

## Review

The blocking `risk_surface` trigger fired once on plan 3's committed range (detector matched `concurrency` and `destructive`) and was adjudicated over two rounds.

Round 1 raised two high findings. One survived: nothing reserved the `observations` row between `unjudged()` selecting a session and `store()` writing it, and the UPDATE carried no status guard, so two concurrent passes both paid for the same session and the later write won - reachable by design, since the plan requires a second `verbatim ingest` to take the lock while a request is in flight. Fixed at `89098d5`. The second was downgraded: the daily-budget overshoot half is documented at `cost.rs:145-150` as an accepted D-07 trade-off, and only the narrower accumulator-loss half survives, as an open item below.

Round 2 (the one narrowed re-arm, on the fix's own diff) raised two, both downgraded, nothing surviving at blocker/high - so the gate passed. Records at `ADJUDICATION-risk_surface-plan-3.json` and `-r2.json`.

## Open items

- `cost::spend` is a read-then-write against the `meta` row, so two concurrent increments store one and the day's accumulator permanently under-reports. Distinct from the bounded one-call overshoot `cost::admits` documents and accepts; an atomic add would close it.
- `cost::admits` runs the minimum-turn and budget gates before the reservation check, so a concurrent caller on an in-flight session can report `BudgetSpent` rather than the new `InFlight`. Affects the skip string only - not whether a request is made, what is stored, or what is charged.
- `crates/verbatim/tests/hook.rs` fails nondeterministically under full-workspace parallel load - a different subset each run (`the_ingest_survives_a_group_kill_and_a_descendant_sweep`, `an_unterminated_line_stops_being_read_at_the_cap`, and others), all passing in isolation and on repeated single-test runs. `hook.rs` is untouched by this phase (`git log 45c8f86..HEAD -- crates/verbatim/tests/hook.rs` is empty), so this is pre-existing contention in process-spawning tests, not a regression.
- AC4's remote half is unverified: it is human-verify and needs a remote OpenAI-compatible key, which this machine does not have. Put a key in the shared credentials file, change only `base_url`, `model` and the key, run the same call, confirm a parsed response.
- The response schema travels in the system message, not in `response_format`. `provider::complete` sends D-09's `{"type":"json_schema","strict":true}` from a private constant with no `json_schema` object beside it; a strict remote endpoint may reject that, which is a one-line change in `provider.rs`.
- OBS-05's Anthropic subscription-auth half is deferred out of this phase (CONTEXT's deferral list). The requirement row needs the note saying phase 7 delivered the OpenAI-compatible half only.
- A reservation abandoned by a killed process is retaken once its 900s lease lapses; where the dead run was a `regenerate` over an already-judged row, retaking it buys a second answer for a session that had one. Bounded by `JUDGED_PER_PASS`; carrying the replaced status inside the token would let the stealer restore it instead.
- `observations regenerate --json` gained no new `data` fields - `judged` and `unasked` are typed on `Regenerated` but reported through `notes`, because promoting them would break `crates/verbatim/tests/cli.rs`'s exact key-set assertion and that file was outside plan 3's lease.
- `observations regenerate` runs each row's UPDATE on its own rather than one transaction over the set, so a row whose blob will not decompress is a note and the rest still rebuild. A caller wanting all-or-nothing does not have it.
- `observations regenerate --json` emits `command: "observations regenerate"`, the only two-word value in that envelope field.
- `cost::JUDGED_PER_PASS = 1` is a fifth bound no task named. It exists because 100 sequential calls at a 120s timeout is the long-lived detached process `PROJECT.md` rules out. Raise it when a task states a second value.
- Declined: a configurable request timeout on `observe::net` (the 120s ceiling stays a compile-time constant beside D-11's rule that only the money-facing knob is a config key), and a `Store`-level accessor pair for the daily-spend row (every `observe` entry point holds a bare `&Connection`; D-12 admits an equivalent pair over the same table).
- Pre-existing `cargo fmt --check` drift in `crates/verbatim-core/src/ingest/backfill.rs:250` and `crates/verbatim/tests/hook.rs:252,681`, untouched by this phase.

## Goal check

The phase goal asks for optional, auditable session summaries whose every claim points back at a real turn, through one provider path, with a redaction boundary that exists only when data leaves the machine. The commits plausibly deliver it, and the four load-bearing claims each have evidence. **Optional**: `37b5834` makes `credentials::resolve` and `provider::complete` both return before doing anything with judgment off, and the testkit attempt log reads zero as a number rather than as an absence. **Auditable and anchored**: `1ce8223` validates every `turn_id` against `turns` of that session before a row is written, and `5ae63b4` carries that same `turn_id` out through the recall claims branch, so a claim is traceable end to end without a fourth MCP tool (`crates/verbatim/tests/mcp.rs` asserts three). **One provider path**: `76fca89`'s source-level test in `tests/provider.rs` walks both crates and asserts `observe::net` is the only namer of the HTTP client. **A boundary only where data leaves**: `dcfe999`'s `egress::for_destination` returns `Cow::Borrowed` on the local path, which is how byte-identity is proven rather than asserted.

Two gaps, both named rather than papered over. Success criterion 4 - the same code path against a local endpoint and against OpenRouter - is verified on its local half only; the remote half needs a key this machine does not have, and it is the one place the `response_format` shape flagged above could fail. Success criterion 6's second half is in the same position: the local-provider path is shown to filter nothing, but "filtered at egress against a remote provider" has been exercised against a stub rather than a real remote. Criterion 6's first half - no connection opened with judgment disabled - is verified at the attempt log rather than at a socket-level trace, which is the same claim one layer up. Everything in criteria 1, 2, 3 and 5 is verified locally and green.
