//! Test support: the shared way to reach the synthetic fixture corpus.
//!
//! Behind the `testkit` cargo feature, off by default, so none of it is
//! compiled into the shipped binary. It lives in the library rather than in a
//! `tests/common` module because both workspace crates test against the same
//! fixtures and a third workspace member to hold them would be worse
//! (`.planning/phases/1/PLAN-1.md`, task 2).

use std::path::{Path, PathBuf};

/// The transcript fixtures, relative to [`fixture_dir`].
///
/// `subagents/workflows/wf_demo/journal.jsonl` is deliberately absent: it is
/// not a transcript (D-12) and lives at [`NON_TRANSCRIPT_FIXTURE`].
pub const TRANSCRIPT_FIXTURES: &[&str] = &[
    "session-basic.jsonl",
    "session-large-record.jsonl",
    "session-continuation.jsonl",
    "session-truncated.jsonl",
    "subagents/agent-alpha.jsonl",
];

/// A `.jsonl` file in the transcript tree that is not a transcript (D-12).
pub const NON_TRANSCRIPT_FIXTURE: &str = "subagents/workflows/wf_demo/journal.jsonl";

/// The one fixture with no trailing newline: a transcript still being written.
pub const TRUNCATED_FIXTURE: &str = "session-truncated.jsonl";

/// A token appearing in exactly one turn across the whole fixture set.
pub const UNIQUE_TOKEN: &str = "brillig";

/// `tests/fixtures` at the workspace root.
pub fn fixture_dir() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> always has two ancestors");
    workspace.join("tests").join("fixtures")
}

/// The path of one fixture, by its name relative to [`fixture_dir`].
///
/// Panics when the fixture is missing: a test naming a fixture that does not
/// exist is a broken test, not a runtime condition to handle.
pub fn fixture_path(name: &str) -> PathBuf {
    let path = fixture_dir().join(name);
    assert!(path.is_file(), "missing fixture: {}", path.display());
    path
}

/// The bytes of one fixture, read verbatim.
pub fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// SplitMix64. A test that generates its own inputs has to be reproducible
/// from the seed it prints, and pulling a crate in for four lines is not worth
/// it.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `0..bound`. Panics on a zero bound rather than dividing by it.
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "below(0) has no value to return");
        self.next_u64() % bound
    }
}

/// A seed from the environment when reproducing a failure, otherwise the clock.
/// Printed either way, which is what makes a randomized failure reproducible.
pub fn seed(label: &str) -> u64 {
    let seed = match std::env::var("VERBATIM_TEST_SEED") {
        Ok(v) => v.parse().expect("VERBATIM_TEST_SEED must be a u64"),
        Err(_) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos() as u64,
    };
    println!("{label}: VERBATIM_TEST_SEED={seed}");
    seed
}

/// Copy a fixture into `dir` (a temp dir, in every current caller) and return
/// the path of the copy.
///
/// The caller owns the directory, so the same helper serves a `TempDir`, a
/// data dir under `VERBATIM_DATA_DIR` and a hand-built tree. Nested fixture
/// names keep their subdirectories, because the sidecar layout is part of what
/// the fixture encodes.
pub fn copy_fixture_into(name: &str, dir: &Path) -> PathBuf {
    let dest = dir.join(name);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::copy(fixture_path(name), &dest)
        .unwrap_or_else(|e| panic!("copy {name} to {}: {e}", dest.display()));
    dest
}
