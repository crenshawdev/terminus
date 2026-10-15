---
phase: 1
plan: 1
requirements: [PRIV-05]
files:
  - crates/terminus-core/tests/egress.rs
  - crates/terminus-core/src/ingest/pass.rs
  - crates/terminus/src/cmd/ingest.rs
  - crates/terminus-core/src/observe/mod.rs
  - crates/terminus-core/tests/judgment.rs
  - crates/terminus/tests/observations.rs
  - crates/terminus/tests/config_mode.rs
  - crates/terminus/src/cmd/observations.rs
  - crates/terminus/tests/cli.rs
  - crates/terminus-core/src/observe/judgment.rs
  - crates/terminus-core/src/observe/provider.rs
  - crates/terminus-core/src/observe/net.rs
  - crates/terminus-core/src/observe/cost.rs
  - crates/terminus-core/src/lib.rs
  - crates/terminus-core/tests/provider.rs
  - crates/terminus-core/tests/cost.rs
---

# Phase 1: No Model Calls - Plan 1

## Goal

Terminus never calls a model, never handles an API key and opens no network connection, matching Baley's ADR 0039. The summary half of observations goes with everything that exists only for it, while the mechanical half and the shape rules `redact_recall` uses stay.

## Must be true when done

- With `[privacy] redact_recall = true`, each sentinel planted in `tests/fixtures/session-secrets.jsonl` is missing from a recall context window over that session, and its rule's marker is there instead. With the knob off, every sentinel is present.
- An ingest pass makes no model request and prints no judgment notes. Every finished session still gets its mechanical observation, and `terminus observations` lists it.
- `terminus observations regenerate` rebuilds the `mechanical` column only. Its `--json` envelope is `since`, `selected`, `regenerated`, `notes`, and `--prompt-version` is an unknown flag (exit 2).
- `observe/judgment.rs`, `provider.rs`, `net.rs` and `cost.rs` and their test files are gone, and nothing in either crate names them except three doc comments on code plan 2 deletes (`cmd/doctor.rs`'s `host_of`, `testkit.rs`'s HTTP stub, `config.rs`'s `ResponseFormat`). Plan 2 Task 4's Verify proves the full-crate search empty.
- `cargo test --workspace --all-features` passes and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- Plan 1 of 4. The plans run in order because they share files (`observe/mod.rs`, `tests/egress.rs`, `store/schema.rs` and others).
- Binding decisions: D-07 (the `PLANTED` reroute), D-01's removal of `--prompt-version`, and D-08's rule that `net::attempts` and the loopback-stub tests go with `net.rs`.
- Later plans: `ureq`, the test kit's HTTP stub, credentials, doctor, egress rule 1 and the `[provider]` refusal are plan 2. The column drop and recall's `kind: observation` are plan 3. D-02's reason text and the docs are plan 4.

## Tasks

### Task 1: Prove every planted secret is caught on a recall projection

- **Files:** crates/terminus-core/tests/egress.rs (anchor: `mod wire` / `PLANTED`)
- **Action:** Per D-07, move `PLANTED`, `AFTER_THE_COOKIE`, `every_planted_shape_is_gone_and_its_rule_named_itself` and `a_mid_line_cookie_takes_its_value_and_leaves_the_turn_standing` out of `mod wire` and onto a recall projection.
  - **Store.** Build it the way `wire()` does now: the real `ingest::run` over `session-secrets.jsonl`, placed with `testkit::copy_rooted_fixture_into`.
  - **Knob.** Turn it on the only way the binary can: write a `terminus.toml` holding `[privacy] redact_recall = true` (and `false` for the falsifying half) and load it through `Config::load_from`, as `tests/redacted_recall.rs`'s `Bench::config` does. There is no in-memory setter, and the test must not invent one.
  - **Projection.** Use `recall::context::window` over the whole session: anchor it at the session's first turn, set `after` to `recall::MAX_CONTEXT_SIDE` (the fixture has 12 records, so one window covers it), and join every `ContextTurn::text`.
  - **Assertions.** For each `PLANTED` row, keep the existing three assertions but read the window instead of a request body: the sentinel is missing and its marker is present with the knob on, and the sentinel is present with it off. The cookie test asserts that the `Cookie` value is gone and that `AFTER_THE_COOKIE` survives in that same turn's text. Its `turn_id=` anchor assertions were about the judgment prompt, so they go.
  - **Why not `recall::get::records`.** A `get` body is the archived JSON line. There the `"password"` pair sits behind escaped quotes, so a different rule, or none, would fire, and the table would no longer pin which rule caught which shape.
  - **Gating.** Gate each new item with `#[cfg(feature = "testkit")]`, never at file level. The existing `wire` doc gives the reason: a file-level gate would silently retire the ungated `nothing_in_the_ingest_path_names_the_egress_filter`.
  - **Leave in `wire`.** Leave its four request-body tests for Task 4, which deletes them with the code they test: `a_secret_in_an_ingested_turn_does_not_reach_an_undeclared_destination`, `the_filtered_body_is_the_same_document_with_different_values`, `the_instruction_turn_survives_the_filter_byte_for_byte` and `the_fixture_carries_enough_turns_to_be_judged`.
- **Verify:** `cargo test -p terminus-core --features testkit --test egress` passes. The two moved tests sit outside `mod wire` and use no `HttpStub`. As a one-time falsification, remove the `redact_jwts` call from `egress::redact`: the planted-shape test must fail and name the JWT row. Put the call back afterwards.

### Task 2: The ingest pass stops judging

- **Files:** crates/terminus-core/src/ingest/pass.rs (anchor: `run_with` / `Summary`), crates/terminus/src/cmd/ingest.rs (anchor: `tree`), crates/terminus-core/src/observe/mod.rs (anchor: `judge_new`), crates/terminus-core/tests/judgment.rs, crates/terminus/tests/observations.rs, crates/terminus/tests/config_mode.rs
- **Action:**
  - **Pass.** `pass::run_with` no longer calls `observe::judge_new`, and `Summary` loses its `judgment` field. The snapshot step keeps its place after the lock drops. The docs on `run_with`, `Summary::snapshot` and `roll` currently explain themselves by pointing at the judgment step. Reword them so they stand on the snapshot's own reason: the `runs` row has committed, and a copy that failed becomes a note.
  - **Ingest command.** `cmd::ingest`'s `tree` loses the loop that prints `summary.judgment.notes`.
  - **`observe/mod.rs`.** Remove `judge_new`, `Judged` and `unjudged`. Task 3 owns `rejudge`, and Task 4 owns the module doc.
  - **Core test.** Remove `an_abandoned_reservation_is_taken_only_once_the_lease_has_lapsed` from `crates/terminus-core/tests/judgment.rs`, because it calls `observe::judge_new`. The rest of that file goes in Task 4.
  - **Integration tests.** In `crates/terminus/tests/observations.rs`, remove `with_judgment_off_a_pass_and_a_hook_run_reach_for_nothing`, `a_provider_call_in_flight_does_not_hold_the_ingest_lock` and `a_pass_whose_answer_cannot_be_used_still_records_itself_and_exits_zero`, along with the `NET` lock and the `observe::net` import that only they used. Leave the regenerate provider tests and the `provider` helper for Task 3.
  - **`config_mode.rs`.** Delete `crates/terminus/tests/config_mode.rs` entirely. Its first test asserts that `credentials::resolve` refuses a 0644 `terminus.toml` at the judgment step, which is a consumer this task removes. That refusal was v0.1.1 phase 3's UAT item (`v0.1.1/phases/3/UAT.md`). The file's other two tests only cover the same refusal's falsifying arms.
- **Verify:** `rg -n 'judge_new|summary\.judgment|observe::Judged' crates --glob '*.rs'` prints nothing. `cargo test --workspace --all-features` passes, including `a_finalized_session_is_listed_with_every_obs_01_fact`, which shows a pass still writes the mechanical observation. Clippy with `-D warnings` is clean.

### Task 3: `observations regenerate` rebuilds facts and nothing else

- **Files:** crates/terminus-core/src/observe/mod.rs (anchor: `rejudge` / `Regenerated` / `Selector`), crates/terminus/src/cmd/observations.rs (anchor: `regenerate` / `regenerate_args` / `regenerated_document`), crates/terminus/tests/observations.rs, crates/terminus/tests/cli.rs
- **Action:**
  - **`observe/mod.rs`.**
    - Remove `observe::rejudge`.
    - `Regenerated` keeps `selected`, `rewritten` and `notes`. It loses `judged`, `unasked`, and `sessions`, which existed only so `rejudge` could ask about the same set.
    - `Selector` loses `prompt_version`, and the selection SQL in `regenerate` loses its `prompt_version` clause, so `--since` is the only narrowing (D-01).
  - **`cmd/observations.rs`.**
    - `regenerate` no longer calls `rejudge` or prints the `provider_enabled` line.
    - `Regenerate` loses `prompt_version`, and `regenerate_args` loses its `--prompt-version` arm, so that flag now lands in the existing `other =>` misuse arm.
    - `regenerated_document` drops the `prompt_version` field, leaving `since`, `selected`, `regenerated` and `notes`.
    - The ingest lock still covers the mechanical rewrite. Reword the comment that justifies its scope by the HTTP calls after it. Also reword the `regenerate` and `Regenerated` docs that describe the judgment columns as belonging to `rejudge`.
  - **`tests/observations.rs`.**
    - Remove `a_prompt_version_no_row_carries_selects_no_row`, `regenerate_with_a_provider_re_judges_only_the_selected_rows`, `regenerate_without_a_provider_leaves_every_judgment_column_alone` and `regenerate_stops_on_a_spent_budget_and_says_what_it_left_unasked`.
    - Remove the `provider` helper and the `HttpStub` import, plus whatever only those tests used: `seed_judgment`, `topic`, `answer`, `SEEDED`, `JUDGED_TOPIC`, and whichever helper writes the `observe.daily_tokens` meta row.
    - `regenerate_rebuilds_the_selected_rows_and_leaves_the_rest_untouched` drops its `prompt_version` assertion.
  - **`tests/cli.rs`.** The `observations regenerate` envelope key list drops `"prompt_version"`.
  - **Out of scope here.** The listing's judgment fields stay until plan 3. This task touches the rebuild only.
- **Verify:**
  - `cargo test -p terminus --all-features --test observations --test cli` passes.
  - `cargo run -q -p terminus -- observations regenerate --prompt-version v1; echo $?` prints a misuse line naming the flag and exits 2. It fails during argument parsing, before any data directory or config is resolved.
  - `rg -n 'rejudge|prompt.version' crates/terminus-core/src/observe/mod.rs` and `rg -n 'prompt-version|args\.prompt_version' crates/terminus/src` both print nothing.
  - The full suite passes.

### Task 4: Delete the judgment, provider, network and cost code

- **Files:** crates/terminus-core/src/observe/judgment.rs, crates/terminus-core/src/observe/provider.rs, crates/terminus-core/src/observe/net.rs, crates/terminus-core/src/observe/cost.rs, crates/terminus-core/src/observe/mod.rs, crates/terminus-core/src/lib.rs, crates/terminus-core/tests/judgment.rs, crates/terminus-core/tests/provider.rs, crates/terminus-core/tests/cost.rs, crates/terminus-core/tests/egress.rs
- **Action:**
  - **Deletions.** Delete the four modules and their `pub mod` lines in `observe/mod.rs`. Delete `tests/judgment.rs`, `tests/provider.rs` and `tests/cost.rs` whole. Per D-08, `net::attempts` and the loopback-stub tests go with `net.rs`.
  - **`tests/egress.rs`.** Delete what Task 1 left of `mod wire`: the four request-body tests, `Wire`, `wire()`, `answer`, `top_level_keys`, `roles` and `MODEL`. Also delete the doc block above it, which describes a stubbed provider call.
  - **`observe/mod.rs` module doc.** Rewrite it to describe what remains: the mechanical half, written by the pass after `feedback::outcomes`, inserted and never overwritten, with failures recorded as notes in `runs.error`. Drop the paragraphs on the judgment half, D-07's lock ordering, "One door to the network" and the cost gates, and drop the "judgment half somebody paid for" clause.
  - **Leave for later tasks.** Two paragraphs stay as they are:
    - "Archival, never derived (D-02)". Plan 4 Task 1 rewrites its reason.
    - The `egress` paragraph. Plan 2 Task 3 rewrites it when `for_destination` and `scrub` go.
  - **`lib.rs`.** Remove the crate-doc paragraph saying the crate links an HTTP client through `observe::net`.
  - **What stays put for now.**
    - `ureq` stays declared until plan 2 Task 1. With `net.rs` gone, nothing names it.
    - Leave `egress.rs`, `config.rs` and `credentials.rs` alone. `for_destination`, `ResponseFormat` and `credentials::resolve` lose their callers but still compile, and each goes in plan 2 with the rest of its concern.
- **Verify:** `rg -n 'observe::(judgment|provider|net|cost)|mod (judgment|provider|net|cost)' crates --glob '*.rs' --glob '!crates/terminus/src/cmd/doctor.rs' --glob '!crates/terminus-core/src/testkit.rs' --glob '!crates/terminus-core/src/config.rs'` prints nothing. Those three files are excluded because each still carries one doc comment naming a deleted module, on code plan 2 deletes; they are outside this plan's lease, and plan 2 Task 4's Verify runs the search unexcluded. `ls crates/terminus-core/src/observe` lists only `egress.rs`, `mechanical.rs` and `mod.rs`. `cargo test --workspace --all-features` passes and clippy with `-D warnings` is clean.

## Notes

- **Departure from the CONTEXT Plan shape directive.** CONTEXT suggests putting the config refusal and doctor first and the redaction reroute last. This plan puts the reroute first and the refusal in plan 2. There are two reasons:
  - Every test that drives judgment (`tests/judgment.rs`, `provider.rs`, `cost.rs`, `credentials.rs`, the `wire` module, the provider sections of `crates/terminus/tests/observations.rs` and `doctor.rs`, and `config_mode.rs`) writes a `[provider]` table. A refusal landing first would fail all of them at that commit, so the refusal has to wait until that code and its tests are gone.
  - `every_planted_shape_is_gone_and_its_rule_named_itself` runs through `judgment::judge`. It has to be rerouted before `judgment.rs` is deleted. Otherwise the only test that ties each sentinel to its rule's marker would disappear between plans.
  - `ureq` and the test kit stub move to plan 2 Task 1 because of the 4-task ceiling.
- **Recalled memory.** These items describe code this plan deletes:
  - The 0644 `api_key` refusal at consumption (`v0.1.1/phases/3/UAT.md`, phase 3).
  - Phase 7's remote-provider UAT items (`v0.1.0/phases/7/UAT.md`) and the remote-observations CAPTURE item (`CAPTURE.md`, phase 7). Plan 4 removes the CAPTURE item.
- **Comments.** New or reworded comments follow `~/.claude/john-voice.md` "Code comments": short and plain.
