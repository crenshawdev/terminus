# Declined: findings this repository said no to

Written by `issue-filing.mjs file` when a gate's finding is DECLINED, read by
`issue-filing.mjs unfixed` as the set it must never ask about again. Same row
grammar as FILED.md: the date, the provider, the repository slug, the finding's
(file, claim) fingerprint and its title.

NOT part of the recall corpus and not a queue. `planning.mjs recall` reads
CAPTURE.md, ARCHIVE.md, the task records and FILED.md by explicit path, and
this file is deliberately absent from that list - a decline is a closed
question, and putting closed questions in front of a planner is the
accumulation the corpus rules exist to prevent. Query it, do not load it.

A line that is not a row is skipped, so a human note added here changes nothing
the seam reads.

- 2026-10-13 github crenshawdev/terminus fea198bf838c2047: [cadence fea198bf838c2047] The redaction task proves the planted rule set only through `recall::context::window`, so it does not deliver AC6's requirement that every planted secret and corresponding m
- 2026-10-13 github crenshawdev/terminus fdb2ad5a6b589dc9: [cadence fdb2ad5a6b589dc9] The plan asserts that files, commands, and errors remain searchable after a newly finished session, but no task creates or updates a test that ingests such a session and exe
- 2026-10-13 github crenshawdev/terminus f8221c560ccd0aab: [cadence f8221c560ccd0aab] Task 4's verification explicitly includes `crates/terminus-core/tests/store_open.rs` via the `--test store_open` target, but that file is absent from both Task 4's Files lis
- 2026-10-13 github crenshawdev/terminus a20a9496429c8068: [cadence a20a9496429c8068] The no-network enforcement is only a lexical blacklist for four Rust names, so its verification can pass while source code still opens a network connection through another A
- 2026-10-13 github crenshawdev/terminus 28151f50422ebe07: [cadence 28151f50422ebe07] The module documentation still says observations must remain archival because their judgment half is a paid model call that replay cannot reproduce, contradicting this chang
- 2026-10-13 github crenshawdev/terminus 993a1d36ec995f6b: [cadence 993a1d36ec995f6b] The source scan does not fail on any socket type as claimed; it only searches for four spellings.
- 2026-10-13 github crenshawdev/terminus dab6e843af0659c0: [cadence dab6e843af0659c0] A schema object that references any retired column permanently prevents every writable open instead of being migrated or producing a usable store.
- 2026-10-13 github crenshawdev/terminus 52c135371ecaf489: [cadence 52c135371ecaf489] The early return leaves the retired token-budget metadata behind whenever the columns are already absent.
- 2026-10-13 github crenshawdev/terminus ada36b0361c3e60f: [cadence ada36b0361c3e60f] Using column absence as the only migration record makes the destructive schema change indistinguishable to older binaries, with no format gate to prevent them opening an inc
- 2026-10-13 github crenshawdev/terminus d8340e6822f5c9a4: [cadence d8340e6822f5c9a4] The plan deliberately retains and adds npm references in active documents, contradicting Phase 2 success criterion 1 in ROADMAP.md, which permits npm mentions only in CHANGE
- 2026-10-13 github crenshawdev/terminus d3f467bcd57a04b4: [cadence d3f467bcd57a04b4] The plan writes or replaces the version file before its second stable-path ownership check, so a refusal can occur after the first write despite the plan's stated refusal-be
- 2026-10-13 github crenshawdev/terminus 2fdb16bbd6024f8a: [cadence 2fdb16bbd6024f8a] Checking ownership twice does not make the destructive `mv -f` safe because the final check and replacement are separate filesystem operations.
- 2026-10-13 github crenshawdev/terminus ef736ac9cf877e34: [cadence ef736ac9cf877e34] The plan deliberately leaves known stale installation documentation in place, so the documentation will not consistently describe the layout produced by plans 1 and 2.
- 2026-10-13 github crenshawdev/terminus 885506f1ff745a44: [cadence 885506f1ff745a44] The path-consistency verification only greps for `crenshawdev` and cannot detect different versions-folder spellings between the Rust implementation and the script.
- 2026-10-13 github crenshawdev/terminus 36c0ffd4fe5d9455: [cadence 36c0ffd4fe5d9455] The staging directory is trusted without rejecting symbolic links in the versions path, so installation can overwrite a file outside the versions folder.
- 2026-10-13 github crenshawdev/terminus 47ee72faa4c8be95: [cadence 47ee72faa4c8be95] An existing stable-path symlink to a marked regular file outside the versions folder is treated as an owned file and replaced.
- 2026-10-13 github crenshawdev/terminus b1189ab95ecec713: [cadence b1189ab95ecec713] Stable-path ownership checking and replacement are separated by a race, allowing an unowned entry to be overwritten.
- 2026-10-13 github crenshawdev/terminus c54fe3a9d9d79107: [cadence c54fe3a9d9d79107] The verified download is not bound atomically to the version file that is later linked and executed, so concurrent installs can make an invocation run bytes it did not verif
- 2026-10-13 github crenshawdev/terminus 9fe918c36c696711: [cadence 9fe918c36c696711] Failure of `terminus install` has no rollback, despite the script already committing the new stable link and allowing the child to mutate Claude settings.
- 2026-10-13 github crenshawdev/terminus ac8f7e877a164f80: [cadence ac8f7e877a164f80] The artifact deliberately contradicts CONTEXT's locked `Plan shape: one plan` decision by splitting the phase into two plans.
- 2026-10-13 github crenshawdev/terminus f03695a4a5ff2c08: [cadence f03695a4a5ff2c08] The cleanup trap recursively deletes the path printed by `mktemp` without validating that it is a newly created scratch directory.
- 2026-10-13 github crenshawdev/terminus dd643e42688cc197: [cadence dd643e42688cc197] The piped-install test never truncates the script, so it does not test the stated guarantee that a pipe cut off mid-transfer runs nothing.
- 2026-10-14 github crenshawdev/terminus 8ddde0be1e330c3c: [cadence 8ddde0be1e330c3c] Range arithmetic is unchecked, so a malformed negative turn offset or length can panic or address the wrong blocks instead of failing that range.
- 2026-10-14 github crenshawdev/terminus 3b4777e47696b83d: [cadence 3b4777e47696b83d] The format-1 fixture is stamped with the current `DERIVED_SCHEMA`, so the test does not exercise opening a real older store whose archive and derived-schema versions are bot
- 2026-10-14 github crenshawdev/terminus 8b340b87575756a8: [cadence 8b340b87575756a8] The asserted every-command coverage explicitly omits `mcp`, even though its different response protocol does not prevent testing refusal and byte preservation.
- 2026-10-15 github crenshawdev/terminus d181fa2ad3822868: [cadence d181fa2ad3822868] The signing command omits the required `-` file operand, so Task 1 cannot produce signatures as claimed.
- 2026-10-15 github crenshawdev/terminus ac62534386b798da: [cadence ac62534386b798da] The AC6 verification benchmarks a musl integration-test executable, not the released `terminus` binary whose allocator is configured in `main.rs`.
- 2026-10-15 github crenshawdev/terminus 7722ac08ed978eed: [cadence 7722ac08ed978eed] The workflow-policy test does not cover every workflow file as AC5 and D-11 require.
- 2026-10-15 github crenshawdev/terminus 699f64f7f48ed4c1: [cadence 699f64f7f48ed4c1] Task 3 declares no files but performs a destructive fresh-install sequence that changes user files and data outside the plan's declared file set.
- 2026-10-15 github crenshawdev/terminus 4f39a8d39f7ab8f5: [cadence 4f39a8d39f7ab8f5] The regression test does not actually guarantee that workflows contain no secret reads or signing steps; it only rejects four exact, case-sensitive substrings.
- 2026-10-15 github crenshawdev/terminus 3d3472e10a277bb2: [cadence 3d3472e10a277bb2] The draft check is vulnerable to a time-of-check/time-of-use race, so it does not guarantee that the release is still a draft when the later upload and edit occur.
- 2026-10-15 github crenshawdev/terminus 2f63241ee7d4acb5: [cadence 2f63241ee7d4acb5] The bring-forward migration cannot preserve the promised fresh-schema column order when an existing `decisions` table already has `dropped` but lacks `compacted`.
- 2026-10-15 github crenshawdev/terminus 52ccb4bb98078d9a: [cadence 52ccb4bb98078d9a] Using one file descriptor prevents pathname replacement from mixing files, but it does not make the prefix verification and tail read atomic; an in-place truncate/rewrite ca
