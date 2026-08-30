---
phase: 2
status: complete
completed: 2026-08-30
---

# Phase 2: Egress Filter Sees What Is Sent - Summary

The egress filter runs over each message's content before `json!` builds the remote request body, and the rule set widened from 5 rules to 9 so the seven credential shapes the roadmap names are caught in transcript text rather than only in hand-written header samples.

## What shipped

- Nine redaction rules, each naming its own marker - `crates/verbatim-core/src/observe/egress.rs`. Rule 4 (header shape) now tests every colon on a line rather than only the first, which is what made it inert against `turn_id=<id> <record_type>: <said>`; `cookie` joined `SECRET_NAMES`; four new value-shape rules cover a bare JWT, a GitHub token, a connection URL's userinfo and a space-separated `--token` flag.
- Redaction moved to per-message content - `crates/verbatim-core/src/observe/provider.rs:293`. One call site, inside the `messages` map, before `request.to_string()`. The whole-body call is deleted.
- A fixture transcript carrying all seven shapes - `tests/fixtures/session-secrets.jsonl`, 10 turns, registered under `project-delta` in `TRANSCRIPT_FIXTURES` and `ROOTED_FIXTURES`.
- Wire-level tests - `mod wire` in `crates/verbatim-core/tests/egress.rs`, feature-gated on `testkit`. The harness ingests the fixture through the real `ingest::run` and drives `judgment::judge` against an `HttpStub`, so the assertions read the exact bytes `complete` hands to `net::post`. `cargo test -p verbatim-core --test egress --features testkit`: 17 passed, 0 failed. Without the feature: 11 passed, 0 failed, `tests/egress.rs`'s pre-existing cases unmodified.

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | bfa70fe | Match a header shape anywhere on a line, bounded to an auth scheme word plus one value run |
| 1 | 2 | aa86f1d | Redact a bare JWT and a GitHub token, over a shared `redact_runs` byte scanner |
| 1 | 3 | 5d2f800 | Redact URL userinfo and a space-separated secret flag |
| 1 | 4 | cc33ea0 | State the widened rule set and its tolerated failure in the module docs |
| 1 | fix | 7d919b9 | Redact a quoted secret flag value (`risk_surface` round 1) |
| 1 | fix | 48c412c | Stop a flag value at its own closing quote, not an inner one (`risk_surface` round 2) |
| 2 | 1 | 6ddc1d7 | Filter each message's content, not the serialized body |
| 2 | 2 | a92b8ae | A fixture transcript carrying all seven credential shapes |
| 2 | 3 | 08798de | Assert over the bytes the real call puts on the wire |
| 2 | 4 | f4d1d3e | Assert all seven shapes are gone and each rule said so |

## Deviations

None - plans executed as written.

## Open items

- **The blocking `risk_surface` gate caught a real leak on plan 1, twice.** Round 1: `value_run_end` stopped ON the opening quote, so `--token "tok"` read as an empty value and went out whole (fixed, 7d919b9). Round 2, on that fix: `flag_value_span`'s closer search took the FIRST match, so a value's own escaped quote ended the span early and everything after it shipped unredacted - `--token "ab\"cd"` lost `ab\` and sent `cd` (fixed, 48c412c). The one-round re-arm cap was spent at that point, so the second fix landed on the user's explicit call rather than on the gate's, and NO third review round fired over it. `48c412c` is covered by its own `#[cfg(test)]` case over all four quote shapes, each of which leaks its sentinel with the guard removed, but it has not been through an adversarial pass.
- **`the_filtered_body_is_the_same_document_with_different_values` does not assert any value changed** (`crates/verbatim-core/tests/egress.rs:548`). It compares message count, role order and top-level key sequence only, so it would stay green if `egress::for_destination` became a no-op. Raised at `low` by the plan-2 review and recorded as confirmed-and-not-fixed; the value-level assertion lives in `every_planted_shape_is_gone_and_its_rule_named_itself` beside it, so the coverage exists - the test's name over-promises what that one test does.
- **The unfixed-findings filing step could not run.** `issue-filing.mjs unfixed` refuses with `no-forge`: `git.forge_provider` and `git.forge_repo` are unset on this repository, so the three refuted findings from both plans' reviews could not be filed as issues. Nothing was dropped - they are in `ADJUDICATION-risk_surface-plan-1.json` and `ADJUDICATION-risk_surface-plan-2.json` with their counter-evidence. Running the forge setup step this repo never answered would let them file.
- **`crates/verbatim/tests/hook.rs` is flaky under full-workspace parallel load**, reported by both plans independently. `the_ingest_survives_a_group_kill_and_a_descendant_sweep` failed in every full-suite run and `every_event_exits_zero_with_an_empty_stdout_inside_the_budget` failed once on a p99 of 10.68 ms against a 10 ms budget. Run serially the file is 7/7, three times out of three. Pre-existing, names nothing in `observe`, and this phase touched neither.
- **`cargo fmt -p verbatim-core -- --check` reports pre-existing diffs** in `crates/verbatim-core/tests/retention.rs` and `crates/verbatim-core/tests/verify.rs`. Neither file was in either plan's lease and neither was touched.
- **The header rule answers for two of the seven shapes.** Plan 2's falsification pass removed one rule at a time and read which sentinels reached the wire: every rule maps to exactly one shape except rule 4, where removing it leaks both `Authorization` and `Cookie`, because both are its shape. That is the rule set's structure rather than a fixture gap.

## Goal check

The commits deliver the goal. The roadmap's diagnosis was that `provider.rs:254` called `egress::for_destination` on the already-serialized body, which made rules 3 and 4 inert; `6ddc1d7` deletes that call and moves the filter onto each `Message::content` inside the `messages` map, and `grep -n for_destination provider.rs` now shows one call site at line 293 plus the module-doc link. Success criterion 1 is met by construction rather than by assertion - `mod wire` drives the real `judgment::judge` against an `HttpStub` and reads the body after the first `\r\n\r\n`, and plan 2 recorded the falsification: restoring the whole-body call puts `pw-VBEGRESS-mash-4d1` on the wire and fails exactly the one test. Criteria 2 and 3 are `every_planted_shape_is_gone_and_its_rule_named_itself` and `the_filtered_body_is_the_same_document_with_different_values`, both green at 17/17 with `--features testkit`. Criterion 4 holds: `a_local_destination_sends_the_body_byte_identical` passes unmodified. Criterion 6 holds: `nothing_in_the_ingest_path_names_the_egress_filter` is present and green in both feature configurations. Two things are worth naming honestly rather than counting as delivered. Criterion 5 asks that the tolerated failure direction be stated in the module docs, and `cc33ea0` states it in AC7's terms and extends it to rules 8 and 9's sentinel-sized floors - but that floor is itself sized against short unrealistic sentinels (`MIN_GITHUB_TOKEN_BODY: usize = 6`, egress.rs:158) because realistic-length values would trip GitHub push protection in a public repo, so the docs state a tolerance that has not been measured against real token lengths. And the second quoted-value fix (`48c412c`) is the one change in this phase no adversarial pass ever saw, because the re-arm cap was spent when it landed; its own tests are honest about what they cover, but the gate did not get a look at it.
