# Task: separate routine notes from run errors

Every pass writes routine lines into `runs.error` on purpose, because it is the
product's only text channel (`ingest/pass.rs:450-457`): "labelled 3 hit, 1
false positive", "retention evicted 2 session(s)", sessions waiting on
retention, excluded sessions left alone. `status` prints them under "error",
and `doctor` reports any non-null `runs.error` as a problem and exits 1. With
injection on by default, ordinary users hit this after normal use. The owner
chose a `notes` column for the routine lines, with `error` kept for what went
wrong, and issue #13's column race fixed in the same task because this is the
first column added since it was filed (2026-10-14).

## Task 1: a `notes` column on `runs`, and no race adding it

Files: `crates/terminus-core/src/store/schema.rs`,
`crates/terminus-core/src/store/open.rs`.

Action:
- `runs` gains `notes TEXT`, declared last, and `BRING_FORWARD_COLUMNS` gains
  it last in the `runs` entry, so a fresh store and an upgraded one have the
  same column order.
- `bring_forward` keeps its cheap read, and when anything is missing it reads
  the missing tables and columns again inside an IMMEDIATE transaction before
  altering, as `drop_summary_columns` already does. Two processes opening one
  older store at once can then no longer both try the same `ALTER`.

Verify: `cargo test -p terminus-core --features testkit --test schema --test
store_open` passes; both iterate `BRING_FORWARD_COLUMNS`, so a store missing
`runs.notes` is shown to gain it and to match a fresh store.

## Task 2: routine lines go to `notes`, problems stay in `error`

Files: `crates/terminus-core/src/feedback/mod.rs`,
`crates/terminus-core/src/retention/apply.rs`,
`crates/terminus-core/src/ingest/pass.rs`,
`crates/terminus-core/src/ingest/backfill.rs`,
`crates/terminus-core/tests/pass.rs`, and the doc comments that say these lines
go to `runs.error` (`observe/mod.rs`, `recover.rs`, `retention/mod.rs`).

Action:
- `Labeled` and `Applied` each report two lists: the lines for what could not
  be done (`error`) and the routine counts (`notes`). Labels written go to
  notes. Evicted, deleted, waiting and excluded counts go to notes. Their
  `notes` fields, which hold failures, go to error.
- `pass::record_pass` writes both columns, each null when empty. File
  failures, watermark repairs, discarded decision files, observation failures
  and a failed pass stay in `error`. `backfill::record_pass` writes `notes` as
  null, since a backfill runs neither step.
- The two retention tests in `tests/pass.rs` read the notes column and assert
  `error` is null.

Verify: `cargo test -p terminus-core --features testkit --test pass --test
feedback --test label` passes, and a pass that only evicts leaves `runs.error`
null.

## Task 3: `status` and `doctor` read the two apart

Files: `crates/terminus/src/cmd/status.rs`, `crates/terminus/tests/status.rs`,
`crates/terminus/tests/doctor.rs`, `docs/json-shapes.md`, `DESIGN-BRIEF.md`.

Action:
- `status` prints the notes under a `notes` heading before `error`, and
  `status --json` gains `"notes"` beside `"error"` in `last_run`.
- `doctor` already reads only `error`, so it needs no change; a test pins that a
  last run with notes and no error is `ok`.
- `docs/json-shapes.md` shows `"notes": null` in the `status` shape, and
  `DESIGN-BRIEF.md:67` lists notes in the `runs` row.

Verify: a `status` test shows a routine line under `notes` and not under
`error` in both modes, and a `doctor` test shows `last_run` ok with notes set.
`cargo test --workspace --all-features` passes.

## Outcome

- Task 1 shipped in 65ef2d1: `runs.notes`, and `bring_forward` reads what is missing again under an IMMEDIATE transaction. The new concurrent-open test failed on the old code in its first round with "duplicate column name: notes" (#13).
- Task 2 shipped in 25632dd: `Labeled` and `Applied` report `errors()` and `notes()`, and `record_pass` writes each to its own column. The labelling test fails without it with "labelling was reported as an error".
- Task 3 shipped in 56dcd7f: `status` prints notes apart from errors in both modes, a `doctor` test pins a notes-only run as ok, and json-shapes, DESIGN-BRIEF and the changelog say what each column holds. Also a changelog line, which the plan did not list.
- The full suite passed, 842 tests across 64 binaries.
