//! The lifecycle commands at the process boundary: `compact`, `usage`,
//! `export` (RET-04, RET-05, PRIV-04).
//!
//! One bench for all three because they are three questions about one store -
//! how big it is, where the bytes went, and what comes out of it - and the
//! interesting cases are the ones where a delete has already happened. Every
//! number is read back through the binary rather than computed a second time
//! here: `usage`'s whole claim is that its totals reconcile, and a test that
//! recomputed the totals its own way would prove only that the test and the
//! command agree.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;
use verbatim_core::retention::{self, Selection};
use verbatim_core::store::DB_FILE_NAME;
use verbatim_core::{testkit, Store};

const PROJECT: &str = "-data-projects-cadence";

/// How many transcripts a stocked bench holds.
///
/// Enough that deleting half frees whole SQLite pages rather than slack inside
/// pages already in use - a compaction that reclaims nothing measurable would
/// make the "strictly smaller" assertion depend on the page size.
const SESSIONS: u8 = 8;

/// The same isolation `cli.rs`, `status.rs` and `retention.rs` use, and for the
/// same reason: a spawn that set only `VERBATIM_DATA_DIR` would resolve the
/// developer's real config and walk the live `~/.claude` tree.
struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let data_dir = root.join("data");
    let config_dir = root.join("config");
    let claude_dir = root.join("claude");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        config_dir,
        claude_dir,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_verbatim"))
            .args(args)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn verbatim")
    }

    fn place(&self, project: &str, n: u8, fixture: &str) -> PathBuf {
        let name = format!("{n:08x}-1111-4111-8111-111111111111.jsonl");
        let dest = self.claude_dir.join("projects").join(project).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    fn ingest(&self) {
        let out = self.run(&["ingest"]);
        assert!(out.status.success(), "ingest: {}", stderr(&out));
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// The footprint `verbatim status` reports: `verbatim.db` and its two WAL
    /// sidecars, read back through the binary rather than measured here.
    fn size_bytes(&self) -> u64 {
        let out = self.run(&["status", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        document(&out)["data"]["size_bytes"]
            .as_u64()
            .expect("status reports size_bytes")
    }

    /// Delete these sessions through the product's own delete (RET-02), not
    /// through hand-written SQL: what `compact` reclaims is exactly the pages
    /// `retention::apply` freed, and a test that deleted differently would be
    /// compacting a store no user can produce.
    fn delete(&self, keys: &[&Path]) {
        let mut store = Store::open(&self.data_dir).unwrap();
        let selection = Selection {
            delete: keys
                .iter()
                .map(|k| k.to_string_lossy().into_owned())
                .collect(),
            ..Selection::default()
        };
        let applied = retention::apply(&mut store, &selection);
        assert_eq!(applied.deleted.len(), keys.len(), "{applied:?}");
        assert!(applied.notes.is_empty(), "{applied:?}");
    }
}

/// A bench holding [`SESSIONS`] transcripts, and their keys in placement order.
fn stocked() -> (Bench, Vec<PathBuf>) {
    let bench = bench();
    let keys: Vec<PathBuf> = (0..SESSIONS)
        .map(|n| bench.place(PROJECT, n, "session-large-record.jsonl"))
        .collect();
    bench.ingest();
    (bench, keys)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one JSON document a `--json` run wrote to stdout, and nothing beside it.
fn document(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_else(|| panic!("stdout was empty"));
    assert_eq!(
        lines.next(),
        None,
        "stdout carried more than the document: {text:?}"
    );
    serde_json::from_str(first)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {text:?}"))
}

// ---------------------------------------------------------------------------
// `compact` (RET-04, AC4)
// ---------------------------------------------------------------------------

/// AC4's first half: after a delete pass, the number the product itself reports
/// for the store's size is strictly smaller once `compact` has run.
///
/// Read through `verbatim status --json` on both sides rather than off the
/// filesystem, because `size_bytes` is what AC4 names and it sums three files -
/// a bare `VACUUM` halves `verbatim.db` while moving the freed pages into the
/// WAL, and this assertion is what fails if the truncating checkpoint is
/// dropped (D-08).
///
/// The archive digest on both sides is the other half: `compact` rewrites the
/// database file and must not change one archived byte. It reclaims space
/// retention already freed and has no opinion of its own about what is kept.
#[test]
fn compact_shrinks_the_footprint_status_reports_and_moves_no_archived_byte() {
    let (bench, keys) = stocked();

    let doomed: Vec<&Path> = keys[..(SESSIONS / 2) as usize]
        .iter()
        .map(PathBuf::as_path)
        .collect();
    bench.delete(&doomed);

    let before = bench.size_bytes();
    let digest_before = testkit::archive_digest(&bench.conn());

    let out = bench.run(&["compact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let after = bench.size_bytes();
    assert!(
        after < before,
        "compact left the store the same size or larger: {before} -> {after}"
    );
    assert_eq!(
        testkit::archive_digest(&bench.conn()),
        digest_before,
        "compact changed the archive"
    );

    // And the sessions that survived the delete are still all there.
    let out = bench.run(&["sessions", "--project", "*", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let listed = document(&out)["data"]["sessions"]
        .as_array()
        .expect("sessions is an array")
        .len();
    assert_eq!(listed, (SESSIONS - SESSIONS / 2) as usize);
}

/// AC4, the mechanism: the write-ahead log is empty the moment `compact`
/// returns.
///
/// Measured 2026-08-22 on a synthetic store: a bare `VACUUM` left `verbatim.db`
/// at 7,540,736 bytes and the WAL at 7,584,952, a total UP from 15.06 MB. A
/// `wal` component of zero in the document is the assertion that the truncating
/// checkpoint really ran, and it fails loudly rather than through an
/// intermittently-passing size comparison.
#[test]
fn compact_json_reports_an_empty_write_ahead_log_and_the_bytes_it_reclaimed() {
    let (bench, keys) = stocked();
    let doomed: Vec<&Path> = keys[..(SESSIONS / 2) as usize]
        .iter()
        .map(PathBuf::as_path)
        .collect();
    bench.delete(&doomed);

    let out = bench.run(&["compact", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stderr(&out), "", "the document already carries the counts");

    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["data"]["after"]["wal"], 0, "{value}");

    let before = value["data"]["before"]["total"].as_i64().unwrap();
    let after = value["data"]["after"]["total"].as_i64().unwrap();
    assert!(after < before, "{value}");
    assert_eq!(value["data"]["reclaimed_bytes"], before - after, "{value}");
}

/// AC4 under contention: a compaction that did not happen is an operational
/// failure that says so, not an empty answer.
///
/// Exit 1 and not 0, unlike a contended `ingest`: this command was asked for
/// explicitly, so silently not doing it would be the wrong answer. And nothing
/// moved - a plain `VACUUM` racing a pass's write transaction is exactly the
/// raw "database is locked" the lock exists to turn into a sentence.
#[test]
fn compact_refuses_while_an_ingest_holds_the_lock_and_changes_nothing() {
    let (bench, keys) = stocked();
    let doomed: Vec<&Path> = keys[..(SESSIONS / 2) as usize]
        .iter()
        .map(PathBuf::as_path)
        .collect();
    bench.delete(&doomed);

    let before = bench.size_bytes();
    let guard = match verbatim_core::ingest::lock::try_acquire(&bench.data_dir).unwrap() {
        verbatim_core::ingest::Attempt::Held => panic!("the lock should have been free"),
        verbatim_core::ingest::Attempt::Acquired(guard) => guard,
    };

    let out = bench.run(&["compact"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let message = stderr(&out);
    assert!(
        message.contains("an ingest is running") && message.contains("LOCK"),
        "the refusal must name what is holding it: {message}"
    );
    assert!(
        !message.contains("database is locked"),
        "compact waited on the SQLite busy handler instead of the lock: {message}"
    );

    let out = bench.run(&["compact", "--json"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["data"]["reclaimed_bytes"], 0, "{value}");

    drop(guard);
    assert_eq!(
        bench.size_bytes(),
        before,
        "a refused compaction moved the store"
    );

    // And it works once the lock is free.
    let out = bench.run(&["compact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(bench.size_bytes() < before);
}

/// A machine that has never ingested: an answer and exit 0, and not one byte
/// created where the store would go.
///
/// `lock::try_acquire` creates the directory it locks in, so the store's
/// absence has to be answered before the lock is reached. A `compact` that left
/// a data directory behind would be the read-path bug D-10 exists to prevent,
/// arriving through a write command.
#[test]
fn compact_against_no_store_is_an_answer_and_creates_nothing() {
    let bench = bench();

    let out = bench.run(&["compact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("no verbatim store"),
        "{}",
        stderr(&out)
    );

    let out = bench.run(&["compact", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["data"]["before"]["total"], 0, "{value}");
    assert_eq!(value["data"]["reclaimed_bytes"], 0, "{value}");

    assert!(
        !bench.data_dir.exists(),
        "compact created the data directory it was asked about"
    );
}
