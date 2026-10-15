PLAN CHECKPOINT: human-verify
Plan: .planning/phases/10/PLAN.md
Tasks: 0 of 4
| Task | Commit | Note |
|---|---|---|
| 1 - Land phases 1 to 9 on `main` | none | Owner's step, human-verify. Not yet done: after `git fetch origin`, `git ls-tree --name-only origin/main install.sh README.md .github/workflows/release.yml .github/workflows/release-check.yml scripts/sign-release.sh` printed nothing, and `git show origin/main:Cargo.toml \| grep '^version'` printed `version = "0.1.1"`. origin/main is at 5257f95 (Merge pull request #3). `git ls-remote --tags origin 'v[0-9]*'` printed nothing, as expected. |
| 2 - Tag `v0.1.0` on `main` and check the draft | none | Owner's step, human-verify. Waits on Task 1. |
| 3 - Sign, publish and check the release | none | Owner's step, human-verify. Waits on Task 2. Irreversible once published. |
| 4 - Say the release is published | none | Executor's step. Runs only after Task 3's checks pass (D-21). |
Deviations: none
Open items: none
