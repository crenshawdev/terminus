# Roadmap: Verbatim

## Overview

The archive is the product, so it is built first and everything else is built on top of it: a store that can hold a session losslessly and prove it, then an ingest that survives a real transcript tree, then a query path that finds a turn from the terminal and from the model. Only once recall works is the thing wired into Claude Code, because hooks and install are worth nothing until there is something to recall. Injection follows, then the feedback loop that measures whether injection helped — deliberately after injection exists, since it labels injection's own decisions. Observations and their egress boundary come next as the one opt-in, one network-touching feature, and lifecycle management lands last because retention is meaningless until there is data worth aging out. Each phase leaves something exercisable: a store you can verify, an ingest you can run, a search you can type, hooks that fire, a brief that appears, stats that prove precision.

## Phases

- [x] **Phase 1: Prompt-True Resume Brief** - the brief's quoted prompt is a turn the user typed, not the tool result the harness wrote as a `user` record
- [x] **Phase 2: Egress Filter Sees What Is Sent** - the redaction rules run over transcript content before it is serialized, so the shapes they name are actually caught on the wire
- [x] **Phase 3: Owner-Only On Disk** - nothing verbatim writes lands group- or world-readable, and `verbatim.toml` gets the mode check the shared credentials file already has
- [ ] **Phase 4: Redacted Recall** - an opt-in knob filters the derived recall projections, because hook and MCP output is model context and therefore an egress boundary

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

### Phase 2: Egress Filter Sees What Is Sent
**Goal:** The five rules in `crates/verbatim-core/src/observe/egress.rs` run over transcript text on the remote provider path. Today they do not. `crates/verbatim-core/src/observe/provider.rs:254` calls `egress::for_destination` on the ALREADY-SERIALIZED JSON body, which makes rules 3 and 4 inert: `redact_header_lines` splits on a real `'\n'` and a serialized body has none, so the whole body is one line whose first colon sits inside `{"model":` and `header_name` rejects it; `redact_json_pairs` walks the outer document, and `quoted_end` skips every `\"`, so the entire transcript is consumed as a single candidate name, fails the `:` test, and is never re-scanned. Only rule 1 (the exact configured credential) and rule 5 (`NAME=value`, which needs no quotes to survive escaping) fire. The existing tests pass because they feed the filter unescaped sample text rather than the bytes that go on the wire.
**Depends on:** Nothing
**Requirements:** PRIV-01, PRIV-03
**Success Criteria:**
1. The egress tests assert over the exact bytes `complete` hands to `net::post`, obtained from the same construction the real path uses rather than a hand-written sample. A rule that passes on unescaped text and fails on the serialized body is a failing test before the fix and a passing one after.
2. A secret planted in a transcript turn is absent from the serialized remote body in each of these shapes: `Authorization: Bearer`, a JSON `"password"` pair, a space-separated `--token` flag, a `Cookie` header, a bare JWT, a GitHub token, and a connection URL carrying userinfo.
3. Redaction runs per message content BEFORE `request.to_string()`, so the body still parses as JSON with the same message count, the same roles and the same key order as an unredacted build produces.
4. `local = true` still returns the body byte-identical and still borrows rather than copies - the existing D-13 test is unchanged and still passes.
5. The over-matching direction is preserved deliberately: a name-substring false positive costs detail in one message, and that is stated in the module docs as the tolerated failure.
6. `nothing_in_the_ingest_path_names_the_egress_filter` still holds. Widening the detector must not put it anywhere near ingest.

### Phase 3: Owner-Only On Disk
**Goal:** Nothing verbatim writes is readable by group or world. `from_mode` appears in `install/binary.rs`, `install/json_file.rs` and tests, and nowhere else: `crates/verbatim-core/src/store/open.rs` and `crates/verbatim/src/cmd/export.rs` set no modes at all, so under umask 022 the data dir and export dir land 755 and `verbatim.db` and `export/*.jsonl` land 644. The export is correctly labelled unredacted, but a label is not a permission. Separately, `credentials.rs` refuses a shared credentials file with any group or world bit, while the tier-2 `api_key` written directly in `verbatim.toml` is read with no such check.
**Depends on:** Nothing
**Requirements:** PRIV-02, PRIV-04
**Success Criteria:**
1. A test that sets umask 022 itself observes, after a fresh run: data dir 0700, `verbatim.db` 0600, its `-wal` and `-shm` 0600, the snapshots dir 0700 with 0600 files, the scratch dir 0700, and the export dir 0700 with 0600 files.
2. A store written by an earlier build has its modes tightened on open, exactly once, and `verbatim verify` passes over it afterwards. Tightening verbatim's OWN files is repair; the shared credentials file keeps today's read-and-refuse behaviour, because it is shared with every other product on the machine and widening or narrowing it silently is the thing that module refuses to do.
3. `verbatim.toml` carrying `provider.api_key` with any group or world bit set is refused the way the shared file is: the refusal names the path and the octal mode and carries no byte of the file's contents.
4. `verbatim doctor` prints the `chmod` that fixes each refusal, and states the Windows ACL deferral (D-15) in words rather than leaving it inferred.
5. `verbatim doctor` warns, without refusing, when `provider.local = true` and `base_url` is not a loopback address. D-13 stands: the declaration decides where bytes go, not the address, so this reports a likely mistake and changes no behaviour.
6. No path creates a file with a wide mode and then narrows it. The mode is set at creation, so there is no window in which the file exists readable.

### Phase 4: Redacted Recall
**Goal:** An opt-in knob filters the DERIVED recall projections. `recall_search` excerpts, `recall_context` windows, the injected brief and `recall_get` all derive from raw archived text, and `crates/verbatim/src/cmd/mcp/tools.rs:498` deliberately hands back the record's own archived bytes. Because Claude Code places hook and MCP output into the model's context, those are egress boundaries even though verbatim itself opens no socket. The canonical archive stays lossless and ingest-time redaction stays barred; this filters a projection on the way out. Default is today's raw behaviour, because every byte in the archive already reached the provider once when it was typed, and redacting by default would delete the product.
**Depends on:** Phase 2 (it reuses the widened rule set)
**Requirements:** PRIV-03, RCL-09, OBS-03
**Success Criteria:**
1. One config knob, default off, routes `recall_search` excerpts, `recall_context` windows, the `SessionStart` brief and `recall_get` through the Phase 2 rule set.
2. With the knob off, every one of those four outputs is byte-identical to the current build's.
3. The archive is untouched in both settings: the stored blob bytes and `verbatim verify`'s digests are the same with the knob on and off, and the ingest path still names no filter.
4. The terminal path can still print raw text while the knob is on, behind an explicit flag, so the knob never makes an owner's own archive unreadable to them.
5. With the knob on, successful judgment `topic`, `outcome` and `claim` text is filtered before it reaches SQLite, closing the gap where only failed responses were scrubbed (`observe/judgment.rs:359`, `:804`) and a provider that echoed a secret made it durable and retrievable.
6. Round trip: a secret planted in a transcript, ingested, then requested back through all four recall paths, is absent from all four with the knob on and present in all four with it off.
