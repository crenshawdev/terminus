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
| `subagents/agent-alpha.jsonl` | D-01: a sidecar reporting its *parent's* `sessionId` |
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

Every transcript fixture except `session-truncated.jsonl` ends with a trailing
`\n`, and no fixture contains a `\r` byte (D-14).
