# risk_surface - phase 3, plan 2

Range `5bb3f97..edf4add` (15 files, +3314/-12). Gate `blocking`, reviewer set
`["claude-subagent"]`, mode `adjudicated`. Matched surfaces: untrusted input
reaching FTS5 `MATCH` (D-09), project scoping and exclusion as an authorization
boundary, a read-only open beside a concurrent WAL writer, blob excerpt cutting.

**Round 1: FAIL** - 2 `high`, and 1 `medium` adjudicated UP to `high`. All three
fixed in `1066d40` and `12c22c9`; 28 suites green, clippy clean.

## Fixed

- **high** `recall/scope.rs` - `longest_prefix` scored a `project` hit and a
  `project_pre_worktree` hit identically and broke the tie by the projection's
  `ORDER BY`. One directory is routinely both (a machine that ingested from a
  worktree before phase 2's folding and again after), so a user standing in
  `/home/u/proj` resolved to `/home/u/main`: another project's turns returned,
  their own hidden. Deterministically the wrong project.
- **high** `config.rs` / `recall/context.rs` - every read named
  `project_pre_worktree` unconditionally, so a store predating the column met a
  raw `no such column` from the first statement of both `search::run` and
  `context::window`. Reachable by upgrading the binary and searching before the
  next ingest, since only `Store::open` runs `bring_forward` while reads open
  read-only (D-10). This is the exact failure `Store::missing_columns` exists to
  prevent and the degraded read D-18 asks for.
- **high (adjudicated up from medium)** `recall/context.rs` - the context window
  resolved `Scope::Everything` unconditionally and took no scope argument, so it
  enforced exclusion but not scoping. RCL-10 auto-scopes **all three** MCP
  tools, and `recall_context` is one of them, so PLAN-4 had no parameter to
  satisfy it with. Turn ids are densely enumerable (`session_no << 24 |
  turn_seq`), so a client walking ids read whole conversations out of projects
  it never stood in. Raised because it is a cross-project read path required
  closed by a phase requirement, not a nice-to-have.

## Open items (medium/low, not fixed - the blocking arm fixes blocker/high only)

1. **medium** `recall/excerpt.rs:207` - `first_token` indexes into
   `text.to_lowercase()` while `window` slices `text.chars()`; the two disagree
   for any character whose lowercase mapping changes length, so the excerpt is
   displaced and can omit the match. Reproduced with U+0130 (Turkish dotted
   capital I): `excerpt.contains("NEEDLE") == false` where the same record with
   `z` in place of it contains it.
2. **medium** `recall/search.rs:346` - the per-value document-frequency query is
   `count(DISTINCT turn_id)` over `idx_entities_lookup(kind, value_norm)`, which
   does not carry `turn_id`, so it builds a temp b-tree. Measured 15.5-16.7 ms
   on a 250k-turn store for a hot value, on the default search path, against a
   single-digit-millisecond phase 5 budget. Grows with the archive.
3. **medium** `recall/excerpt.rs:182` - excerpt cutting is unbounded in one
   record's size: two `Vec<char>` materializations and an O(tokens x chars)
   search, all to produce 240 characters, repeated per hit. Measured 13.7 ms and
   ~32 MB transient per hit on an 8 MB record with a 32-token query.
4. **low** `recall/query.rs:76` - `separator_components` splits on Rust's
   `char::is_alphanumeric`, which is broader than what `unicode61` indexes, so a
   token of characters in the gap (Devanagari U+093E, Thai U+0E31, Hebrew
   U+05B4) becomes a zero-term phrase and the conjunction silently returns zero
   hits. Exit 0, so RCL-06 holds, but with no reason attached.
5. **low** `recall/search.rs:217` - a query that reduces to no tokens returns
   `reason: None`, so RCL-10's "empty result with a reason" is unmet for the one
   empty result the query layer produces itself. `verbatim search "***"` cannot
   be told from "the archive holds nothing".

## Re-arm status

NOT re-reviewed. A `rearm` outcome for `risk_surface` is already recorded under
corr `3-80e0a9b` - spent on plan 1's fix - and `references/triage-gate.md` caps
the trigger at ONE round per run and keys the counter by trigger, not by plan.
