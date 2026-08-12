# Synthetic transcript fixtures

Real Claude Code transcripts are private and never enter this repository
(`DESIGN-BRIEF.md:500`). These files are synthetic, deterministic, and shaped
to the facts `.planning/phases/1/CONTEXT.md` measured against the real corpus.
They are the CI corpus; the private real-corpus fixture is reached by env var.

The fixtures are byte-load-bearing. Tests assert on their exact bytes, and
`session-truncated.jsonl` is a byte prefix of `session-basic.jsonl`. Editing one
by hand without re-deriving the others will fail
`crates/verbatim-core/tests/fixtures.rs`.

| File | Exists to exercise |
|---|---|
| `session-basic.jsonl` | All fifteen record types D-03 names; the D-02 out-of-order timestamp; the D-03 "both `uuid` and `timestamp`" rule; a `file-history-snapshot` with no `sessionId`; a `parentUuid` chain |
| `session-large-record.jsonl` | D-05: a turn larger than one 64 KB block |
| `session-continuation.jsonl` | D-11: file-level lineage through a foreign `session_id` |
| `session-truncated.jsonl` | D-14: a watermark that must stop at the last `\n`, not at end-of-file |
| `session-compacted.jsonl` | D-21/D-08: a session ending in a `compact_boundary` record whose `compactMetadata` bytes are stored verbatim |
| `session-recall.jsonl` | Phase 3 D-01/D-13: a `SearchManager` turn, an `src/worker/S.ts` turn, and AC3's structured-versus-prose pair - a `Read` `tool_use` naming a file against an assistant turn naming that same file in prose |
| `session-errors-a.jsonl` | Phase 3 D-03/D-04: `tool_result` blocks with `is_error` and a `toolUseResult` carrying `stdout`/`stderr`/`interrupted`, holding the a-half of both stderr pairs |
| `session-errors-b.jsonl` | Phase 3 D-04, the b-half: one stderr differing from a's only in the parts normalization strips, one differing only in a bare integer it must not |
| `subagents/agent-alpha.jsonl` | D-01: a sidecar reporting its *parent's* `sessionId` |
| `subagents/agent-echo.jsonl` | Phase 3 D-07: a sidechain turn whose message text is byte-identical to a top-level turn, so "sorts below at equal score" has two genuinely equal scores to compare |
| `subagents/workflows/wf_demo/agent-deep.jsonl` | D-16: a sidecar two directories deeper than the usual one, which only an unbounded walk reaches |
| `subagents/workflows/wf_demo/journal.jsonl` | D-12: a `.jsonl` in the tree that is not a transcript at all |
| `subagents/agent-alpha.meta.json` | D-04: the sidecar metadata file, stored as opaque bytes and never a session |

## What each one pins

**`session-basic.jsonl`** carries all fifteen `type` values: the four turn types
(`user`, `assistant`, `attachment`, `system`), each with both `uuid` and
`timestamp`, and the eleven state types (`last-prompt`, `ai-title`, `mode`,
`permission-mode`, `file-history-snapshot`, `queue-operation`,
`bridge-session`, `file-history-delta`, `agent-name`, `agent-setting`,
`frame-link`), which produce no turn, FTS or entity row. D-03 requires *both*
identity fields, so the state records split the cases deliberately: `ai-title`
carries a `uuid` and no `timestamp`, while `mode`, `permission-mode`,
`queue-operation` and `last-prompt` carry a `timestamp` and no `uuid`. A
classifier that checks either field alone passes a fixture with only one of
those shapes and fails this one.

Two adjacent `assistant` turns have decreasing `timestamp` values in file order
(records 9 and 10). D-02 measured this in 31 of 62 real transcripts: `turn_seq`
comes from byte order, never from sorting on `timestamp`. The
`file-history-snapshot` record carries no `sessionId` at all, matching all 129
real records of that type.

The token `brillig` appears in exactly one turn across the whole fixture set,
for tests that need a match with a known, unique answer. It is deliberately
*outside* the byte prefix `session-truncated.jsonl` keeps, so ingesting both
fixtures into one store still yields exactly one hit.

**`session-large-record.jsonl`** holds one turn whose text field is 204,800
bytes of deterministic filler. That record is the **first line of the file**, so
its offset in the uncompressed session stream is 0 and it occupies exactly four
64 KB blocks. Do not reorder this file: at a non-zero offset the same record
straddles a fifth block and the AC1 block-count assertions change.

**`session-continuation.jsonl`** distinguishes the two lineage signals D-11
keeps separate. `sessionId` (camelCase) is this file's own session; `session_id`
(snake_case) names `session-basic.jsonl`'s session, and that is the file-level
signal `session_meta.continues_from` is populated from. `parentUuid` stays
message-level threading and is not a substitute.

**`session-truncated.jsonl`** is `session-basic.jsonl` cut inside record ten,
with no trailing newline: nine complete records plus a partial one. It is the
"transcript still being written" case. The resume offset is the byte just past
its last `\n` (2,995), strictly less than the file length. Appending the rest of
the cut line and the remaining records must produce a store identical to
ingesting the complete file in one pass.

**`session-compacted.jsonl`** is three ordinary turns followed by the record a
compaction leaves behind: `type: "system"`, `subtype: "compact_boundary"`, with
a `uuid`, a `timestamp`, a `logicalParentUuid` and a `compactMetadata` object.
D-21 is what it pins - `system` is already a turn type and the record carries
both identity fields, so the boundary is an ordinary turn plus a derived row,
never a new record class. Its keys sit in the same order as the one real
boundary record in the corpus, and `compactMetadata` deliberately does **not**
sit last, so nothing may extract it by position.

