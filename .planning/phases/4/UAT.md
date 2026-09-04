---
status: testing
phase: 4
fields_version: 1
started: 2026-09-04
updated: 2026-09-04
---

## Items

### 1. Four projections leak by default, filter when the knob is on
expected: A secret planted in a fixture transcript comes back with its characters intact from recall_search's excerpt, recall_context's window, recall_get's body and the SessionStart brief's quote when [privacy] redact_recall is absent; with it set true and nothing else changed, none of the four contains the secret and each carries that rule's marker constant.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: Knob-off/knob-on pairs over one store at library level (redacted_recall.rs:139/:152, recall.rs:1693/:1710 and :1915/:1930, inject_brief.rs:680/:702) and at the process boundary (mcp.rs:1819 for all three tools, brief.rs:429 for the SessionStart hook). Filtered output carries config::REDACTED, the marker the header rule pushes at egress.rs:530/:609/:696, and none of AUTHZ_SENTINEL / COOKIE_SENTINEL / VBEGRESS. Five entry points resolve Redaction::of - search.rs:355,:430, context.rs:202, get.rs:219, brief.rs:89, prompt.rs:444.

### 2. Default output is byte-identical to the pre-phase build
expected: With the boolean absent, the golden test over the fixture store finds all four projections byte-identical to the pre-phase capture, and every existing test in tests/mcp.rs, tests/hook.rs and the recall test files passes unmodified.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: recall_golden.rs:276 against tests/goldens/recall-projections.txt, whose only commit is 6499e53, the phase's first commit ahead of the knob (3c8977e); recall_golden.rs:308 proves the golden holds a live sentinel. Test files show 1214 insertions / 1 deletion since 6499e53~1, the deletion being an import line. One full workspace run with testkit: zero failures.

### 3. The archive on disk is untouched in both settings
expected: Over one fixture store ingested once, the stored blob bytes and verbatim verify --json digests are equal with the boolean on and off, and nothing_in_the_ingest_path_names_the_egress_filter still passes.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: recall_cli.rs:1388 turning_the_knob_on_leaves_the_archive_byte_for_byte_where_it_was - one ingest, blob-by-blob equality and identical `verify --json` documents across the flip, with ok/failures/checked asserted so the equality is two clean passes. egress.rs:231 nothing_in_the_ingest_path_names_the_egress_filter still passes.

### 4. --raw is the owner's only escape hatch, and no env var is
expected: With the boolean on, verbatim show <id> prints the marker and verbatim show <id> --raw prints the secret's bytes; a verbatim hook SessionStart run and an MCP recall_get call both emit filtered output no matter what environment variables are set on the spawn.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: show.rs:49 and search.rs:74 use Config::without_recall_redaction() per invocation; config.rs:724 is the only assignment to the resolved field and reads the TOML alone. recall_cli.rs:1235 and :1308 prove marker-then-raw at the process boundary with `--raw` output equal to the no-config run; recall_cli.rs:1119 proves no other command takes the flag; brief.rs:467 and mcp.rs:1907 prove RAW, VERBATIM_RAW and VERBATIM_REDACT_RECALL=false on the spawn change nothing.

### 5. A judgment is filtered before it reaches SQLite
expected: With the boolean on, a provider response whose topic, outcome and one claim text each carry a planted secret produces observations rows in which a direct SQL read of all three columns finds the marker and none of the secret's characters.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: judgment.rs:817 `redacted` scrubs topic, outcome and every claim text at the store call site (judgment.rs:365), turn_ids untouched. tests/judgment.rs:814 reads topic, decisions, learned and unresolved by SQL and finds the marker and no sentinel, under a config declaring provider.local = true. The outcome half is settled in the stronger direction by tests/judgment.rs:868, as the recorded deviation states.

### 6. Rows written before the flip are filtered on read
expected: An observation row stored while the boolean was absent, then queried through recall_search with kind: "observation" and the boolean on, returns the marker rather than the secret's characters.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: search.rs:355 resolves the redaction on the observation branch. redacted_recall.rs:309 inserts one claim directly into `observations`, reads it twice under the two configs, gets the claim verbatim then the marker, keeps every identity field equal, and re-reads the column by SQL to show the stored row still holds the sentinel.

### 7. The hook holds its wall budget with redaction on
expected: crates/verbatim/tests/hook.rs passes its p99 wall budget over 100 runs of every event with the boolean on.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: hook.rs:802 every_event_stays_inside_the_budget_with_the_redaction_knob_on: 100 runs of each of the four events with the knob written into the hook's config dir, exit 0 and empty stdout per run, p99 < 10 ms per event. hook binary 8/8.

## Summary

total: 7
passed: 7
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 0
