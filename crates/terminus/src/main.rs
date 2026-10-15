//! The `terminus` binary. Its code is in `lib.rs`.

use std::process::ExitCode;

// musl's allocator is slow under threads. See mimalloc in the workspace Cargo.toml.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    terminus::main()
}
