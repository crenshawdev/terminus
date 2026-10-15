---
phase: 12
plan: 2
requirements: ["#20", "#17", "#39"]
files:
  - crates/terminus-core/src/index/text.rs
  - crates/terminus-core/src/derive.rs
  - crates/terminus-core/src/parse/record.rs
  - crates/terminus-core/tests/compaction.rs
  - crates/terminus-core/tests/parse.rs
  - crates/terminus-core/src/feedback/label.rs
  - crates/terminus-core/tests/label.rs
---

# Phase 12: Quick Issue Fixes - Plan 2

## Goal

The phase goal: twelve open GitHub issues that are small enough to fix inside one function or one doc section are fixed and closed before the release is announced, which cuts the open list from 39 to 27. #17 gets its type check here, and its second case moves to phase 13.

This plan fixes the three indexing issues. `Projector::push` charges its separator, so the indexed text stays inside `MAX_BODY_BYTES` (#20). A compaction boundary needs `type` `system` (#17, the type-check half only). The miss-labelling query drops its `recall_get` arm and matches `recall_search` by the extractor's rule (#39).

## Must be true when done

- `index::text::project` never returns more than `MAX_BODY_BYTES` bytes, including when the pieces reach the cap only once separators are counted. Its result never ends on a bare `'\n'`, and nothing is appended after a piece has been cut short.
- A `user`, `assistant` or `attachment` record carrying `subtype: compact_boundary` gets no `compaction_boundaries` row, while the real `system` boundary still gets its row. `Record::is_compact_boundary` gives the same answer as the rule `derive_turn` writes rows by.
- The miss-labelling query matches a turn whose `tool_name` is `recall_search` or `mcp__terminus__recall_search`. It matches none of `recall_get`, `xrecall_search` and `MCP__TERMINUS__RECALL_SEARCH`. This holds for ingest's `miss()` and for replay's `sought()` alike.
- `DERIVED_SCHEMA` is still 4. No stored row is rewritten by this plan, and rows written before it are corrected only by the next `terminus reindex`.
- `cargo test -p terminus-core --all-features` and `cargo test -p terminus --all-features --test replay` pass.

## Context

- D-05, D-06 and D-07 in `.planning/phases/12/CONTEXT.md` bind this plan exactly. #17's second case, a real boundary with no `uuid` or `timestamp`, is deferred to phase 13 and is not touched here. rusqlite's `functions` feature stays off, and existing labels are not rewritten.
- `MAX_BODY_BYTES` bounds the projection `index::text::project` returns. The expansion tokens `index::project` appends after it have their own budget, `MAX_EXPANSION_BYTES`, and are not part of #20.
- Every task leaves `cargo clippy --workspace --all-features --all-targets -- -D warnings` and `cargo fmt --all --check` clean. Both are clean on the tree today.

## Tasks

### Task 1: Charge the separator to the body budget (#20)

- **Files:** crates/terminus-core/src/index/text.rs (`Projector::push` and a new tests module)
- **Action:** Make `Projector::push` count each `'\n'` separator against `remaining`. Write a separator only when at least one whole character of the next piece fits after it, so a body never ends on a bare newline. Once a piece has been cut short, either because the budget ran out partway through it or because not even its first character fit after the separator, nothing more is appended for the rest of the record. The state that marks a cut must also stop `leaves`, which today stops early only on `remaining == 0`, so a long tail of later leaves is not walked for nothing. Keep slicing on `floor_char_boundary`, because a byte-indexed cut through a multi-byte character panics and takes the session's ingest with it. Update `push`'s doc comment to say the separator is charged and why: an uncharged separator ran a full body past the cap by one byte per piece, and stalled once `remaining` fell below the next piece's first character. Do not bump `DERIVED_SCHEMA` (D-05). `recall/excerpt.rs` calls `text::project` too, so it follows the change with no edit of its own. Add a `#[cfg(test)]` module to `text.rs`. Its tests can build a `Projector` with a small budget directly, and they cover four cases. First, the issue's case: a 10-byte budget and the pieces `aaaa`, `bbbb` and `cccc` produce at most 10 bytes, not ending on `'\n'`. Second, the stall case: once the budget left is smaller than a separator plus the next piece's first character, that piece and every later piece add nothing, including a later piece short enough to have fit. Third, through `project`, a record whose string leaves fill `MAX_BODY_BYTES` only once separators are counted yields at most `MAX_BODY_BYTES` bytes. Fourth, a piece cut partway through leaves budget behind and still ends the record: with a 7-byte budget, the pieces `a😀😀` and `x` produce exactly `a😀`. `floor_char_boundary` cuts the first piece at byte 5, which leaves 2 bytes, enough for a separator and `x`, so this test fails when only a stall, and not a partial cut, stops later pieces.
- **Verify:** `cargo test -p terminus-core --all-features --lib index::text` passes, and each of the four tests above fails against today's `push`, with the separator written uncharged. Restore the fix after that check and never commit the reverted form. `cargo test -p terminus-core --all-features --test index --test recall --test recall_golden` passes. Clippy and fmt are clean.

### Task 2: Require type system for a compaction boundary (#17, type check only)

- **Files:** crates/terminus-core/src/derive.rs (`TurnRow::is_compact_boundary`), crates/terminus-core/src/parse/record.rs (`Record::is_compact_boundary` and its doc comment), crates/terminus-core/tests/compaction.rs, crates/terminus-core/tests/parse.rs
- **Action:** `TurnRow::is_compact_boundary` returns true only when the `Turn` the row already carries has `record_type` `system` and the row's subtype is `COMPACT_BOUNDARY`. Read the type off `row.turn`, never by re-parsing `row.record`, because the seam's own comments forbid a second parse site. Give `Record::is_compact_boundary` the same rule against `Record::record_type`, so the two cannot disagree (D-06). Its doc comment currently says it only names the subtype for "the one caller that writes a derived boundary row", and only tests call it. Correct that, and say the type is checked because `subtype` is read off any record that carries one. Do not change classification (`TURN_TYPES`, D-03), do not handle a boundary that lacks `uuid` or `timestamp` (deferred to phase 13), and do not bump `DERIVED_SCHEMA`. In `tests/compaction.rs`, add a test that appends records which are the real boundary line from `testkit::boundary_line()` with `type` changed to `user`, `assistant` and `attachment` in turn, each with a fresh `uuid`, then ingests. None of them may get a `compaction_boundaries` row, while each is still archived as a turn. In `tests/parse.rs`, assert that `Record::is_compact_boundary` is false for those same three lines and still true for the unmodified boundary line.
- **Verify:** `cargo test -p terminus-core --all-features --test compaction --test parse --test inject_compaction --test derive` passes, including the new tests and the unchanged `appending_a_boundary_records_it_and_disturbs_nothing_already_archived` and `a_rebuild_reproduces_the_boundary_row_from_the_blob_alone`. The new compaction test fails when the type check in `TurnRow::is_compact_boundary` is temporarily removed. Restore it afterwards. `grep -n 'DERIVED_SCHEMA: i64 = 4' crates/terminus-core/src/store/open.rs` still matches. Clippy and fmt are clean.

### Task 3: Match recall_search in the miss query by the extractor's rule (#39)

- **Files:** crates/terminus-core/src/feedback/label.rs (`sought_sql`), crates/terminus-core/tests/label.rs
- **Action:** In `sought_sql`, delete the `recall_get` arm. Nothing extracts entities from a `recall_get` call's input, because `index::entity` special-cases only `recall_search`. Replace the `LIKE '%recall\_search'` arm with the rule `index::entity::is_recall_search` states: the name is exactly `recall_search`, or it ends in `_recall_search`, compared case-sensitively. SQLite's `LIKE` is case-insensitive for ASCII and treats a bare `_` as a wildcard, which is how the current arm accepts `MCP__TERMINUS__RECALL_SEARCH`. A case-sensitive comparison such as `GLOB`, where `_` is literal, or an exact `substr` comparison, expresses the rule in SQL. Do not register a Rust function with SQLite, because rusqlite's `functions` feature stays off (D-07). Name `is_recall_search` in a comment beside the SQL, so the next reader knows the two must agree. Both `miss()` and `sought()` build on `sought_sql`, so this one change reaches both. Do not rewrite existing labels. In `tests/label.rs`, add a test that isolates the SQL from the extractor. For each of the five names `recall_search`, `mcp__terminus__recall_search`, `recall_get`, `xrecall_search` and `MCP__TERMINUS__RECALL_SEARCH`, set up a turn with that `tool_name` after a decision (the bench's `decide` helper writes one), plant an `entities` row on it holding a spelling the decision named and withheld, and call `terminus_core::feedback::label::sought`. Planting is needed because the extractor writes no entity for the three non-matching names, so without it the test could not tell the old SQL from the new. It must find the spelling for the first two names and return `None` for the other three.
- **Verify:** `cargo test -p terminus-core --all-features --test label --test feedback` passes, including the new test and the unchanged `a_recall_call_for_something_the_injector_declined_is_a_miss`. The new test fails against the old two `LIKE` arms, which you can check by restoring them temporarily and then putting the fix back. `cargo test -p terminus --all-features --test replay` passes. Clippy and fmt are clean.
