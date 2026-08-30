---
status: testing
phase: 2
fields_version: 1
started: 2026-08-30
updated: 2026-08-30
---

## Items

### 1. Planted secret never reaches the wire body
expected: A test drives provider::complete against testkit::HttpStub with local absent, reads the recorded body, and the planted secret is absent from it. Reverting the per-message redaction to the whole-body call makes that test fail.
criterion: AC1
status: pass
first_pass: pass
source: verifier
evidence: provider.rs:293 is the sole for_destination call site, inside the messages map before request.to_string(); tests/egress.rs:495 passes. Revert-falsification reproduced independently: the real rule set over the serialized-body shape leaves the planted password untouched, so the whole-body call fails that test.

### 2. All seven credential shapes are redacted and each rule names itself
expected: For each of the seven shapes planted in the ingested fixture transcript (Authorization: Bearer, a JSON "password" pair, a space-separated --token flag, a Cookie header, a bare JWT, a GitHub token, a connection URL with userinfo), the secret's characters do not appear in the stub-recorded body, and the body contains that rule's named marker constant.
criterion: AC2
status: pass
first_pass: pass
source: verifier
evidence: tests/egress.rs:284-320 PLANTED table and :640 every_planted_shape_is_gone_and_its_rule_named_itself pass with --features testkit; each sentinel is counter-checked present in the local=true body. Note three of seven share the generic REDACTED marker, so the marker half is weak for those three.

### 3. Redaction changes values, not the document shape
expected: The recorded body parses as JSON, and its message count, the role of each message in order, and its top-level key sequence equal those of a build with redaction removed.
criterion: AC3
status: pass
first_pass: pass
source: verifier
evidence: tests/egress.rs:548 passes; key order read off the body text, not a parsed map (:445). judge vs judge_again differ only in cost gating, so the two bodies are a fair comparison.

### 4. A local destination is left byte-identical
expected: With local = true the recorded body is byte-identical to the unredacted body, and the existing D-13 borrow test in crates/verbatim-core/tests/egress.rs passes unmodified.
criterion: AC4
status: pass
first_pass: pass
source: verifier
evidence: tests/egress.rs:44 passes unmodified including the Cow::Borrowed assertion; the stub-recorded local body carries all seven sentinels intact.

### 5. A mid-line Cookie redaction does not erase the rest of the turn
expected: A turn containing Cookie: <secret> mid-line renders with the text following the redacted value still present in the recorded body, and that turn's turn_id= anchor intact.
criterion: AC5
status: pass
first_pass: pass
source: verifier
evidence: tests/egress.rs:673 passes; reproduced directly on the fixture turn - trailing prose and turn_id=6 both survive.

### 6. The ingest guard still runs and the egress test file is not empty
expected: nothing_in_the_ingest_path_names_the_egress_filter passes, and cargo test --test egress reports a non-zero test count.
criterion: AC6
status: pass
first_pass: pass
source: verifier
evidence: cargo test --test egress: 11 passed with the guard green; with --features testkit: 17 passed. Per-module gate, no file-level #![cfg].

### 7. Module docs state the tolerated failure direction
expected: egress.rs's module docs state the over-matching direction as the tolerated failure: a name-substring false positive costs detail in one message.
criterion: AC7
status: pass
first_pass: pass
source: verifier
evidence: egress.rs:66-72 states the name-substring over-match as the tolerated failure in AC7's own terms, and the behaviour matches ('the monkey:' is redacted).

### 8. A double-quoted header shape is redacted by nothing
expected: behavior wrong - redact_header_spans defers every name whose preceding byte is '"' to the JSON-pair rule (egress.rs:363-366), but the JSON-pair rule only matches "name": "value". A header that opens inside a double-quoted string - the ordinary curl spelling - therefore falls through both rules and ships whole.
origin: verifier
status: pass
first_pass: fail
source: model
evidence: Fixed at ba5de15. `cargo test -p verbatim-core --lib observe::egress` -> 15 passed 0 failed, and `cargo test -p verbatim-core --test egress --features testkit` -> 17 passed 0 failed. The wire fixture now plants both curl spellings (sk-VBEGRESS-curlz-5e6, sid-VBEGRESS-qcrumb-1f9) as rows 8 and 9 of the PLANTED table, so every_planted_shape_is_gone_and_its_rule_named_itself covers them on the bytes the real call puts on the wire. Falsification recorded before the fix landed: restoring the quote exemption and the value_run_end span failed exactly the three new cases (a_double_quoted_header_name_is_still_a_header, a_double_quoted_cookie_header_is_still_a_header, a_quoted_header_value_is_taken_from_inside_its_quotes) while a_json_pair_is_still_rule_5s_and_keeps_its_comma passed both ways, which is what shows the exemption was redundant for the JSON pair it named. Full verbatim-core suite green; verbatim crate green with hook.rs 7/7 run serially (its parallel-load flakiness is pre-existing and documented in the phase SUMMARY).
reported: behavior wrong - redact_header_spans defers every name whose preceding byte is '"' to the JSON-pair rule (egress.rs:363-366), but the JSON-pair rule only matches "name": "value". A header that opens inside a double-quoted string - the ordinary curl spelling - therefore falls through both rules and ships whole.
severity: major
cause: egress.rs:367-369 skips a header name whose preceding byte is '"', deferring it to rule 5 unconditionally. But redact_json_pairs (egress.rs:461-476) requires the quoted run to CLOSE before the colon: quoted_end finds the closing quote, then demands ':' immediately after it. In the curl spelling `-H "Authorization: Bearer <secret>"` the opening quote precedes the name but the run closes AFTER the value, so quoted_end returns the trailing quote, `name` becomes the whole `Authorization: Bearer <secret>` run, is_secret_name rejects it, and the pair rule declines too. Both rules pass and the header ships whole. The exemption is written as if a preceding quote implied a JSON pair; it only implies the name is quoted, which the curl form also satisfies. Single-quoted spellings are unaffected because ' is not the exempted byte.
fix: ba5de15, retest

## Summary

total: 8
passed: 8
failed: 0
pending: 0
skipped: 0
blocked: 0
reworked: 1