The metadata reproduces the measured numbers: `preTokens` 45,500, `postTokens`
7,436, `cumulativeDroppedTokens` 38,064, and a `preservedMessages` whose `uuids`
list is a *proper* subset of its `allUuids` list. That subset relation is the
fact D-08 rests on: the uuid lists describe the preserved segment and cannot
enumerate the ~38k dropped tokens, which contradicts `DESIGN-BRIEF.md:140`. The
bytes are therefore stored verbatim and nothing in this phase interprets them.

The boundary is the **last** line of the file, so a test can append that one
record to an already-ingested transcript and watch a boundary row appear under a
session that had none. Adding a line after it breaks that.

**`subagents/agent-alpha.jsonl`** reports `session-basic.jsonl`'s `sessionId` on
every record with `isSidechain: true`. This is D-01's collision case: 812 real
sidecar files do exactly this, so a `sessions` table keyed on the record's
`sessionId` would overwrite the parent's blob with a small agent transcript.

**`subagents/workflows/wf_demo/agent-deep.jsonl`** is a sidecar at the deep
position. Real sidecars sit at two depths: 781 at
`<project>/<sessionId>/subagents/agent-*.jsonl` and 41 two levels further down
at `<project>/<sessionId>/subagents/workflows/wf_*/agent-*.jsonl` (D-16). A
depth-limited walk finds `agent-alpha.jsonl` and silently drops this one, and
without a fixture in the deep position no test on the synthetic corpus catches
that. Like `agent-alpha.jsonl` it reports the parent's `sessionId` with
`isSidechain: true`.

**`subagents/workflows/wf_demo/journal.jsonl`** is not a transcript. Its records
are `{agentId, key, result, type}` with none of `sessionId`, `uuid`, `timestamp`
or `cwd`. Four such files exist in the real tree, and recursive discovery with
`dot: true` reaches them, so the parser must tolerate them without erroring.

**`subagents/agent-alpha.meta.json`** is the meta file Claude Code writes
beside a sidecar transcript: same directory, same stem, a different extension.
816 real ones exist against 818 sidecars, so a missing one is normal and the
column simply stays null. It carries the five fields every real one carries -
`agentType`, `description`, `toolUseId`, `spawnDepth`, `model` - and its
`toolUseId` names the `tool_use` block in `session-basic.jsonl` that spawned the
agent, so the fixture set stays internally coherent.

Ingest stores its **bytes**, unparsed (D-04): the format is undocumented and may
drift, and keeping the bytes lets phase 3 extract `description` for ranking
without a reingest. It is not a transcript, so it is not in
`TRANSCRIPT_FIXTURES` and the JSONL assertions below do not apply to it -
discovery's filename filter excludes it on the extension, which is what keeps it
from ever becoming a session.

**`session-recall.jsonl`** is the phase 3 index fixture. It carries a turn whose
message text contains `SearchManager` (found by a search for `SearchManager` and
by a search for `manager`, which is the only expansion rule that changes recall),
a turn whose text contains `src/worker/S.ts` (found by a search for `worker`),
and AC3's structured-versus-prose pair: a `Read` `tool_use` whose `file_path` is
`docs/RETRY.md`, and a *separate* assistant turn naming `docs/RETRY.md` in prose
and nothing else. The first emits a `path` entity and a `paths` row; the second
emits neither, which is what "entities come from structured tool records, never
from prose" has to mean. A `Grep` `tool_use` carries the `retryBudget` pattern
its `symbol` extraction reads. Its last turn's message text is byte-identical to
`subagents/agent-echo.jsonl`'s.

**`session-errors-a.jsonl`** and **`session-errors-b.jsonl`** are two distinct
sessions carrying the two stderr pairs AC2 is about. Their `tool_result` blocks
carry `is_error` and their records carry a top-level `toolUseResult` with
`stdout`, `stderr` and `interrupted` - the key set measured on 2,920 real Bash
results, and the only place an error signal exists, since the transcript has no
exit-code field anywhere.

The first pair (`is_error: true`) differs between the two files *only* in a
`:line:col` suffix, a `0x` address, an ISO-8601 timestamp and a UUID, so
normalization collapses it to one value. The second pair (`is_error: false`,
non-empty `stderr`, which is D-03's other arm) differs *only* in a bare integer
that is not a line number, and must stay two values: stripping bare integers was
measured to merge genuinely different errors (46 recurring values before, 39
after). `session-errors-a.jsonl` ends with a third result that is `interrupted`
with an empty `stderr` and no error flag - the control for "an interruption is
not an error".

**`subagents/agent-echo.jsonl`** is a second sidecar reporting
`session-recall.jsonl`'s `sessionId` with `isSidechain: true`, and it has no
`agent-*.meta.json` beside it, which is the normal case for 2 of every 818 real
sidecars. Its second turn's message text is byte-identical to the last turn of
`session-recall.jsonl`, so the two project to the same FTS body and score
identically under BM25. That is what makes "a sidechain turn ranks below a
top-level turn of equal score" a statement a test can hold to, rather than one
that passes because the scores happened to differ.

## The rooted `cwd`

The phase 1 and 2 fixtures hardcode `"cwd": "/data/code/verbatim"`. The four
phase 3 fixtures carry `{{ROOT}}` instead, and `testkit::copy_rooted_fixture_into`
rewrites it to a root the test owns, creating the project directory it names.
Two projects exist across the four - `project-alpha` for `session-recall.jsonl`
and its sidecar, `project-beta` for the two error sessions - so a project-scoped
search has something to be both true and false about, on any checkout and
without a project key that depends on where this repository happens to sit.

Every transcript fixture except `session-truncated.jsonl` ends with a trailing
`\n`, and no fixture contains a `\r` byte (D-14).
