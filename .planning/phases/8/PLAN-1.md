---
phase: 8
plan: 1
requirements: [REL-01]
files:
  - Cargo.toml
  - Cargo.lock
  - crates/terminus/Cargo.toml
  - crates/terminus/src/main.rs
  - .planning/config.json
---

# Phase 8: Release Builds - Plan 1

## Goal

The phase goal: a tagged version produces a GitHub release that Phase 6's `install.sh` accepts. CI builds `terminus-<os>-<arch>` for Linux and macOS on x86_64 and aarch64, writes each manifest and attaches both to a draft release, and the owner signs and publishes it on his own machine.

This plan makes the source ready to be released. On musl targets the binary uses mimalloc as its global allocator, and the hook budget is measured on a musl build. The workspace version becomes 0.1.0, and cadence stops cutting its own bare tag when it lands the work.

## Must be true when done

- **mimalloc is the allocator on musl, and only there.** `cargo tree` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` lists `mimalloc` under `terminus`. For `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin` and `aarch64-apple-darwin` it does not.
- **A local musl build is static and holds mimalloc, and the hook budget holds on it (AC6, human-verify).** `file` reports the x86_64 musl release binary as statically linked, `nm` finds mimalloc's `mi_malloc_aligned` and `mi_free` in it, and the corpus test passes on that build with every hook event's p99 under the 10 ms release wall.
- **The version is 0.1.0.** `Cargo.toml` and both `terminus` entries in `Cargo.lock` say 0.1.0, and a release build's `terminus --version` prints `0.1.0`.
- **Cadence cuts no release tag.** `git.create_tag` is `false` in `.planning/config.json`, and that commit changes no other key.
- **The gates hold.** `cargo build --release --locked -p terminus` succeeds, `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.

## Context

- Plan 1 of 4. Plans 2 and 3 share no file with this plan. Plan 4 is the live release, and it runs only after this plan has landed on `main`.
- D-01 (linking), D-03 (only its `git.create_tag` half), D-04 (version) and D-08 (build command) in `.planning/phases/8/CONTEXT.md` bind this plan.
- Every entry in `[workspace.dependencies]` has a comment saying why it is there, and the new one follows that. CLAUDE.md requires reading `~/.claude/john-voice.md` in full before writing comments or commit messages, and its Code comments rule applies.
- musl 1.2.6 and `musl-gcc` are installed on this machine (`pacman -Q musl`, 2026-10-14), but the `x86_64-unknown-linux-musl` rustup target is not. Installing a target changes the owner's toolchain, so the local musl build in Task 1 is a human-verify step.
- Out of scope: the workflows, the signing script, the docs, a `rust-toolchain.toml`, and anything for macOS. The macOS build needs no source change for D-01, and plan 2's build job checks its linkage.

## Tasks

### Task 1: Use mimalloc as the global allocator on musl targets

- **Files:** Cargo.toml (anchor: `[workspace.dependencies]`), crates/terminus/Cargo.toml (anchor: `[dependencies]`), crates/terminus/src/main.rs (anchor: the crate attribute and `use` lines above `const USAGE`), Cargo.lock
- **Action:**
  - **The dependency (D-01).** Add `mimalloc` 0.1.52, the current release on crates.io as of 2026-10-14, to `[workspace.dependencies]`. Its default feature set is empty, and no feature is turned on.
    - Its comment says why it is there. musl's own allocator is much slower than glibc's under multi-threaded allocation, and backfill allocates across rayon threads. mimalloc is compiled into the binary, so it adds no runtime dependency. The choice is a judgment, and AC6 measures it.
  - **Only on musl.** The `terminus` crate depends on it under `[target.'cfg(target_env = "musl")'.dependencies]`, so glibc and macOS builds never compile it. `terminus-core` does not depend on it, because the binary chooses the allocator.
  - **The allocator.** In `main.rs`, register `mimalloc::MiMalloc` as the `#[global_allocator]` behind `#[cfg(target_env = "musl")]`. It does not apply on macOS, because D-01 limits mimalloc to the musl targets. Add no cargo feature for it.
  - **The lock.** Refresh `Cargo.lock` with a build, never by hand, so that `--locked` holds. The lock gains the `mimalloc` and `libmimalloc-sys` packages and the new dependency line under `terminus`. It changes no existing package's version.
