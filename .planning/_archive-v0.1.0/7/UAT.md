---
status: testing
phase: 7
fields_version: 1
started: 2026-08-22
updated: 2026-08-22
---

## Items

### 1. Cold start from scratch
expected: Stop everything, clear ephemeral state, start from scratch - a fresh store opens clean, the observations table is created by bring_forward on an already-initialized store with no reindex, and one primary query (verbatim observations) returns real data.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: schema.rs::the_observations_table_reaches_a_store_that_predates_it (drop, reopen, table back, no derived rebuild) plus observations.rs::a_finalized_session_is_listed_with_every_obs_01_fact returning a populated document; the store-older-than-the-table arm returns a reason and exit 0 rather than leaking `no such table` (crates/verbatim/tests/observations.rs:549).

### 2. Mechanical observations with judgment off
expected: For a finalized session with judgment disabled, `verbatim observations` lists that session's files, tools, commands with their arguments, errors, branch, commits, turn count and duration, and the instrumented network seam records zero connection attempts.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: crates/verbatim/tests/observations.rs:435 asserts every OBS-01 fact by name in --json and on the human listing (command keeps its arguments, commit subject, branch, 6 turn(s), 1m 40s); :733 asserts net::attempts::count()==0 for a pass and that a live configured stub endpoint got zero requests across a spawned pass and a hook run.

### 3. One judgment call, anchored claims, gates hold
expected: With judgment enabled against the local endpoint, a finalized session above the minimum turn count produces exactly one HTTP request and stored JSON validating against the schema, every decisions/learned/unresolved entry carrying a turn_id that resolves to a real turn row; a session below the minimum turn count, and any session after the daily token budget is exhausted, produce zero requests.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: judgment.rs:193 (attempts==1, one stub request, every anchor re-queried against `turns` and confirmed to belong to that session); cost.rs:152 and :194 (attempts==0 for TooShort and BudgetSpent, spend row unmoved, each with a falsifying positive half).

### 4. Parse failure arm retries once and does not block ingest
expected: Against a stub endpoint returning unparseable content, exactly two requests are made and the run is stored with status = parse_failed holding the raw response, and an ingest pass over the same session still completes and writes its runs row.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: judgment.rs:387 (attempts==2, status parse_failed, raw stored and scrubbed, claim columns null, both requests charged, no third request later); crates/verbatim/tests/observations.rs:822 the pass records itself and exits 0; pass.rs:124 runs judgment after the runs row commits and observations.rs:771 proves the lock is free during the call.

### 5. Same code path against a remote OpenAI-compatible endpoint
expected: Switching only base URL, model and key between the local endpoint and a remote OpenAI-compatible one produces a stored observation both times, with no other configuration changed. (human-verify: needs a remote OpenAI-compatible API key - none present on this machine)
criterion: AC4
status: pass
first_pass: fail
source: model
evidence: Live remote run against DeepSeek (api.deepseek.com/v1/, deepseek-chat) at commit f4d5663, isolated scratch config+data+corpus, fixture session-basic.jsonl (8 turns, is_final, above MIN_TURNS=6), credential resolved from the [deepseek] namespace of ~/.config/jcrenshaw/credentials.toml (mode 600). `verbatim ingest` -> exit 0; `verbatim status` reports 1 session, 8 turns, files_committed 1, error null. The observations row reads status=ok, model=deepseek-chat, prompt_version=obs-judgment-2, topic 'Reading and testing session-basic.jsonl fixture', outcome 'completed'. Every claim anchor resolves: decisions turn_id 1 -> 1 row; learned turn_ids 0, 5, 7 -> 1 row each; unresolved empty. Turn id range 0-7 over 8 turns. The two earlier 400s are both closed: the missing sibling json_schema object, and deepseek-chat refusing json_schema mode outright. Local half: no local endpoint exists on this machine now (ollama not running, neither ollama nor llama-server installed), so it rests on item 3's passing proof against the loopback stub plus the 2026-08-21 live ollama probe recorded in CONTEXT D-08/D-09. Regression guards added and passing: provider.rs call::the_schema_rides_the_request_in_the_field_a_strict_endpoint_reads, call::the_json_object_mode_sends_the_shape_deepseek_accepts, call::the_none_mode_sends_no_response_format_even_with_a_schema, call::a_request_with_no_schema_names_no_response_format. Suites green at this commit: provider 17, config 27, judgment 6, cost 12, credentials 11, egress 8, observe 7, observations 17, cli 13. AC4 amended during this UAT (user decision, 2026-08-22): the three-settings claim holds for schema-capable endpoints; a narrower one needs response_format, which selects a field value, not a code path, so OBS-05's single code path is intact.
severity: major
cause: provider::complete sends D-09's response_format as the bare constant {"type":"json_schema","strict":true} (crates/verbatim-core/src/observe/provider.rs:56-58) with no `json_schema` object beside it - the schema travels in the system message instead. OpenAI's json_schema mode requires a sibling `json_schema` field carrying name+schema, and DeepSeek enforces it. Local ollama and the loopback stub both accept the truncated shape, which is why every existing test passes. SUMMARY.md flagged this exact risk as an open item and called it a one-line change in provider.rs; this run confirms it against a real remote endpoint.
fix: f4d5663, verified live

