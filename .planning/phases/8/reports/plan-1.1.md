PLAN CHECKPOINT: human-verify
Plan: .planning/phases/8/PLAN-1.md
Tasks: 1 of 3
| Task | Commit | Note |
|---|---|---|
| 1 - Use mimalloc as the global allocator on musl targets | 3b321c3 | Automated half verified. `mimalloc` 0.1.52 (default features `[]`, none enabled) added to `[workspace.dependencies]` with a why-comment; `terminus` depends on it under `[target.'cfg(target_env = "musl")'.dependencies]`; `main.rs` registers `mimalloc::MiMalloc` as `#[global_allocator]` behind `#[cfg(target_env = "musl")]`. Lock refreshed by `cargo build --release -p terminus`: adds `mimalloc` 0.1.52 and `libmimalloc-sys` 0.1.49 (depends on `cc` only) plus the `mimalloc` line under `terminus`; no existing `version` line changed. `cargo tree ... --target <t> \| grep -c '^mimalloc '`: x86_64-unknown-linux-musl 1, aarch64-unknown-linux-musl 1, x86_64-unknown-linux-gnu 0, x86_64-apple-darwin 0, aarch64-apple-darwin 0. `cargo build --release --locked -p terminus` ok; `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean; `cargo check --all-targets` clean. `cargo test --workspace --all-features` not run per task: the contract allows the full suite once, at the end of the plan, and it runs there. Human-verify half (AC6 and the local half of AC2) is outstanding: needs the `x86_64-unknown-linux-musl` rustup target, which only the owner adds. |
Deviations: none
Open items: none
