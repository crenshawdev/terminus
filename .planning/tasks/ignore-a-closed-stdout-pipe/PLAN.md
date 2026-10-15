# Task: ignore a closed stdout pipe

The binary prints to stdout with `println!` at 162 places and `print!` at 2,
across 19 files. Every one of them panics with exit 101 when the reader closes
the pipe: `terminus sessions | head` prints a Rust panic, and the hook's one
stdout write would break its exit-0 rule. `show` turns a closed pipe into a
silent exit 1, and only `doctor` treats it as success. The owner chose one
write helper that drops output once the reader is gone, so the command still
finishes its work and exits with its own code (2026-10-14).

## Task 1: one stdout writer for the whole binary

Files: `crates/terminus/src/out.rs` (new), `crates/terminus/src/main.rs`,
`crates/terminus/src/cmd/json.rs`, `crates/terminus/src/cmd/show.rs`, and every
file under `crates/terminus/src/` that calls `println!` or `print!`.

Action:
- `out.rs` holds `outln!` and `out!`, which write the same bytes as `println!`
  and `print!` through one function. A `BrokenPipe` is dropped; any other write
  error panics exactly as `print!` would, since a full disk under a redirect is
  not a reader that went away.
- `main.rs` declares the module with `#[macro_use]` before `mod cmd`, and denies
  `clippy::print_stdout` outside tests so a new `println!` is refused.
- Every `println!(` and `print!(` under `crates/terminus/src/` becomes
  `outln!(` and `out!(`. `eprintln!` is untouched.
- `Document::emit` writes through the same function. `try_emit`'s doc stops
  saying `emit` panics.
- `show`'s render returns `io::Result`, and its caller treats `BrokenPipe` as
  done rather than `Failure::Silent`, so `show | head` exits with show's own
  code. `broken_pipe` goes.

Verify: `grep -rn '\bprintln!(\|\bprint!(' crates/terminus/src` prints nothing,
and `cargo build -p terminus` succeeds.

## Task 2: prove a closed reader changes no exit code

Files: `crates/terminus/tests/cli.rs`.

Action: one test that runs `sessions`, `sessions --json`, `search`, `show` and
`status` against an ingested store twice, once with stdout read in full and once
with the read end closed before the child writes, and asserts the two exit
codes match and stderr never says "panicked".

Verify: the test passes, and fails with exit 101 when run against the binary
built from the start commit (`git stash` of task 1's source files).

## Outcome

- Task 1 shipped in 5808f88: `out.rs` holds `outln!` and `out!`, all 164 print sites in 19 files use them, `show` ends with its own code on a closed pipe, and `clippy::print_stdout` is denied outside tests.
- Task 2 shipped in 6f23067, formatted in 656f7e3. The test fails against d66a5ec with the stdout panic and passes at HEAD.
- Deviation: the full suite caught `tests/compaction.rs` still asserting the old verify wording from 6636cf8; fixed in 4c1fea6. The full suite then passed, 839 tests across 64 binaries.