### 6. Loose credentials file is refused and the key never leaks
expected: A credentials file with mode 0644 is refused with a message naming the file and its mode, no observation is generated, and no assertion over stdout, stderr, runs.error or the observations table finds the key's value.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: credentials.rs:140/:170/:189 (message names path and 644, every group/world bit refused, 0600 loads as the falsifier, both formatters render REDACTED); observe/mod.rs:441-449 returns from judge_new before the store opens so no observation row is written; egress.rs:108/:133 the scrubber is unconditional and destination-independent.

### 7. No connection with judgment off; egress filtered by destination
expected: A full ingest and hook run with judgment disabled records zero connection attempts at the instrumented seam and no file outside the provider module names the HTTP client type; with local absent or false the request body is filtered before send, and with local = true it is sent unfiltered.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: provider.rs:29 source-level single-namer test; observations.rs:733 two-instrument zero-attempt proof; provider.rs:476 drives local=true and local=false over the same body and asserts the secrets are present in one and absent in the other, with the filtered body still valid JSON.

### 8. regenerate --since is scoped and reindex preserves observations
expected: `verbatim observations regenerate --since <date>` rebuilds only sessions after that date and leaves earlier rows unchanged, and `verbatim reindex` on the same store leaves every observation row present.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: observations.rs:587 (both rows spoiled first, only the selected one rebuilt); reindex.rs:414 (the observation row survives a rebuild unmoved); cli.rs:801-803 holds the two-word command to the envelope contract.

### 9. Switch the [provider] block from the local endpoint to a remote OpenAI-compatible one, changing only base_url, model and the key, and run one judgment
expected: A stored observation both times with no other configuration changed; watch for a strict endpoint rejecting `response_format: {"type":"json_schema","strict":true}` sent with no `json_schema` object beside it (SUMMARY open item, one-line change in provider.rs).
origin: verifier
why_human: Out-of-reach resource: it needs a remote OpenAI-compatible API key, and no such key exists on this machine. CONTEXT tags AC4 human-verify for exactly this reason; no probe here can reach a remote endpoint without one, and the network is off limits to this pass.
status: pass
first_pass: fail
source: model
evidence: Live remote run against DeepSeek (api.deepseek.com/v1/, deepseek-chat) at commit f4d5663, isolated scratch config+data+corpus, fixture session-basic.jsonl (8 turns, is_final, above MIN_TURNS=6), credential resolved from the [deepseek] namespace of ~/.config/jcrenshaw/credentials.toml (mode 600). `verbatim ingest` -> exit 0; `verbatim status` reports 1 session, 8 turns, files_committed 1, error null. The observations row reads status=ok, model=deepseek-chat, prompt_version=obs-judgment-2, topic 'Reading and testing session-basic.jsonl fixture', outcome 'completed'. Every claim anchor resolves: decisions turn_id 1 -> 1 row; learned turn_ids 0, 5, 7 -> 1 row each; unresolved empty. Turn id range 0-7 over 8 turns. The two earlier 400s are both closed: the missing sibling json_schema object, and deepseek-chat refusing json_schema mode outright. Local half: no local endpoint exists on this machine now (ollama not running, neither ollama nor llama-server installed), so it rests on item 3's passing proof against the loopback stub plus the 2026-08-21 live ollama probe recorded in CONTEXT D-08/D-09. Regression guards added and passing: provider.rs call::the_schema_rides_the_request_in_the_field_a_strict_endpoint_reads, call::the_json_object_mode_sends_the_shape_deepseek_accepts, call::the_none_mode_sends_no_response_format_even_with_a_schema, call::a_request_with_no_schema_names_no_response_format. Suites green at this commit: provider 17, config 27, judgment 6, cost 12, credentials 11, egress 8, observe 7, observations 17, cli 13. AC4 amended during this UAT (user decision, 2026-08-22): the three-settings claim holds for schema-capable endpoints; a narrower one needs response_format, which selects a field value, not a code path, so OBS-05's single code path is intact.
severity: major
cause: provider::complete sends D-09's response_format as the bare constant {"type":"json_schema","strict":true} (crates/verbatim-core/src/observe/provider.rs:56-58) with no `json_schema` object beside it - the schema travels in the system message instead. OpenAI's json_schema mode requires a sibling `json_schema` field carrying name+schema, and DeepSeek enforces it. Local ollama and the loopback stub both accept the truncated shape, which is why every existing test passes. SUMMARY.md flagged this exact risk as an open item and called it a one-line change in provider.rs; this run confirms it against a real remote endpoint.
fix: f4d5663, verified live

## Summary

total: 9
passed: 9
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 2
