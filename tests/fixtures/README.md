# Synthetic transcript fixtures

Real Claude Code transcripts are private and never enter this repository
(`DESIGN-BRIEF.md:500`). These files are synthetic, deterministic, and shaped
to the facts `.planning/phases/1/CONTEXT.md` measured against the real corpus.
They are the CI corpus; the private real-corpus fixture is reached by env var.

The fixtures are byte-load-bearing. Tests assert on their exact bytes, and
`session-truncated.jsonl` is a byte prefix of `session-basic.jsonl`. Editing one
by hand without re-deriving the others will fail
`crates/terminus-core/tests/fixtures.rs`.

| File | Exists to exercise |
|---|---|
| `session-basic.jsonl` | All fifteen record types D-03 names; the D-02 out-of-order timestamp; the D-03 "both `uuid` and `timestamp`" rule; a `file-history-snapshot` with no `sessionId`; a `parentUuid` chain |
| `session-large-record.jsonl` | D-05: a turn larger than one 64 KB block |
| `session-continuation.jsonl` | D-11: file-level lineage through a foreign `session_id` |
| `session-truncated.jsonl` | D-14: a watermark that must stop at the last `\n`, not at end-of-file |
| `session-compacted.jsonl` | D-21/D-08: a session ending in a `compact_boundary` record whose `compactMetadata` bytes are stored verbatim |
| `session-recall.jsonl` | Phase 3 D-01/D-13: a `SearchManager` turn, an `src/worker/S.ts` turn, and AC3's structured-versus-prose pair - a `Read` `tool_use` naming a file against an assistant turn naming that same file in prose |
| `session-edits.jsonl` | Phase 5 D-05: an `Edit` `tool_use` storing an **absolute** path beneath the fixture root, the spelling the real corpus almost always uses, with symbols beside it so one turn carries two independent entities - and a prose turn naming the same file structurally invisibly |
| `session-envelope.jsonl` | v0.1.1 phase 1 D-02/D-13/D-15: a session whose **last** `user` record is a `<task-notification>` envelope and whose second-to-last is a `tool_result`, so the turn the resume brief quotes moves under INJ-07's rule |
| `session-secrets.jsonl` | v0.1.1 phase 2 D-15: one turn for each of the seven credential shapes the egress filter must catch on the remote provider path - an `Authorization: Bearer` header, a JSON `"password"` pair, a space-separated `--token` flag, a mid-line `Cookie` with prose after it, a bare JWT, a GitHub token and a connection URL carrying userinfo |
| `session-errors-a.jsonl` | Phase 3 D-03/D-04: `tool_result` blocks with `is_error` and a `toolUseResult` carrying `stdout`/`stderr`/`interrupted`, holding the a-half of both stderr pairs |
| `session-errors-b.jsonl` | Phase 3 D-04, the b-half: one stderr differing from a's only in the parts normalization strips, one differing only in a bare integer it must not |
| `subagents/agent-alpha.jsonl` | D-01: a sidecar reporting its *parent's* `sessionId` |
| `subagents/agent-echo.jsonl` | Phase 3 D-07: a sidechain turn whose message text is byte-identical to a top-level turn, so "sorts below at equal score" has two genuinely equal scores to compare |
| `subagents/workflows/wf_demo/agent-deep.jsonl` | D-16: a sidecar two directories deeper than the usual one, which only an unbounded walk reaches |
| `subagents/workflows/wf_demo/journal.jsonl` | D-12: a `.jsonl` in the tree that is not a transcript at all |
| `subagents/agent-alpha.meta.json` | D-04: the sidecar metadata file, stored as opaque bytes and never a session |

## `hooks/`: the payloads, not the transcripts

`hooks/session-start.json`, `hooks/user-prompt-submit.json`,
`hooks/session-end.json` and `hooks/post-compact.json` are a different kind of
fixture. They are not transcripts and they are not ingested: each is the single
line of JSON Claude Code writes to a hook's stdin before closing it, and
`crates/terminus/tests/hook.rs` feeds them to `terminus hook <event>` to hold
AC1's "exit 0, nothing on stdout, p99 under 10 ms".

Their fields are read off the 2.1.231 payload schemas rather than guessed. Every
payload carries the base object the bundle's builder returns - `session_id`,
`transcript_path`, `cwd`, `prompt_id`, `permission_mode`, `agent_type` - then
`hook_event_name`, then that event's own fields: `source`, `model` and
`session_title` for `SessionStart`, `prompt` and `session_title` for
`UserPromptSubmit`, `reason` for `SessionEnd`, `trigger` and `compact_summary`
for `PostCompact`. The enumerated values are the schema's own: `source`
`"resume"`, `reason` `"prompt_input_exit"`, `trigger` `"auto"`.

Nothing in phase 4 parses a single one of those fields (D-15) - the hook reads
the line and drops it. They are shaped correctly anyway because phase 5 reads
them, and a fixture invented then would be a fixture invented against the code
that consumes it.

`hooks/session-start-compact.json` is the fifth, and the only one of the five
that is a *recording* rather than a reading of the schema. Phase 5 D-08 builds
INJ-05's trigger on a `SessionStart` whose `source` is `"compact"`, and until
2026-08-20 nothing here had seen one: the enum was verified present in bundle
2.1.237 but the emit site was never located. It was then captured live - a
temporary matcher-less `SessionStart` entry appending its stdin to a file, a
fresh session, then `/compact` - and **a compaction does fire a `SessionStart`,
with `hook_event_name` `SessionStart` and `source` `"compact"`**, after the
compaction completes and under the same `session_id` as the `"startup"` line
that opened the session. So this is the affirmative case, not D-08's fallback.

