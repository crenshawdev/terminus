---
phase: 2
plan: 1
requirements:
  - PRIV-01
files:
  - crates/verbatim-core/src/observe/egress.rs
---

# Phase 2: Egress Filter Sees What Is Sent - Plan 1

## Goal

The rules in `crates/verbatim-core/src/observe/egress.rs` catch the seven shapes
the roadmap names when they appear in transcript text - a turn collapsed to one
line behind a `turn_id=` prefix - rather than only in the hand-written header
samples the current tests feed them.

## Must be true when done

- A header shape found MID-LINE, behind a `turn_id=<id> <record_type>: ` prefix,
  is redacted; `Authorization: Bearer <value>` loses the scheme word and the
  value together and reads `Authorization: [redacted]`.
- The prose that follows a redacted header value on the SAME line is still
  there, and so is that line's `turn_id=` anchor.
- `cookie` is a secret name, so a `Cookie:` header is a shape the rules test.
- Four shapes that carry no name beside them - a bare JWT, a GitHub token, a
  connection URL carrying userinfo, and a space-separated `--token` flag - are
  each redacted, and each leaves its OWN marker naming which rule fired.
- Ordinary prose, an ordinary compact JSON document and a pretty-printed one all
  come back byte-identical: every existing test in
  `crates/verbatim-core/tests/egress.rs` passes with that file unmodified.
- The module docs list every rule the set now runs and state the tolerated
  failure direction, and no rule's doc assumes its input is a JSON request body.

## Context

