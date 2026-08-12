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
