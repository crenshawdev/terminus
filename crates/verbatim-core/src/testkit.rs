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
    "session-compacted.jsonl",
    "subagents/agent-alpha.jsonl",
    "subagents/workflows/wf_demo/agent-deep.jsonl",
    "session-recall.jsonl",
    "session-errors-a.jsonl",
    "session-errors-b.jsonl",
    "subagents/agent-echo.jsonl",
];

/// The token the phase 3 fixtures carry where a real transcript carries an
/// absolute `cwd`.
///
/// The phase 1 and 2 fixtures hardcode `/data/code/verbatim`, which makes any
/// project-scoped assertion over them true only on a checkout at that literal
/// path - and, worse, true for the wrong reason on this one, since `git
/// rev-parse` answers for a directory that really is there. The phase 3
/// fixtures name a root the test owns instead, so a project key is whatever the
/// test built and nothing else.
pub const FIXTURE_ROOT_TOKEN: &str = "{{ROOT}}";

/// The fixtures carrying [`FIXTURE_ROOT_TOKEN`], and the project directory each
/// one's `cwd` names beneath the substituted root.
///
/// Two distinct projects across the four, which is what lets a scoped search
/// have something to be both true and false about.
pub const ROOTED_FIXTURES: &[(&str, &str)] = &[
    ("session-recall.jsonl", "project-alpha"),
    ("subagents/agent-echo.jsonl", "project-alpha"),
    ("session-errors-a.jsonl", "project-beta"),
    ("session-errors-b.jsonl", "project-beta"),
];

/// The project directory one rooted fixture's `cwd` names beneath `root`.
///
/// Panics for a fixture that is not in [`ROOTED_FIXTURES`]: a test asking where
/// a fixture's project lives, for a fixture that hardcodes its `cwd`, is a
/// broken test rather than a runtime condition.
pub fn fixture_project(name: &str, root: &Path) -> PathBuf {
    let (_, project) = ROOTED_FIXTURES
        .iter()
        .find(|(fixture, _)| *fixture == name)
        .unwrap_or_else(|| panic!("{name} is not a rooted fixture"));
    root.join(project)
}