CONTEXT locks: D-03/D-04 (rule 3 matches mid-line and stops at a bounded
terminator), D-09 (`cookie` joins `SECRET_NAMES`; four new value-shape rules),
D-10 (pinned GitHub prefix list), D-11 (hand-written byte scanners, no regex
crate), D-12 (loose length floors sized against this repo's short sentinels),
D-13 (a named marker constant per new rule), D-06 (no rule may assume JSON
input, because phase 4 reuses this set over recall excerpts and brief windows).

D-05: widening `redact` widens `scrub` too - one rule set, two entry points - so
every provider error string and `runs.error` note passes through the new rules.
That is accepted, not worked around: it is v0.1.0 phase 7's D-16
(`_archive-v0.1.0/7/CONTEXT.md`) doing its job - the redaction boundary
covers error construction, because `runs.error` is free text with no log file
behind it and `verbatim status` prints it, so a leak there is durable.

This plan's lease is `egress.rs` alone, so its own tests go in a `#[cfg(test)]
mod tests` inside that file, which is this workspace's established pattern (18
source files carry one). `crates/verbatim-core/tests/egress.rs` belongs to plan
2 and must keep passing here unmodified.

## Tasks

### Task 1: Match a header shape anywhere in a line, and bound what it takes

- **Files:** crates/verbatim-core/src/observe/egress.rs (start at
  `redact_header_lines` and `header_name`; `SECRET_NAMES` changes too)
- **Action:** `header_name` calls `line.find(':')`, so rule 3 only ever tests the
  FIRST colon on a line. `judgment::transcript` writes every turn as
  `turn_id=<id> <record_type>: <said>` and `one_line` collapses the turn to a
  single line, so that first colon always sits behind a prefix carrying a space
  and an `=`, which `header_name`'s bare-token test rejects - the rule is inert
  on transcript text even once the serialization problem is fixed (D-03). Make
  the rule consider EVERY colon on the line, taking the run of ASCII letters,
  digits, `-` and `_` immediately before each one as the candidate name and
  keeping the existing `is_secret_name` test on it. A candidate must be a WHOLE
  token: the byte before the run is the line start or a byte that is not part of
  a name, and it must NOT be a double quote - `  "api_key": "x",` is rule 4's
  case, and rule 3 taking that line would swallow the trailing comma and hand
  the endpoint a document that is not JSON, which
  `a_pretty_printed_json_pair_keeps_its_punctuation` already pins. Add `cookie`
  to `SECRET_NAMES` (D-09), so a `Cookie:` header is a name this rule tests.
  Replace from the colon through the value with a BOUNDED span rather than
  through the rest of the line as today's arm does (D-04): a whole turn is one
  line, so running to end of line erases the rest of the turn from the prompt
  while leaving its `turn_id=` anchor standing, and the model can still anchor a
  claim to a turn it was shown a fragment of. Two properties bind the span
  jointly: an auth scheme word between the colon and the value is taken WITH the
  value, so `Authorization: Bearer <value>` still renders `Authorization:
  [redacted]` the way `the_shape_rules_hold_without_a_resolved_credential`
  asserts; and text beyond the value on the same line survives. Keep `REDACTED`
  as this rule's marker - it already names the rule in place. Keep
  `split_inclusive('\n')` and the held-back line ending: this filter still runs
  over text that may be an HTTP request, where a lost `\r` is a protocol error
  rather than cosmetics.
- **Verify:** `cargo test -p verbatim-core --features testkit` passes, with new
  `#[cfg(test)]` cases in `egress.rs` showing that `scrub(None, ...)` over
  `turn_id=42 user: I set Authorization: Bearer sk-PLANTED-1 and it worked`
  returns text that contains neither `sk-PLANTED-1` nor `Bearer`, still contains
  `turn_id=42` and still contains `and it worked`; that over
  `turn_id=7 user: Cookie: sid-PLANTED-2; the rest of the turn` it returns text
  without `sid-PLANTED-2` that still contains `the rest of the turn`; and that
  `crates/verbatim-core/tests/egress.rs` passes with zero edits, in particular
  `a_pretty_printed_json_pair_keeps_its_punctuation`,
  `text_with_nothing_secret_in_it_comes_back_unchanged` and
  `the_shape_rules_hold_without_a_resolved_credential`.

### Task 2: Redact the two shapes that name themselves

- **Files:** crates/verbatim-core/src/observe/egress.rs (start at `redact` and
  `REDACTED_PRIVATE_KEY`)
- **Action:** Add two rules to the set `redact` runs, for the two shapes D-09
  measured that carry no name beside them: a bare JWT - a run beginning `eyJ`
  followed by base64url bytes (ASCII letters, digits, `-`, `_`, `.`) - and a
  GitHub token, a run beginning with one of a PINNED prefix list. Hold the
  prefixes `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` and `github_pat_` in a named
  constant array beside `SECRET_NAMES`, commented as a 2026-08-30 snapshot with
  no in-repo source of truth that will go stale; a generic `gh?_` shape was
  rejected because it matches unrelated text (D-10). Hand-written scanners over
  `as_bytes()` like every rule already in this file - no regex crate is added to
  a workspace whose `Cargo.lock` has none, for a module whose whole design is
  index arithmetic (D-11). Give each rule its own public marker constant beside
  `REDACTED_PRIVATE_KEY`, naming what went (D-13): a value that vanished into a
  bare `[redacted]` leaves a reader unable to tell which rule fired, and the
  module doc's promise that every rule leaves such a marker becomes false
  in-tree. Each marker must itself be inert against every other rule in the set
  - a marker carrying a `name: value` or `NAME=value` shape whose name matches
  `SECRET_NAMES` gets redacted again by a later rule, and no test can then
  attribute a catch to a rule. Set the length floors LOOSE, sized so this repo's
  short unrealistic sentinels match (`ghp_abc123XYZ` is the one already in
  `crates/verbatim-core/tests/provider.rs`); realistic-length values in a public
  repo risk GitHub push protection, which is why the sentinels are short (D-12).
  The measurement bounds what looseness costs: over 60 real transcripts / 21 MB,
  `ghp_`-shaped and `github_pat_` runs occurred 0 times and bare `eyJ`-prefixed
  runs 4 times. Neither rule may assume its input is a JSON request body - phase
  4 reuses this exact set over recall excerpts, brief windows and `recall_get`
  output (D-06).
- **Verify:** `cargo test -p verbatim-core --features testkit` passes, with new
  `#[cfg(test)]` cases in `egress.rs` showing that a bare `eyJ`-prefixed
  sentinel and a `ghp_`-prefixed sentinel, each planted in plain prose with NO
  name-keyed text beside them so no other rule can be what caught them, are both
  absent from `scrub(None, ...)`'s output and each leaves its own marker
  constant; that scrubbing the output a second time returns it unchanged, which
  is the markers being inert; and that
  `crates/verbatim-core/tests/egress.rs::text_with_nothing_secret_in_it_comes_back_unchanged`
  still passes unmodified.

### Task 3: Redact the two shapes their surrounding syntax names

- **Files:** crates/verbatim-core/src/observe/egress.rs (start at `redact` and
  `redact_assignments`)
- **Action:** Add the other two rules D-09 requires, for values that carry no
  name-keyed pair but are positioned by the syntax around them. First, a
  connection URL carrying userinfo - the span between a `://` and the `@` that
  closes it - replaced while the scheme and everything from the `@` onward stay,
  so a reader can still see which host was reached. The scan for that `@` must
  stop at a byte that cannot appear in userinfo (a `/`, `?`, `#`, whitespace or
  a quote), or `https://example.com/path@thing` is read as a credential.
  Measured 8 `://user@`-shaped occurrences over the same 60-file / 21 MB sample.
  Second, a space-separated flag whose name is a secret name: `redact_assignments`
  scans for `=` and therefore catches `--token=x` already (its name run admits
  `-`), but not `--token x`, which was measured 5 times in the same sample. The
  flag's value is the next whitespace-delimited run, and it stops at the same
  quote and backslash bytes `redact_assignments` already stops at, for the
  reason stated there: this filter runs over text that sits inside a JSON
  string, and running to the next space would swallow the string's closing quote
  and the comma after it. Each rule gets its own named marker constant (D-13),
  each marker inert against every other rule, hand-written scanners with no
  regex crate (D-11). Neither rule may assume its input is a JSON request body
  (D-06).
- **Verify:** `cargo test -p verbatim-core --features testkit` passes, with new
  `#[cfg(test)]` cases in `egress.rs` showing that `scrub(None, ...)` over a
  turn-shaped line carrying `postgres://user:pw-PLANTED-3@db.example.invalid/app`
  returns text without `pw-PLANTED-3` that still contains
  `db.example.invalid/app` and this rule's marker; that over a line carrying
  `gh auth login --token tok-PLANTED-4 --scopes repo` it returns text without
  `tok-PLANTED-4` that still contains `--scopes repo` and that rule's marker;
  that `https://example.com/path@thing` and `https://example.invalid/keys` come
  back unchanged; and that
  `crates/verbatim-core/tests/egress.rs::the_scrubber_takes_the_credential_out_of_a_401_body`
  passes unmodified, still finding `invalid_api_key` in its output.

### Task 4: State the widened rule set and its tolerated failure in the module docs

- **Files:** crates/verbatim-core/src/observe/egress.rs (the inner-doc module
  header at the top of the file)
- **Action:** The module header numbers five rules and says `for_destination` is
  "the request body's gate"; both are now false. Renumber the list so every rule
  the set runs is on it, each with the one-line reason it is there and the
  marker it leaves, matching the constants tasks 2 and 3 actually added. Restate
  `for_destination` as the gate for whatever text its caller hands it, not for a
  request body specifically: plan 2 moves the remote path's call to each
  message's content, and phase 4 reuses this same set over recall excerpts and
  brief windows, so a doc that assumes a JSON document is a doc phase 4 has to
  rewrite (D-06). Keep the "egress only, never ingest" section as it stands.
  State the over-matching direction as the TOLERATED failure explicitly and in
  those terms (AC7): a name-substring false positive costs a caller detail in
  one message, while under-redaction costs a key rotation - and extend that
  paragraph to cover the new value-shape rules, whose length floors are set
  against short unrealistic sentinels and therefore over-match more than a floor
  chosen against real credentials would (D-12). Say the GitHub prefix list is a
  dated snapshot. No behaviour changes in this task.
- **Verify:** `cargo test -p verbatim-core --features testkit` and
  `cargo doc -p verbatim-core --no-deps` both pass; every constant that
  `grep -n "pub const REDACTED" crates/verbatim-core/src/observe/egress.rs`
  lists appears in the module header's rule list and every rule in that list is
  one `redact` actually calls; and the header states, in words, that a
  name-substring false positive costs a caller detail in one message while
  under-redaction costs a key rotation, and that the new value-shape rules'
  floors are set against short sentinels.

## Notes

Runs BEFORE plan 2, which is not the usual reason for a split. The two plans
declare no file in common, but plan 2's acceptance test asserts that the body on
the wire contains "that rule's named marker constant" (AC2), and those constants
are created here. They are sequential, not parallel.

Plan 2 owns `crates/verbatim-core/tests/egress.rs`. Every task above runs those
tests and none may edit them; if a widened rule genuinely requires an existing
assertion to change, that is a deviation to report rather than an edit to make.
