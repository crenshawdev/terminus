# Roadmap: Verbatim

## Overview

The archive is the product, so it is built first and everything else is built on top of it: a store that can hold a session losslessly and prove it, then an ingest that survives a real transcript tree, then a query path that finds a turn from the terminal and from the model. Only once recall works is the thing wired into Claude Code, because hooks and install are worth nothing until there is something to recall. Injection follows, then the feedback loop that measures whether injection helped — deliberately after injection exists, since it labels injection's own decisions. Observations and their egress boundary come next as the one opt-in, one network-touching feature, and lifecycle management lands last because retention is meaningless until there is data worth aging out. Each phase leaves something exercisable: a store you can verify, an ingest you can run, a search you can type, hooks that fire, a brief that appears, stats that prove precision.

## Phases

- [ ] **Phase 1: Prompt-True Resume Brief** - the brief's quoted prompt is a turn the user typed, not the tool result the harness wrote as a `user` record

## Phase Details

### Phase 1: Prompt-True Resume Brief
**Goal:** `SessionStart`'s "It last asked" quotes a turn the user actually typed. Claude Code writes tool results as `type: "user"` records and `record_type` passes that field through unchanged, so any session that ended inside a tool loop has its harness payload quoted back under a label that attributes the words to the user. Nothing stored can currently tell the two apart: `tool_name` is written only on the assistant side, null on all 119,649 `user` rows.
**Depends on:** Nothing (first phase of this cycle)
**Requirements:** INJ-07
**Success Criteria:**
1. Every `user` row in a freshly ingested store is classified as typed or tool-result with no third state, and the classification is a stored column read by one `AND` on the query `brief.rs` already runs - no additional blob is opened for it.
2. A session whose last `user` record is a tool result renders "It last asked:" quoting its last typed prompt; a session with no typed prompt at all renders no prompt line rather than falling back to the tool result.
3. `SessionStart` wall time stays inside the p99 budget `crates/verbatim/tests/hook.rs` already asserts over 100 runs of every event, measured on the real corpus rather than a fixture.
4. A store written by the previous build acquires the column and is backfilled from blobs alone, never by re-reading transcripts, and `verbatim verify` passes over it afterwards.
5. Two renders against an unchanged store are byte-identical (INJ-02), including for a session whose quoted turn changed under the new rule.
6. The turn ids recorded in the session state file are the ids of the turns actually quoted under the new rule, so INJ-04's suppression still covers exactly what the brief put on screen.
