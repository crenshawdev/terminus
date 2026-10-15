PLAN COMPLETE
Plan: .planning/phases/2/PLAN-2.md
Tasks: 4 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - The detached spawn is a new process group and nothing else | 51a6ee4 | spawn.rs grep empty; hook tests 8/8 pass; clippy clean |
| 2 - Install's paths and copy are Unix only | 7129fdf | AC2 grep empty over binary.rs, targets.rs, json_file.rs; binary.rs grep prints only the `.windows(` slice call; targets.rs prints only lines 27 and 29 in the Exec form section; bin unit tests 46/46, doctor 15, install 19, uninstall 14 pass; clippy clean |
| 3 - Doctor, install and uninstall print only Unix fixes | 5095949 | source grep empty; `windows` grep prints only doctor.rs:446 (exec-form comment) and :1208 (on_path doc); doctor --json key set identical before and after (4 top-level, 14 data keys); install/doctor/doctor --json/uninstall output carries no attrib, del, .exe or Windows; read-only data dir fix is `chmod u+w '<dir>'`; doctor 15, install 19, uninstall 14 pass; clippy clean |
| 4 - The npm package leaves the repository | 2404121 | `npm/` gone; `git grep -i npm` over crates, docs, Cargo.toml, .gitignore empty; commit deletes 10 files under npm/ and modifies .gitignore |
| Suite | - | `cargo test --workspace --all-features` (workspace manifest; workflow.test_command is null): 749 passed, 0 failed, matches the AC5 baseline. AC2 grep over crates/terminus/src empty |

Deviations:
- [deviation] Task 4 said `npm/` holds nine tracked files and its Verify expected nine deletions; `git ls-files npm` lists ten (npm/README.md, pack-local.sh, five platform package.json, and npm/terminus README.md, package.json, bin/terminus.js). All ten were removed, since the goal is the whole tree; nothing else rode the commit.

Open items:
- `.planning/reads.jsonl` showed as modified in the worktree throughout, written by tooling outside this plan; it was left unstaged and is not in any commit.
