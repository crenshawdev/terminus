---
status: testing
phase: 1
fields_version: 1
started: 2026-10-13
updated: 2026-10-13
---

## Items

### 1. Cold start on a fresh and an upgraded store
expected: From a clean build, a brand-new store opens and archives a session, a copy of an old store with the summary columns opens and drops them, and a search on each returns real turns.
origin: smoke
status: pass
first_pass: pass
source: verifier
evidence: Current debug build on temp stores. Fresh: bare `ingest` exit 0, and `search lantern.rs` returned turn 1. Upgraded (the ten columns and the meta key added to a copy): `status` exit 0 dropped them to 4 columns, and `search lantern.rs` returned 1 hit.

### 2. No HTTP or TLS crate, and a socket in source fails the build
expected: `cargo tree --workspace -e all --target all` lists no ureq, reqwest, hyper or rustls, and a test fails when any file under crates/*/src names std::net, TcpStream, TcpListener or UdpSocket.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: `cargo tree --workspace -e all --target all` names no ureq, reqwest, hyper or rustls (or ring, webpki, httparse, tokio). tests/no_network.rs passes, and it fails naming the file when `use std::net::TcpStream;` is planted in a copy of either crate's src.

### 3. No API key lookup and no credentials.toml path
expected: A source scan finds no environment lookup of an _API_KEY name and no credentials.toml path outside the redaction test fixtures.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: No `_API_KEY` or `credentials.toml` in crates/*/src; the only hits are input text in tests/egress.rs:24,49. credentials.rs is deleted, and api_key appears in src only in withhold_secret_excerpt and egress unit-test input.

### 4. A leftover [provider] table refuses to load
expected: A terminus.toml with an empty [provider] table, or one holding api_key = "<sentinel>", makes `terminus status` exit non-zero with a message naming the file and saying model calls were removed, and the sentinel appears in neither stdout nor stderr.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: `terminus status` exited 1 for an empty table, an api_key sentinel, `enabled = false`, a bare scalar and a mixed file. Each stderr named the file and said model calls were removed, and no sentinel appeared on either stream. Refusal at config.rs:499-501, ProviderRemoved at error.rs:159.

### 5. An old store loses the ten summary columns and nothing else
expected: A store with the old summary columns opens with none of the ten left in PRAGMA table_info(observations), every mechanical document is byte-identical, `terminus verify` passes, and a fresh store's observations columns equal the upgraded store's.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: On a 14-column temp store, the open went to 4 columns with mechanical byte-identical, the meta key gone, the layout equal to a fresh store's, `verify` 0 failed and a second open clean. `a_store_with_the_summary_columns_opens_without_them` passes.

### 6. A new session still gets a mechanical observation
expected: A session finished after the change is listed by `terminus observations`, `terminus search` finds its files, commands and errors, and recall_search no longer offers kind: observation and its description no longer mentions summaries.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: A finalized temp session was listed with only the four keys; mechanical held the edited file, the command with its arguments and the error. `terminus search` found the file, command and error turns. recall_search's kind description lists four record types and no summaries, and there are 3 tools.

### 7. Every planted secret is redacted on every recall surface
expected: With redact_recall on, every secret planted in session-secrets.jsonl is absent from recall_search excerpts, recall_context, recall_get and the SessionStart brief, and each rule's marker appears in its place.
criterion: AC6
status: pass
first_pass: fail
source: model
evidence: After 840e1e3: `cargo test -p terminus-core --features testkit --test egress` -> 10 passed 0 failed, including every_planted_shape_is_gone_from_a_recall_get_body (all 9 PLANTED sentinels through recall::get::records over all 12 fixture turns, knob on: each gone, marker present) and every_planted_shape_is_gone_and_its_rule_named_itself (context window). With get.rs reverted to `.apply`, the get test failed on pw-VBEGRESS-mash-4d1. redacted_recall 6 passed, inject_brief 13 passed, terminus brief + mcp 30 passed. Full `cargo test --workspace --all-features` -> 749 passed, 0 failed. risk_surface review (openai gpt-5.6-sol): 1 raised, 1 refuted, 0 survivors.
reported: behavior wrong: with redact_recall on, recall_get still returns the planted JSON password value whole. The gap predates the phase, but AC6 names recall_get and the SUMMARY reports AC6 as proven.
severity: major
cause: recall_get (recall/get.rs:236-240) runs the shape rules over the raw archived JSON line, but the rules are written for decoded text. A JSON pair typed inside a message is stored with escaped quotes (`\"password\":\"pw-VBEGRESS-mash-4d1\"`, tests/fixtures/session-secrets.jsonl line 4). Rule 4's quoted_end (observe/egress.rs:536-545) treats `\"` as an escape, so the name it reads runs to the end of the text field and is never a secret name; rule 5 stops an unquoted value at a backslash, so `KEY=\"value\"` in a raw line redacts nothing either. recall_search and recall_context redact decoded turn text, so they catch it. Predates this phase; the get tests read only the Authorization turn.
fix: 840e1e3, retest

