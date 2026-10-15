---
phase: 9
status: complete
completed: 2026-10-14
---

# Phase 9: README and User Docs - Summary

A root `README.md` that opens on lost sessions and leads with the signed release, four user docs pages under `docs/`, `TRUST.md`, `SECURITY.md` and the switch to MIT, each reviewed by the owner in RustRover and checked against the code with `/cad-docs-verify`.

## What shipped

- The license is MIT: `LICENSE` is Baley's MIT text, and `Cargo.toml`, `DESIGN-BRIEF.md`, `.planning/PROJECT.md` and the CHANGELOG say so.
- `TRUST.md` - the shared trust text, naming John Crenshaw.
- `SECURITY.md` - private reports through the Security tab; private vulnerability reporting is on for crenshawdev/terminus.
- `docs/release.md` - how a release is built and signed, what `install.sh` checks, the pinned key with its fingerprint, and how to check a release by hand.
- `docs/configuration.md` - every `terminus.toml` key with its default and effect, where the file and the data directory are found, and the environment variables and Claude Code settings Terminus reads.
- `docs/commands.md` - every command with every option its parser accepts, except `backfill --work` and the top-level `--reparent`.
- `docs/install.md` - installing, updating, a local build, uninstalling, and what stays behind.
- `docs/json-shapes.md` - corrected in eight places against what each command emits.
- `README.md` - thirteen sections in the brief's order, Trust through License.
- `.planning/phases/9/CLAIMS.md` - each Trust and Current state claim beside what backs it, and a line in phase 10's Task 1 saying the landing PR carries it.

## Commits

| Plan | Task | Commit | Description |
|---|---|---|---|
| 1 | 1 | 6296392 | License Terminus under MIT |
| 1 | 2 | 1dc995f | Add TRUST.md, with the brief's line 3 naming John Crenshaw |
| 1 | 3 | f175aeb | Add SECURITY.md |
| 2 | 1 | 939d53a | Write docs/release.md |
| 2 | 2 | e3862df | Write docs/configuration.md |
| 2 | 3 | 9f61d13 | Write docs/commands.md |
| 2 | 4 | d673e64 | Write docs/install.md |
| 2 | 4 | 9fbfb89 | Fix the four stale claims `/cad-docs-verify docs/` found in the plan's pages |
| 2 | 4 | 555ab1c | Fix json-shapes.md against the code (owner's ruling) |
| 3 | 1 | 6a8d0a7 | README: the opening and the Status line |
| 3 | 2 | 9e05188 | README: Trust and What it is designed to refuse |
| 3 | 3 | 7fb953e | README: What it does and Current state |
| 3 | 4 | d0259b0 | README: How it works and Hosts and platforms |
| 4 | 1 | 241c512 | README: Installing and updating, Commands and Settings |
| 4 | 2 | bb48468 | README: How it is designed and built, Issues and contributions, Sponsoring and License |
| 4 | 2 | 4289b41 | README: fix the four claims `/cad-docs-verify README.md` found |
| 4 | 3 | 684c72a | CLAIMS.md and the line in phase 10's plan |

## Deviations

- [deviation] `TRUST.md` is no longer byte-identical to `/code/baley/TRUST.md`. At task 2's review the owner changed line 3 from "crenshawdev" to "John Crenshaw". The coordinator made the edit in the brief's block, widened PLAN-1's `files:` to cover `BRIEF.md`, and the executor re-extracted `TRUST.md` and amended task 2's commit (1dc995f). Baley's copy still says "crenshawdev".
- [deviation] Plan 1, task 1: the plan's unscoped `git diff --stat` check listed five planning files that were already modified before the phase started. The staged set at commit time held only the task's five files, and the earlier changes were left untouched.
- [deviation] At the owner's ruling, PLAN-2's `files:` was widened to `docs/json-shapes.md` so the stale claims `/cad-docs-verify docs/` found there were fixed in this phase (555ab1c), along with the replay example's `entity_rank`, changed from 5 to the default 3.
- [deviation] The executors for plans 3 and 4 each ran the full suite more than once. The extra runs only counted results; every run was green.

## Open items

- `/code/baley/TRUST.md` line 3 still says "crenshawdev". The owner updates it in Baley so the shared copy matches again.
- `crates/terminus/src/main.rs` USAGE says `uninstall` removes what `install` added without naming the versions folder it keeps. Out of this phase, which changes no code.
- `crates/terminus/src/cmd/doctor.rs:116-117` and `:178-180`: the comments on a null `fix` are narrower than what the code does. Out of this phase, which changes no code.

## Goal check

The commits deliver the phase goal. `README.md` opens on the reader's problem, a new session or a compaction losing earlier work (README.md:3), before naming any part of Terminus. Its sections run Trust through License in the brief's order (`grep '^## ' README.md`), and the Trust section leads with the signed release and links the release page (README.md:15). The user docs cover installing (`docs/install.md`), every command (`docs/commands.md`), every setting (`docs/configuration.md`) and how a release is built, signed and checked by hand (`docs/release.md`, sections at lines 13, 30, 42 and 84). `/cad-docs-verify` found 10 stale claims in `docs/` and 3 in the README; all 13 were fixed (9fbfb89, 555ab1c, 4289b41), and the README's line 52 was reworded at the owner's ruling. Every plan's closing suite was green (858 passed in plans 2 and 4). Two things are outside this phase: the install line on README.md works only once phase 10 lands `install.sh` on `main` (D-15), and Baley's `TRUST.md` has to be updated to match.
