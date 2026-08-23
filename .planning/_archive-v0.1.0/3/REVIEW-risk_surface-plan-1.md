# risk_surface - phase 3, plan 1

Range `80e0a9b..c27e15e` (19 files, +2482/-48). Gate `blocking`, reviewer set
`["claude-subagent"]`, mode `adjudicated`. Matched surfaces: DB schema/migration
(`DERIVED_SCHEMA` 2 -> 3, two new tables), untrusted-input parsing (the whole
projection/entity/normalizer path), bulk rebuild.

**Round 1: FAIL** - two `high` findings, both confirmed against the real corpus
by independent measurement before acting on them.

**Round 2 (the one permitted narrowed re-arm): PASS** - both blockers confirmed
closed, nothing `blocker`/`high` introduced.

## Fixed

- **`toolUseResult.file.filePath` was never read** (f7db432). `Read` nests its
  result path one level down; only the top-level `filePath` was read, so the
  result-side turn of every `Read` emitted no `path` entity and no `paths` row.
  Measured over a 400-file sample: 8,719 `toolUseResult` objects, nested key
  1,238 against top-level 1,171. D-02's evidence line counted the top level
  only, which is what the code followed.
- **`path_words` stored shell punctuation as part of the key** (5bb3f97).
  `cd /a/b; make` wrote `/a/b;`; `cargo test 2>/dev/null` wrote `2>/dev/null`.
  Measured 9,816 of 21,061 path-shaped words (46.6%) carrying a shell control
  character - the common case, not the tail.

## Open items (medium/low, not fixed - the blocking arm fixes blocker/high only)

1. **medium** `index/text.rs:98` - `MAX_BODY_BYTES` does not actually bound the
   projected body: the `'\n'` separator is not charged against `remaining`, and
   `take` is 0 whenever the remaining budget is smaller than the next leaf's
   first character, so the `remaining == 0` short-circuit never fires and the
   row grows one byte per leaf. All-ASCII case doubles the bound; a
   1-byte-then-multibyte leaf sequence is unbounded. Real corpus max record is
   205 KB, so no live exposure.
2. **medium** `index/entity.rs` - splitting on `;&|<>()` cuts a real path that
   contains one of those seven characters into fragments
   (`Screenshot(1).png` -> `Screenshot`). 0 of 6,169 structured path values in
   a 300-file sample carry any of them, and a structured key is unaffected.
3. **medium** `index/entity.rs` - because `\` counts as a separator, splitting
   on `|` turns one garbage entity from a BRE alternation into several
   (`foo\|bar\|baz` -> `foo\`, `bar\`). Backslash-only fragments rose 2,619 ->
   3,316 (+27%), 24.7% of unique Bash-derived path entities. Nothing real is
   evicted by the cap; the table is a quarter escape debris.
4. **medium** `index/entity.rs` - `MAX_ERROR_BYTES` bounds only the `error`
   kind. `path`, `command` and `symbol` are written at whatever length the
   record supplies. Real corpus max 521 bytes.
5. **medium** `index/entity.rs:392` - when `timestamp_at` rejects a timestamp,
   `line_col_at` still fires and normalizes only the `:MM:SS` tail, so
   `[09:14:22] refused` and `[10:02:11] refused` become two error values. Zero
   live exposure: 0 of 778 real stderr strings in the sample carry any
   timestamp. Complements the `host:8080` open item already recorded, which
   names the over-matching direction only.
6. **medium** `index/entity.rs:470` - `program_of` returns the first argv word,
   so a compound command records the wrapper. 81.7% of Bash calls contain
   `&&`/`||`/`;`/`|`; `cd` is the recorded program for 17.8% of them. Also left
   on whitespace-only splitting, so it still carries the punctuation the
   `path_words` fix stripped (39 of 4,829 command entities, top value `&&`).
7. **low** `index/entity.rs:521` - `identifier_tokens` splits on
   non-alphanumeric, so a regex escape's letter is glued on: `\bsearch_manager`
   -> symbol `bsearch_manager`. 5 of 155 real Grep patterns.
8. **low** - the `://` and `$` drops also discard `file://` URLs and Windows UNC
   admin shares (`\\server\c$\share\x`), both of which do name one file. Zero
   occurrences in the sample.
9. **low** - both fixes change what the derivation emits without moving
   `DERIVED_SCHEMA`, so a store built by a mid-phase binary keeps the pre-fix
   rows with no signal. Dev-only exposure today, and PLAN-2 forbids a second
   bump inside the phase.
