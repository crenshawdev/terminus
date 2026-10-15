PLAN CHECKPOINT: human-verify
Plan: .planning/phases/9/PLAN-1.md
Tasks: 1 of 3
| Task | Commit | Note |
|---|---|---|
| 1 - Switch the license to MIT | 6296392 | LICENSE copied from /code/baley/LICENSE (cmp exits 0); Cargo.toml `license = "MIT"`; cargo metadata --locked prints `terminus-core MIT` and `terminus MIT`; no Apache-2.0 or patent line left in Cargo.toml, DESIGN-BRIEF.md, PROJECT.md or LICENSE; CHANGELOG Changed bullet added naming Apache-2.0 as the old license; clippy and cargo check clean; commit signed (G). Awaiting the owner's RustRover review of LICENSE and Cargo.toml. |
Deviations: [deviation] Task 1's Verify says `git diff --stat` lists only the five task files; the working tree already held uncommitted changes to .planning/FILED.md, .planning/phases/1/PLAN-1.md, .planning/phases/1/PLAN-2.md, .planning/phases/7/PLAN-2.md and .planning/reads.jsonl before this dispatch, so the unscoped stat lists ten files. `git diff --cached --stat` at commit time listed only the five task files, and those pre-existing changes were left unstaged and untouched.
Open items: none
