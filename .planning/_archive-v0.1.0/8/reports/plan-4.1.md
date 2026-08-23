PLAN CHECKPOINT: structural
Plan: .planning/phases/8/PLAN-4.md
Tasks: 1 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Take one consistent snapshot and keep only the newest few | 3f94aca | `store::snapshot` with `take` (VACUUM INTO into a `snapshots` subdirectory, temp name then rename) and `prune`. 5 new cases in `crates/verbatim-core/tests/snapshot.rs`, all pass; clippy clean. Snapshot names use ISO-8601 **basic** format (`verbatim-20260822T223340.123Z.db`) rather than the extended form every stored column holds: `:` is not a legal filename character on Windows, a first-class target. Still fixed-width and still lexicographically time-ordered, which is the property the prune uses. |
| 2 - Run snapshots by default, on a schedule, outside the lock | (uncommitted) | Implemented and verified in the working tree; NOT committed, because the lease gate refuses one file the fix needs. See below. |

Deviations: none

Open items: none

## Checkpoint detail

**What is in the working tree, uncommitted, for task 2**

- `crates/verbatim-core/src/config.rs` - `[snapshot]` table on `FileConfig`
  (`enabled` / `interval_hours` / `keep`), a resolved `Snapshot` whose `Default`
  is on / 24 h / 3, `DEFAULT_SNAPSHOT_INTERVAL_HOURS` and
  `DEFAULT_SNAPSHOTS_KEPT`, and the three accessors.
- `crates/verbatim-core/src/ingest/pass.rs` - `Snapshotted` on `Summary` beside
  `judgment`, `META_LAST_SNAPSHOT`, and `roll()` called from `run_with` after
  `judge_new`: gate first off a `meta` timestamp read against SQLite's own
  clock, then `snapshot::take` + `snapshot::prune`, then the stamp. Every
  failure is a note; nothing raises.
- `crates/verbatim/src/cmd/ingest.rs` - the notes print on stderr beside the
  judgment notes.
- `crates/verbatim-core/tests/config.rs` - 5 new cases, including task 2's
  Verify (a).
- `crates/verbatim-core/tests/snapshot.rs` - 5 new pass-level cases covering
  Verify (b) through (e).

`cargo test -p verbatim-core --test config --test snapshot` passes (38 + 10).
`cargo clippy --workspace --all-targets -- -D warnings` is clean.

**What is blocked**

Snapshots on by default means `pass::run_with` now writes a third `meta` row
(`last_snapshot_at`) that `backfill::run_with` does not - by design, because
backfill IS the walk and nothing more. `crates/verbatim-core/tests/backfill.rs`
compares the two stores table by table and now fails on `("meta", 2)` versus
`("meta", 3)`. That file already carries the exact concept for this: a
`POST_WALK_TABLES` list of tables written by a step backfill deliberately does
not take, with a doc comment explaining that comparing them "compares that
deliberate asymmetry rather than the walk the two implementations share".

The fix is one word:

```rust
const POST_WALK_TABLES: &[&str] = &["observations", "decisions", "labels", "meta"];
```

It is applied in the working tree - with it the FULL workspace suite is green,
verified twice - but `crates/verbatim-core/tests/backfill.rs` is not in
PLAN-4's `files:` lease, and `lease-check --phase 8 --plan 4` refuses it:

```
{"ok":false,"reason":"undeclared-files","undeclared":["crates/verbatim-core/tests/backfill.rs"]}
```

No other phase-8 plan declares that path (checked PLAN-1 and PLAN-5, the two
that share files with this one).

**Alternatives considered and rejected**

- Keep the schedule state somewhere other than `meta` - e.g. derive "when was
  the last snapshot" from the newest file name in the snapshots directory. Fully
  inside the lease, but it contradicts D-15 ("a timestamp row in `meta` gating
  the snapshot to at most once per 24 hours"), which is a locked decision and
  therefore its own checkpoint.
- Make `backfill` take a snapshot too. Also outside the lease
  (`src/ingest/backfill.rs`), and wrong: backfill deliberately runs none of the
  post-walk steps.
- Commit task 2 without the backfill fix. That leaves HEAD red on a test my
  change broke.