- **Verify:**
  - `cargo tree -p terminus -e normal --prefix none --locked --target <t> | grep -c '^mimalloc '` prints at least `1` for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. It prints `0` for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin` and `aarch64-apple-darwin`. `cargo tree` resolves for a target without that target being installed.
  - `git diff Cargo.lock` adds `mimalloc` and `libmimalloc-sys` and changes no existing package's `version` line.
  - `cargo build --release --locked -p terminus` succeeds. `cargo test --workspace --all-features` passes, and `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean.
  - human-verify (AC6, and the local half of AC2). This needs the `x86_64-unknown-linux-musl` rustup target, which is absent here. The owner runs:
    - `rustup target add x86_64-unknown-linux-musl`, once.
    - `cargo build --release --locked -p terminus --target x86_64-unknown-linux-musl`. Then `file target/x86_64-unknown-linux-musl/release/terminus` says `static-pie linked` or `statically linked`. Rust's musl targets build a static PIE by default, and `file` 5.x names that `static-pie linked`.
    - `nm target/x86_64-unknown-linux-musl/release/terminus | grep -E ' [Tt] (mi_malloc_aligned|mi_free)$'` prints both symbols.
    - `TERMINUS_TEST_CORPUS="$HOME/.claude" cargo test --release --locked -p terminus --target x86_64-unknown-linux-musl --test corpus -- --nocapture` passes. Its printed lines name the release arm, and each hook event's p99 is under 10 ms. `hook_budget` in `crates/terminus/tests/corpus.rs` asserts that wall in release.

### Task 2: Set the workspace version to 0.1.0

- **Files:** Cargo.toml (anchor: `[workspace.package]` `version`), Cargo.lock (anchor: the `terminus` and `terminus-core` package entries)
- **Action:**
  - **The version (D-04).** Change `[workspace.package]` `version` from `0.1.1` to `0.1.0`, so the first release is 0.1.0, as `CHANGELOG.md` and `PROJECT.md` say. Both `terminus` entries in `Cargo.lock` follow it. Refresh the lock with a build rather than by hand, and change nothing else.
  - **Leave the docs alone.** The CHANGELOG is not touched here, because plan 4 turns its Unreleased section into 0.1.0 after the release is published (D-21).
  - Nothing refuses a lower version. Installing the release later moves the owner's link from the local 0.1.1 build back to 0.1.0, and that is expected (D-04).
- **Verify:**
  - `cargo metadata --no-deps --format-version 1 --locked | jq -r '.packages[] | "\(.name) \(.version)"'` prints `terminus-core 0.1.0` and `terminus 0.1.0`.
  - `cargo build --release --locked -p terminus && target/release/terminus --version` prints exactly `0.1.0`.
  - This task's diff to `Cargo.lock` is exactly the two `version` lines of `terminus` and `terminus-core`.
  - `cargo test --workspace --all-features` passes.

### Task 3: Stop cadence cutting a bare release tag

- **Files:** .planning/config.json (anchor: `git.create_tag`)
- **Action:**
  - **The setting (D-03).** Set `git.create_tag` to `false`. When it is `true`, `/cad-land` cuts a bare `0.1.0` tag on the landed base (its tag-after-merge step). That tag matches no release trigger and builds nothing. Release tags are `v<X.Y.Z>`, and plan 4 pushes `v0.1.0` by hand.
  - **Change nothing else.** At planning time, the working copy of this file already holds uncommitted edits that belong to no task in this phase: `stakes`, `review.reviewers` and `review.providers` are removed (`git diff .planning/config.json`, 2026-10-14).
    - If those edits are still uncommitted when this task runs, commit only the `create_tag` hunk. Stage it non-interactively, by applying a patch of that one hunk with `git apply --cached`.
    - The owner's other edits stay in the working tree as they are.
- **Verify:**
  - `jq '.git.create_tag' .planning/config.json` prints `false`.
  - `git show --stat HEAD` lists only `.planning/config.json`, and `git show HEAD` changes only the `create_tag` line.

## Notes

- **AC6 is now within reach here.** CONTEXT marked AC6 human-verify because musl was absent. musl is now installed, so the one missing piece is the rustup target. The owner adds it, because that changes his toolchain.
- **Recalled prior art.** The manifest and asset names that plan 2 writes come from Phase 5 D-01 (`.planning/phases/5/CONTEXT.md`, phase 5). Nothing in this plan changes them.
