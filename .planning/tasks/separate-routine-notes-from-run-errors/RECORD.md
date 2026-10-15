# Task: separate-routine-notes-from-run-errors

## What shipped

- runs gains a notes column; routine pass lines (labels written, retention counts) go there and runs.error keeps failures only, so status shows them apart and doctor no longer fails after ordinary use. bring_forward re-reads what is missing under an IMMEDIATE transaction, fixing the column race in #13. The risk_surface review (openai gpt-5.6-sol) raised one finding, refuted: status opens through Store::open, which brings the store forward.

## Commits

| Task | Commit | Description |
| --- | --- | --- |
| 1 | 65ef2d137efee44ddeeee4123c4be06c278e7c38 | feat(store): add runs.notes, and stop two opens racing to add a missing column |
| 1 | 25632ddfd7876c62186cf2fc97ea950b5e7fff48 | fix(ingest): write what a healthy pass did to runs.notes, and keep runs.error for failures |
| 1 | 56dcd7fd63a4de1eaf98163594b774dbfb81a251 | feat(status): print a pass's routine notes apart from its errors |
| 1 | 25304ec2092e6770d15797e98de21b922e2633c2 | docs: drop the routine-notes todo now that it is fixed |
| 1 | b33f9461d7031c22f6b1b9c559fb5f5d6b7cc2d2 | docs: task plan separate-routine-notes-from-run-errors |

## Files

### Task 1: separate-routine-notes-from-run-errors

- **Files:** .planning/CAPTURE.md, .planning/tasks/separate-routine-notes-from-run-errors/PLAN.md, CHANGELOG.md, DESIGN-BRIEF.md, crates/terminus-core/src/feedback/mod.rs, crates/terminus-core/src/ingest/pass.rs, crates/terminus-core/src/retention/apply.rs, crates/terminus-core/src/retention/mod.rs, crates/terminus-core/src/store/open.rs, crates/terminus-core/src/store/schema.rs, crates/terminus-core/tests/label.rs, crates/terminus-core/tests/pass.rs, crates/terminus-core/tests/retention.rs, crates/terminus-core/tests/store_open.rs, crates/terminus/src/cmd/status.rs, crates/terminus/tests/doctor.rs, crates/terminus/tests/status.rs, docs/json-shapes.md