The capture is those bytes with the identifying fields replaced, because this
repository is public and a real transcript never enters it: `session_id`,
`transcript_path` and `cwd` carry the same synthetic values as the other four,
`prompt_id` is the same synthetic UUID they use, and `model` is theirs rather
than the build-specific name the live session reported. `hook_event_name` and
`source` are exactly as captured, and they are the two fields anything reads.

Two things that capture contradicts about the paragraph above it, left standing
because nothing yet reads the fields involved. The live `"compact"` line carries
`prompt_id` and the live `"startup"` line does not, so `prompt_id` is not an
unconditional base field; and neither live line carried `permission_mode`,
`agent_type` or `session_title` at all. A phase that starts reading one of those
three should re-observe it rather than trust the four older fixtures.

Each is one line with a trailing newline, matching `stdin.write(payload + "\n")`.

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

**`session-edits.jsonl`** is the phase 5 injection fixture, and what it holds
that nothing else does is an **absolute** path. D-05 measured 1,029 absolute
against 2 relative `file_path` values over 120 sampled real transcripts, so an
archived `path` entity is almost always absolute while the user types a relative
one - and `Query::matches_entity` needs every token of the stored value present
in the query, so the two cannot meet without resolving the relative spelling
against the payload's `cwd`. A fixture storing a relative path would let a test
of that resolution pass without the resolution.

Its `Edit` call names `{{ROOT}}/project-alpha/crates/gizmo/lantern.rs` and
carries `lanternFlicker` and `lanternSteady` across its two edit sides, so one
turn emits four distinct entities - a tool, a path and two symbols - which is
what INJ-03's "two or more independent entities co-occurring" needs to have
something to fire on. Its third turn names that same absolute path in **prose**
and emits no entity at all, so "matched structurally" and "mentions the words"
stay two different answers about one file, the way `session-recall.jsonl`
already does for the relative case.

**`session-secrets.jsonl`** is the v0.1.1 phase 2 egress fixture: ten turns
that exist to be *sent*, not to be found. Each of the seven shapes the widened
egress rule set is asked to catch sits in a turn of its own, so a wire test can
assert on one sentinel at a time and attribute the catch to one rule.

Four of the seven - the bare JWT, the GitHub token, the connection URL's
userinfo and the space-separated `--token` value - sit with **no name-keyed text
beside them**. That is the load-bearing part: written `GITHUB_TOKEN=ghp_...`
instead, the assignment rule would catch it and the assertion would be about
nothing. The `Cookie` turn puts its value **mid-line** with ordinary prose after
it on the same line, which is what makes "the rest of the turn survived the
match" a thing a test can read.

Its sentinels are short, unrealistic and distinct from one another
(`ghp_VBEGRESSgh0zq`, `sk-VBEGRESS-authz-9f2`), following this repository's
existing convention: a realistic-length credential in a public repository risks
GitHub push protection, and a shared sentinel would let one rule's catch pass as
another's.

Like `session-envelope.jsonl` it is deliberately inert. Its project key is a
**fourth** one, `project-delta`, which no assertion anywhere scopes against; it
carries no `tool_use` and therefore emits no entity; and its vocabulary shares
none of the corpus's counted tokens - no `brillig`, no `SearchManager`, no
`cargo`, none of the paths the entity tests match on.

**`session-envelope.jsonl`** is the v0.1.1 phase 1 brief fixture: five records
whose last `user` record is not one a person typed. In order - a `user` text
turn (`wire up the quince exporter and tell me what it prints`, the prompt the
brief must end up quoting), an `assistant` turn carrying a `Glob` `tool_use`, a
`user` turn whose `message.content` is a `tool_result` block and whose top-level
`toolUseResult` carries a `stdout` string and nothing else, an `assistant` text
turn, and last a `user` text turn opening with `<task-notification>`.

That covers D-02's two not-typed shapes at once, and it is the *last* record
being one of them that matters: a brief reading "the last `user` turn" quotes
the envelope, and a brief reading "the last typed `user` turn" quotes record
one. `session-recall.jsonl` cannot observe that difference - its only `user`
record is a plain text block - which is why `crates/terminus/tests/brief.rs`'s
byte-identity test seeds from this file instead (D-15).

Three things about it are deliberate. Its project key is a **third** one,
`project-gamma`: every project-scoped assertion elsewhere in the repository is
written against `project-alpha` or `project-beta`, so a new member of neither
moves none of them. Its tool is `Glob`, which appears in no other fixture and
whose `pattern` input emits no entity beyond the tool itself, so no entity count
anywhere moves either. And its vocabulary shares nothing with the corpus's
counted tokens - no `brillig`, no `SearchManager`, no `cargo`, none of the
paths the entity tests match on.

## The rooted `cwd`

The phase 1 and 2 fixtures hardcode `"cwd": "/data/code/verbatim"`. The seven
rooted fixtures carry `{{ROOT}}` instead, and
`testkit::copy_rooted_fixture_into` rewrites it to a root the test owns,
creating the project directory it names. Four projects exist across them -
`project-alpha` for `session-recall.jsonl`, its sidecar and `session-edits.jsonl`,
`project-beta` for the two error sessions, `project-gamma` for
`session-envelope.jsonl` alone and `project-delta` for `session-secrets.jsonl`
alone - so a project-scoped search has
something to be both true and false about, on any checkout and without a project
key that depends on where this repository happens to sit. `session-edits.jsonl`
needs the root for a second reason: the absolute path it stores has to be
beneath the same root its own `cwd` names, or no prompt resolved against that
`cwd` could reach it.

Every transcript fixture except `session-truncated.jsonl` ends with a trailing
`\n`, and no fixture contains a `\r` byte (D-14).
