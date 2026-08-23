PLAN COMPLETE
Plan: .planning/phases/8/PLAN-3.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1: `verbatim compact` | 970f2c4 | `VACUUM` + `PRAGMA wal_checkpoint(TRUNCATE)` under the ingest lock; new `cmd::Footprint`/`footprint`/`human` in `cmd/mod.rs`; 4 cases in the new `tests/lifecycle.rs`. Falsified by removing the checkpoint: `after.wal` became 428,512 and `reclaimed_bytes` -428,512 - D-08's measurement reproduced, and the wal-zero case is the one that caught it while the `status` size comparison still passed. |
| 2: `verbatim usage` | 1a4c529 | Archive bytes by project and by month through `config::visible`, plus a `dbstat('main',1)` footprint with freelist and lock-page rows; 3 cases. Falsified by zeroing the freelist row: the footprint sum came up 8,192 short of `page_count * page_size`. |
| 3: `verbatim export` | 305b303 | One `.jsonl` per session from `blob::read_all`, plus a pretty-printed `manifest.json`; the unredacted-contents notice in the manifest, the human output and the `--json` document; 6 cases. |
| 4: swept contract + docs | 0420c48 | `compact`, `usage`, `export` in `DATA_COMMANDS`; `sweep_args` grew an export destination; three sections in `docs/json-shapes.md`. Falsified by adding an undocumented field to `compact`'s list: property 1 failed naming the mismatch. |

Measured, not assumed:
- `dbstat('main', 1)` over the live 1,114,308,608-byte store: 0.144 s wall for the whole grouped answer, against 272,048 rows for the per-page form. The aggregate form is what shipped.
- The same store's `sum(pgsize)` plus freelist came to 1,114,304,512 against a file of 1,114,308,608 - short by exactly one 4,096-byte page. That is SQLite's lock page at byte `0x40000000`, present on any database past 1 GiB, and it has a row of its own so the documented reconciliation holds on the stores big enough to care rather than only on small ones.
- Whole workspace: `cargo test --workspace` green, `cargo clippy --all-targets -- -D warnings` clean.

Deviations: none

Open items:
- `cmd::status` and `cmd::uninstall` still hold private copies of the three-file footprint rule that `cmd::mod`'s new `footprint`/`human` now also expresses. Folding them in is an edit to two shipped human outputs (and to `status.rs`, which this plan's `files:` lease does not name), so the new helper documents the duplication instead of removing it.
- `usage`'s null buckets are keyed with a JSON `null` rather than a sentinel string like `"(no project)"`. Chosen for consistency with the rest of the `--json` contract, where an absent project is already null and a project key is always an absolute path; the property the plan asks for - the row exists, is counted, and its bytes are inside the total - is what the test asserts.
- `export`'s destination rule is stricter than the plan's "must not already hold an export": any non-empty destination is refused. A `.jsonl` the user put there themselves is the file that must not be silently replaced, and the refusal message tells the two cases apart.
- The plan's task 4 `Verify:` mentions "the empty-result test that each exits 0 against a machine with no store". `cli.rs` has no such sweep - property 3a runs against the populated bench - so the three no-store cases live in `tests/lifecycle.rs`, one per command, each also asserting no data directory is created. Adding a no-store sweep to `cli.rs` would newly hold nine already-shipped commands to a property none of them was written against.