/// Every project directory [`ROOTED_FIXTURES`] names beneath `root`, in
/// declaration order and without repeats.
pub fn fixture_projects(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for (_, project) in ROOTED_FIXTURES {
        let path = root.join(project);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// Copy a rooted fixture into `dir` with [`FIXTURE_ROOT_TOKEN`] rewritten to
/// `root`, creating the project directory the `cwd` names.
///
/// The directory is created and not merely named: project identity resolves
/// through git for a `cwd` that still exists, and the read-side tests chdir into
/// it. `root` should therefore be somewhere the test owns and no repository
/// contains, so the key degrades to the path itself on every machine.
///
/// Backslashes are doubled on the way in because the substitution lands inside
/// a JSON string literal, where a Windows root would otherwise be an invalid
/// escape and the whole record would stop parsing.
pub fn copy_rooted_fixture_into(name: &str, dir: &Path, root: &Path) -> PathBuf {
    let project = fixture_project(name, root);
    std::fs::create_dir_all(&project)
        .unwrap_or_else(|e| panic!("create {}: {e}", project.display()));

    let text = String::from_utf8(fixture_bytes(name)).expect("fixtures are UTF-8");
    let replacement = root.to_string_lossy().replace('\\', "\\\\");
    let rooted = text.replace(FIXTURE_ROOT_TOKEN, &replacement);

    let dest = dir.join(name);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::write(&dest, rooted).unwrap_or_else(|e| panic!("write {}: {e}", dest.display()));
    dest
}

/// The fixture whose **last** line is a compaction boundary (D-21).
///
/// Named rather than spelled at each site because two phases' tests append that
/// one line to another transcript to make a boundary appear under an
/// already-archived session, which is what AC4 asks about.
pub const COMPACTED_FIXTURE: &str = "session-compacted.jsonl";

/// The last line of [`COMPACTED_FIXTURE`], without its newline: one whole
/// `compact_boundary` record.
pub fn boundary_line() -> Vec<u8> {
    let bytes = fixture_bytes(COMPACTED_FIXTURE);
    let start = bytes[..bytes.len() - 1]
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .expect("the fixture holds more than one record");
    bytes[start..bytes.len() - 1].to_vec()
}

/// A `.jsonl` file in the transcript tree that is not a transcript (D-12).
pub const NON_TRANSCRIPT_FIXTURE: &str = "subagents/workflows/wf_demo/journal.jsonl";

/// The `agent-*.meta.json` beside `subagents/agent-alpha.jsonl` (D-04).
///
/// Deliberately NOT in [`TRANSCRIPT_FIXTURES`]: it is one JSON object rather
/// than JSONL, discovery's filename filter must keep excluding it, and it must
/// never become a session. Ingest reads it only as opaque bytes for
/// `session_meta.agent_meta`.
pub const AGENT_META_FIXTURE: &str = "subagents/agent-alpha.meta.json";

/// The one fixture with no trailing newline: a transcript still being written.
pub const TRUNCATED_FIXTURE: &str = "session-truncated.jsonl";

/// A token appearing in exactly one turn across the whole fixture set.
pub const UNIQUE_TOKEN: &str = "brillig";

/// Names a **real** Claude config directory whose `projects` subdirectory is a
/// transcript tree to measure against.
///
/// PROJECT.md's testing constraint is a private real-corpus fixture reached by
/// an environment variable, and this is it. Nothing in the repo can hold that
/// tree: it is 1,896+ private transcripts and ~988 MB. A test that reads it
/// skips, loudly, when this is unset, so CI stays green without it.
///
/// It names the *config* directory and not the `projects` tree, so it is the
/// same shape as [`crate::config::CLAUDE_CONFIG_DIR_ENV`] and a run can be
/// pointed at a real tree by copying one value between them.
pub const CORPUS_DIR_ENV: &str = "VERBATIM_TEST_CORPUS";

/// The corpus directory, or `None` with the reason already printed.
///
/// Printed rather than silent: a test that skips without saying so is a test
/// everyone believes is running.
pub fn corpus_dir() -> Option<PathBuf> {
    let Some(value) = std::env::var_os(CORPUS_DIR_ENV).filter(|v| !v.is_empty()) else {
        println!("skipping: {CORPUS_DIR_ENV} is unset, so there is no real corpus to measure");
        return None;
    };
    let dir = PathBuf::from(value);
    let projects = dir.join("projects");
    assert!(
        projects.is_dir(),
        "{CORPUS_DIR_ENV} is set to {}, which has no `projects` directory; \
         it must name a Claude config directory, not the tree itself",
        dir.display()
    );
    Some(dir)
}

/// What a walk of a JSONL stream found: how many lines, which top-level
/// `type` values, and which lines `serde_json` refused.
///
/// The counted failures are the point. `Record::parse` deliberately keeps a
/// non-JSON line as a record with no fields rather than erroring, because a
/// transcript is archived verbatim whatever it holds - so "zero unparseable
/// lines" is a claim the product code structurally cannot make for a test, and
/// this is the independent reader that can.
#[derive(Debug, Default)]
pub struct LineSurvey {
    pub lines: usize,
    /// Top-level `type` values and how many lines carried each. Counted and
    /// never asserted as a closed set (D-25): 16 distinct types were measured,
    /// the set grows upstream, and an exact assertion fails on correct data.
    pub types: std::collections::BTreeMap<String, usize>,
    /// Lines with no `type` at all. Not a failure - the schema is upstream's.
    pub untyped: usize,
    /// Every line `serde_json` could not parse, as `(session, line number)`,
    /// capped so a pathological corpus cannot exhaust memory.
    pub unparseable: Vec<(String, usize)>,
    pub unparseable_total: usize,
}

impl LineSurvey {
    /// How many unparseable lines are kept for the failure message.
    const KEPT: usize = 20;

    /// Absorb one session's decompressed stream, attributed to `session`.
    pub fn absorb(&mut self, session: &str, stream: &[u8]) {
        for (index, line) in stream.split(|b| *b == b'\n').enumerate() {
            // A trailing newline yields one empty tail slice, which is not a
            // line. An empty line anywhere else is not one either.
            if line.is_empty() {
                continue;
            }
            self.lines += 1;
            match serde_json::from_slice::<serde_json::Value>(line) {
                Ok(value) => match value.get("type").and_then(|t| t.as_str()) {
                    Some(kind) => *self.types.entry(kind.to_owned()).or_insert(0) += 1,
                    None => self.untyped += 1,
                },
                Err(_) => {
                    self.unparseable_total += 1;
                    if self.unparseable.len() < Self::KEPT {
                        self.unparseable.push((session.to_owned(), index + 1));
                    }
                }
            }
        }
    }
}

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
///
/// Three of the five entries changed in phase 3, and the reason is the point:
/// `assistant`, `attachment` and `restart` matched `"type":"assistant"`,
/// `"attachment":{...}` and `"gitBranch":"restart"` - JSON keys and field values
/// no user would ever search for - so the comparison had been running over hits
/// that existed only because the raw line was indexed (D-01). Each entry now
/// reaches through a different arm of the projection: a message text block, a
/// `system` record's top-level `content` and a `Bash` tool input, an
/// `attachment` object's string leaf, and a `toolUseResult.stderr`.
pub const FIXED_QUERIES: &[&str] = &["brillig", "cargo", "BRIEF", "SearchManager", "panicked"];

/// Run [`FIXED_QUERIES`] plus a turn range and a turn-id lookup, and read back
/// every `entities` and `paths` row, serialized to JSON in a stable order so two
/// runs are byte-comparable.
///
/// The rows carry `turns.id` and each hit's coordinates, not just a count: a
/// rebuild that renumbered turns would return the same number of hits pointing
/// at different records, which is the exact failure D-10 exists to prevent.
///
/// The exact-match tables are in here for the same reason. AC3 compares this
/// output across a rebuild, and until phase 3 it covered only `turns_fts` and
/// the turn rows - so an extractor that rebuilt `entities` to different values,
/// or to none at all, would have left the comparison byte-identical and the
/// criterion asserting nothing about the tables it names.
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
    out.push_str("\n  ],\n");

    // The exact-match half of recall (RCL-02, RCL-03). Ordered by the columns
    // themselves rather than by rowid: two rebuilds have to agree on the VALUES,
    // and an ordering that leaned on insertion order would hide a re-derive that
    // emitted the same set in a different sequence.
    out.push_str("  \"entities\": [\n");
    let rows: Vec<String> = conn
        .prepare(
            "SELECT turn_id, kind, value_norm FROM entities
             ORDER BY turn_id, kind, value_norm",
        )
        .expect("prepare the entity scan")
        .query_map([], |r| {
            Ok(format!(
                "    [{}, {:?}, {:?}]",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .expect("run the entity scan")
        .map(|r| r.expect("read an entity row"))
        .collect();
    out.push_str(&rows.join(",\n"));
    out.push_str("\n  ],\n");

    out.push_str("  \"paths\": [\n");
    let rows: Vec<String> = conn
        .prepare("SELECT turn_id, path FROM paths ORDER BY turn_id, path")
        .expect("prepare the path scan")
        .query_map([], |r| {
            Ok(format!(
                "    [{}, {:?}]",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
            ))
        })
        .expect("run the path scan")
        .map(|r| r.expect("read a path row"))
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