### 8. Doctor drops the key checks and the suite passes
expected: `terminus doctor --json` carries no credentials, config_mode, mode_checks or provider_local key, and `cargo test --workspace --all-features` passes.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: `doctor --json` contains none of credentials, config_mode, mode_checks or provider_local anywhere in its output. The full `cargo test --workspace --all-features` run exited 0 with 748 passed and 0 failed.

### 9. observations regenerate rebuilds the mechanical facts only
expected: `terminus observations regenerate` rebuilds the mechanical column and refuses --prompt-version.
status: pass
first_pass: pass
source: verifier
evidence: `regenerate --json` returned exactly since, selected, regenerated and notes, and mechanical was unchanged. `--prompt-version v1` exited 2 with `unexpected argument '--prompt-version'`.

### 10. kind observation is refused by name
expected: `terminus search --kind observation` exits 2 naming the retired kind, and recall_search with kind: observation returns isError rather than an empty result.
status: pass
first_pass: pass
source: verifier
evidence: `search --kind observation` exited 2 with `kind "observation" is no longer offered: session summaries were removed from Terminus`. MCP recall_search with kind observation returned isError true with the same reason.

### 11. Source comments still describe the removed judgment half and provider credential
expected: stale: four comments describe deleted code as current, one of them on the redaction path.
origin: verifier
status: pass
first_pass: fail
source: model
evidence: After ce67f81: `grep -n 'judgment\|provider credential\|loopback stub'` over ingest/pass.rs, recall/search.rs, testkit.rs and observe/mechanical.rs prints nothing; `cargo check -p terminus-core` clean.
reported: stale: four comments describe deleted code as current, one of them on the redaction path.
severity: cosmetic
cause: Comments outside every plan's lease were not updated when the code they describe was deleted: ingest/pass.rs:318-321 (judgment half outside the lock, slow provider), recall/search.rs:407-410 (rule 1 fed the provider credential), testkit.rs:58-60 (real judgment request against a loopback stub), observe/mechanical.rs:6-8 (a machine with judgment disabled).
fix: ce67f81, retest

### 12. Copy one rolling snapshot `terminus-<time>.db` from the real data directory into an empty temp directory as `terminus.db`, run this branch's `terminus status` with TERMINUS_DATA_DIR pointed there, then `sqlite3 <dir>/terminus.db 'PRAGMA table_info(observations)'` and `SELECT count(*) FROM observations`, and run `terminus verify`.
expected: Four columns (session_key, session_id, generated_at, mechanical), the same row count as before the open, no `observe.daily_tokens` meta row, and verify reporting 0 failed.
origin: verifier
why_human: Out-of-reach resource: it needs the owner's real store, which this verification was told not to touch. The drop was observed on a simulated old store and by the AC4 test. The SUMMARY says it ran on the 2026-10-12 snapshot (14 to 4 columns, 6,440 rows, verify 0 failed), but I could not observe that run.
status: pass
first_pass: pass
source: model
evidence: Copied snapshots/terminus-20261012T150811.267Z.db to a scratch dir as terminus.db. Before: 14 observations columns, 6440 rows, 1 observe.daily_tokens meta row, sha256 of (session_key, hex(mechanical)) 049108a2...2056b8. `TERMINUS_DATA_DIR=<scratch> target/debug/terminus status` exit 0. After: columns session_key,session_id,generated_at,mechanical; 6440 rows; 0 meta rows; same sha256 049108a2...2056b8. `terminus verify` -> `6442 session(s) checked, 0 failed`, exit 0.

## Summary

total: 12
passed: 12
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 2
