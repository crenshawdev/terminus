---
status: testing
phase: 5
fields_version: 1
started: 2026-08-20
updated: 2026-08-20
---

## Items

### 1. SessionStart brief: one JSON object, in budget, under wall budget
expected: SessionStart against a store with indexed history for the payload's project emits exactly one JSON object with hookSpecificOutput.hookEventName = SessionStart, additionalContext within the configured budget, p99 under the asserted wall budget over 100 runs.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: cargo test -p verbatim --test brief -- --nocapture: 'SessionStart brief: p50 1.27 ms, p99 2.36 ms over 100 runs', 1 passed; the test parses each run's single stdout line, checks hookEventName=SessionStart and additionalContext <= the brief_chars written into verbatim.toml.

### 2. Brief is deterministic and clock-free
expected: Two SessionStart runs against an unchanged store produce byte-identical stdout, and no time-of-day pattern appears anywhere in the output.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: two_session_starts_against_an_unchanged_store_are_byte_identical passes: both spawns' stdout compared byte for byte, and a char-window NN:NN scan over the output finds no time of day.

### 3. Relative path in prompt finds the stored absolute turn
expected: A UserPromptSubmit prompt naming a path relative to the payload cwd that a past session edited (stored absolute) emits that turn; the same prompt with the path removed emits nothing and exits 0.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: a_prompt_naming_a_file_gets_the_turn_that_edited_it passes; independent live probe of the built binary injected 'turn 1 (2026-08-13): Edit <abs>/crates/gizmo/lantern.rs ... lanternFlicker' for the prompt 'what changed in crates/gizmo/lantern.rs', and the same prompt with the path removed wrote nothing at exit 0.

### 4. Repeat prompt suppressed, reason on disk
expected: The same entity-bearing prompt fed twice under one session_id injects on the first and emits nothing on the second, with the suppression reason readable in the per-session state file.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: the_same_prompt_twice_in_one_session_is_answered_once passes with a cross-session control; live probe left $DATA/injection/<session>.json holding {"injected":[1],"suppressed":[{"turn_id":1,"reason":"already_injected"}]} and the second prompt wrote nothing.

### 5. Post-compaction prompt draws only from dropped turns
expected: After a SessionStart with source "compact", the next UserPromptSubmit draws candidates only from turns absent from preservedMessages.uuids, at most 3; a later prompt in the same session no longer does.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: after_a_compaction_the_next_prompt_draws_from_what_fell_out passes, with an unflagged control proving the prompt reaches both turns and ranks the preserved one FIRST, so the restricted answer is not an artifact; compaction_owed flips true on source=compact and false after the prompt spends it; a_compaction_whose_boundary_is_not_committed_yet_keeps_the_debt covers the ingest race; MAX_TURNS=3 asserted by at_most_three_turns_are_ever_injected.

### 6. Missing or locked store: silent exit 0 inside deadline
expected: With the store file deleted, and separately with the store held by an exclusive writer past the deadline, UserPromptSubmit exits 0 with empty stdout inside the deadline.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: tests/inject.rs 7/7 pass including the deleted-store, exclusive-writer and not-a-database cases, each wall-clock bounded; live probe after deleting verbatim.db: exit=0, empty stdout, 1 ms.

### 7. Free-text-only match emits nothing
expected: A prompt whose only match is free text - no rank 1-3 exact entity, no two independent co-occurring entities - emits nothing.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: a_prompt_that_matches_only_free_text_writes_nothing passes for both shapes; eligible() requires an entity match plus rank<3 or >=2 distinct entities (inject/prompt.rs:290-299); live probe of a generic prose prompt wrote nothing at exit 0.

### 8. Live resume brief in a real session
expected: Starting a real Claude Code session in a project with indexed history, the session begins with the resume brief: last session's date, branch, and final exchange, with no error banner from the hook.
status: pass
first_pass: fail
reported: that was a quick answer no search - fresh interactive session in /code/cadence answered the last-session branch question from injected context, no error banner
severity: major
cause: Nothing from phase 5 is deployed. ~/.local/bin/verbatim is a stale Aug 8 binary from an earlier incarnation (subcommands precompact/inject, no `hook <event>` dispatch); even its hooks report all four entries missing from settings.json. The current repo has never been built (no target/ dir), no hooks are wired anywhere (0 verbatim mentions in user or project settings), and no store/indexed history exists. The brief cannot appear: no hook fires, and there is no binary or index for it to read.
fix: deployment, not code: built current binary, verbatim install --yes wired 4 hooks + mcp, backfill archived 3206 sessions / 401306 turns; doctor clean; direct SessionStart probe emits the brief. Retest in a fresh session

### 9. Start a real Claude Code session in a project with indexed history and read the opening context
expected: The session begins with the resume brief - last session's date, branch and final exchange - and no red hook_non_blocking_error banner appears.
origin: verifier
why_human: Out-of-reach resource: it needs an interactive Claude Code harness rendering hook stdout. The brief bytes themselves were produced and inspected here from the real binary; what cannot be observed from this process is the harness accepting and displaying them.
status: pass
first_pass: fail
reported: same interactive session: brief present in context, no hook error banner rendered
severity: major
cause: Same as item 8: current binary never built or installed, hooks unwired, no store.
fix: same deployment fix as item 8, retest

## Summary

total: 9
passed: 9
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 2
