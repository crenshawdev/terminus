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
    "subagents/workflows/wf_demo/agent-deep.jsonl",
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

/// The fixed query set AC4 compares before and after a rebuild.
///
/// Every one of these matches at least one fixture turn, and `brillig` matches
/// exactly one across the whole corpus. A query that matched nothing would make
/// the comparison pass on an empty index, which is the failure this set exists
/// to catch.
pub const FIXED_QUERIES: &[&str] = &["brillig", "assistant", "cargo", "restart", "attachment"];

/// Run [`FIXED_QUERIES`] plus a turn range and a turn-id lookup, serialized to
/// JSON in a stable order so two runs are byte-comparable.
///
/// The rows carry `turns.id` and each hit's coordinates, not just a count: a
/// rebuild that renumbered turns would return the same number of hits pointing
/// at different records, which is the exact failure D-10 exists to prevent.
pub fn query_set_json(conn: &rusqlite::Connection) -> String {
    let mut out = String::from("{\n");

    out.push_str("  \"matches\": [\n");
    for (index, query) in FIXED_QUERIES.iter().enumerate() {
        let hits: Vec<i64> = conn
            .prepare("SELECT rowid FROM turns_fts WHERE turns_fts MATCH ?1 ORDER BY rowid")
            .expect("prepare the fts query")
            .query_map([query], |r| r.get::<_, i64>(0))
            .expect("run the fts query")
            .map(|r| r.expect("read a rowid"))
            .collect();
        out.push_str(&format!("    {{\"q\": \"{query}\", \"hits\": ["));
        out.push_str(
            &hits
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str("]}");
        out.push_str(if index + 1 == FIXED_QUERIES.len() {
            "\n"
        } else {
            ",\n"
        });
    }
    out.push_str("  ],\n");

    // A turn range for one session, and one lookup by turn id: the two shapes
    // recall reads with, beside the search shape above.
    out.push_str("  \"turns\": [\n");
    let rows: Vec<String> = conn
        .prepare(
            "SELECT id, session_key, turn_seq, record_type, coalesce(tool_name, ''),
                    coalesce(ts, ''), stream_offset, byte_len
             FROM turns ORDER BY id",
        )
        .expect("prepare the turn scan")
        .query_map([], |r| {
            Ok(format!(
                "    [{}, {:?}, {}, {:?}, {:?}, {:?}, {}, {}]",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })
        .expect("run the turn scan")
        .map(|r| r.expect("read a turn row"))
        .collect();
    out.push_str(&rows.join(",\n"));
    out.push_str("\n  ]\n}\n");
    out
}

/// A digest over every `sessions` and `session_meta` row.
///
/// The archive is what a rebuild must not touch, so it gets a comparison of its
/// own rather than being inferred from the derived tables looking right.
pub fn archive_digest(conn: &rusqlite::Connection) -> String {
    let mut hasher = blake3::Hasher::new();
    let mut statement = conn
        .prepare(
            "SELECT s.session_key, s.session_no, s.blob, m.session_id, m.transcript_path,
                    m.checksum, m.uncompressed_len, m.continues_from, m.first_turn_at,
                    m.last_turn_at, m.cwd, m.branch
             FROM sessions s LEFT JOIN session_meta m USING (session_key)
             ORDER BY s.session_no",
        )
        .expect("prepare the archive scan");
    let mut rows = statement.query([]).expect("run the archive scan");
    while let Some(row) = rows.next().expect("read an archive row") {
        for column in 0..12 {
            let value: rusqlite::types::Value = row.get(column).expect("read an archive column");
            hasher.update(format!("{value:?}\u{1f}").as_bytes());
        }
        hasher.update(b"\x1e");
    }
    hasher.finalize().to_hex().to_string()
}

/// Read one turn's bytes out of its session blob, reporting how many blocks
/// had to be decompressed to do it.
///
/// The block count is AC1's instrument and STOR-01's whole claim: "reading a
/// single turn decompresses only the blocks that turn occupies" is a statement
/// about a number, so the number is measured rather than argued from the code.
///
/// Panics rather than returning an error: every caller is a test that named a
/// turn it just wrote.
pub fn read_turn(conn: &rusqlite::Connection, turn_id: i64) -> (Vec<u8>, usize) {
    let (session_key, offset, len) = conn
        .query_row(
            "SELECT session_key, stream_offset, byte_len FROM turns WHERE id = ?1",
            [turn_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap_or_else(|e| panic!("turn {turn_id}: {e}"));

    let blob: Vec<u8> = conn
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [&session_key],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("blob for {session_key}: {e}"));

    let reader = crate::blob::BlobReader::open(&blob).expect("a stored blob parses");
    reader.reset_block_counter();
    let bytes = reader
        .read_range(offset as u64, len as u64)
        .expect("a turn's range is inside its own session stream");
    (bytes, reader.blocks_decompressed())
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
