# risk_surface - phase 3, plan 3

Range `7b45330..b68355c` (19 files, +3752/-89). Gate `blocking`, reviewer set
`["claude-subagent"]`, mode `adjudicated`. Matched surface: public API / wire
contracts - `docs/json-shapes.md`, the `{command, ok, reason, data}` envelope
over six commands, and the 0/1/2 exit-code vocabulary.

**Round 1: FAIL** - 2 `high`, both the same defect in two copies. Fixed in
`4e48c97`; 29 suites green, clippy clean.

Not re-reviewed, on the same footing as plan 2: the `rearm` counter for
`risk_surface` is keyed by trigger and run and was spent on plan 1's fix, and
the user chose to proceed rather than spend a confirming round.

## Fixed

- **high** `cmd/search.rs`, `cmd/sessions.rs` - `day` guarded on `ts.len() >=
  10` and sliced `&ts[..10]`. `len` is bytes and the slice needs a char
  boundary, so an archived timestamp with a multi-byte character across byte 10
  panicked both commands: exit 101, outside the documented vocabulary, with
  panic text where a `verbatim: ` diagnostic belongs. `sessions` panicked
  mid-loop, leaving stdout syntactically valid and silently truncated. The
  input is ordinary: `parse::record` stores `timestamp` as whatever JSON string
  the transcript carried and validates neither shape nor encoding.

## Open items (medium/low, not fixed - the blocking arm fixes blocker/high only)

1. **medium** `cmd/json.rs:121` - `Document::emit` writes through `println!`,
   which panics with exit 101 on a closed stdout pipe. `show.rs` has a
   `broken_pipe` helper for the human path and nothing covers the JSON path, so
   `verbatim show --json 1 2 3 | head -c 100` panics where the doc says a pipe
   to `jq` works.
2. **medium** `cmd/mod.rs:132` - `time_bound` validates digit and punctuation
   shape but never the calendar, so `--since 2026-08-32` is accepted, sorts
   above every real August timestamp, and silently hides the month with exit 0
   and no reason. `2026-13-01` and `9999-99-99T99:99:99.999Z` likewise. Its own
   doc comment says it exists to prevent exactly this.
3. **medium** `recall/get.rs:247` - `rows_for` binds one SQL placeholder per
   requested id with no server-side cap, so more ids than
   SQLITE_MAX_VARIABLE_NUMBER (32,767) is an exit-1 operational failure that
   writes no `--json` document and prints the internal SQL to stderr, where a
   command-line mistake should be exit 2. PLAN-4's `recall_get` hits the same
   wall, since the cap does not exist library-side either - plan 3's own report
   already flagged the missing cap as belonging there.
4. **low** `cmd/show.rs:166` - a closed stdout in human mode maps to
   `Failure::Silent`, which is exit 1: the operational-failure code the comment
   two lines above says it must not look like. Under `pipefail`, "you stopped
   reading" is indistinguishable from "the archive failed".
5. **low** `recall/get.rs:151` - the exclusion arm returns
   `Reason::ProjectExcluded { project }`, naming the excluded project's
   absolute path, while the same function's doc says an out-of-scope id must be
   `NoSuchTurn` precisely so no reason confirms another project holds it. Ids
   are densely enumerable, so sweeping them under `--project '*'` maps which id
   ranges exist in each excluded project and what those projects are called.
6. **low** `docs/json-shapes.md:112` - the `sessions` shape documents
   nullability nowhere while `search` and `show` document theirs; five of its
   eleven fields are null in ordinary states (no `gitBranch`, no `session_meta`
   row, no `cwd`), so a consumer typed from the document breaks on real data.
