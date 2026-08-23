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

    /// Write `verbatim.toml` into the config directory this bench points the
    /// binary at.
    fn config(&self, text: &str) {
        std::fs::write(self.config_dir.join("verbatim.toml"), text).unwrap();
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

// ---------------------------------------------------------------------------
// `usage` (RET-05, AC4)
// ---------------------------------------------------------------------------

/// Every session's archived bytes as SQLite reports them, over the whole store.
///
/// The reconciliation target `usage` documents, computed the one way that is
/// not a second implementation of the report: one `sum()` over one column.
fn archived_bytes(bench: &Bench) -> i64 {
    bench
        .conn()
        .query_row(
            "SELECT coalesce(sum(length(blob)), 0) FROM sessions",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// The bytes in one of the two archive tables, added up by the reader.
fn table_total(rows: &serde_json::Value) -> i64 {
    rows.as_array()
        .unwrap_or_else(|| panic!("an archive table is an array: {rows}"))
        .iter()
        .map(|row| row["bytes"].as_i64().expect("bytes is a number"))
        .sum()
}

/// AC4's second half: both archive tables reconcile to
/// `sum(length(sessions.blob))`, the footprint reconciles to the database file,
/// and the two nulls the live store actually has are inside those totals rather
/// than dropped out of them (D-09, D-18).
///
/// The null rows are what make this more than an arithmetic identity. A
/// `GROUP BY` that silently discards them still produces two tables that look
/// right and a total that no longer adds up, and the live store has exactly one
/// session with no `project` and one with no `first_turn_at`.
///
/// Two sessions are deleted first so the freelist is not empty. On a store
/// nothing has ever deleted from, `freelist_count` is zero and the footprint
/// reconciles whether or not the freelist has a row at all - which would make
/// this pass for the wrong reason and leave `compact`'s own number unproven.
#[test]
fn both_archive_tables_and_the_footprint_each_reconcile_to_their_own_total() {
    let (bench, keys) = stocked();
    let freed: Vec<&Path> = keys[(SESSIONS - 2) as usize..]
        .iter()
        .map(PathBuf::as_path)
        .collect();
    bench.delete(&freed);
    {
        let conn = bench.conn();
        conn.execute(
            "UPDATE session_meta SET project = NULL WHERE session_key = ?1",
            [keys[0].to_string_lossy()],
        )
        .unwrap();
        conn.execute(
            "UPDATE session_meta SET first_turn_at = NULL WHERE session_key = ?1",
            [keys[1].to_string_lossy()],
        )
        .unwrap();
    }
    let expected = archived_bytes(&bench);
    assert!(
        expected > 0,
        "this test needs archived bytes to account for"
    );

    let out = bench.run(&["usage", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stderr(&out), "", "the document already carries the report");
    let value = document(&out);
    let data = &value["data"];

    assert_eq!(data["sessions"], (SESSIONS - 2) as i64, "{value}");
    assert_eq!(data["archive_bytes"], expected, "{value}");
    assert_eq!(
        table_total(&data["by_project"]),
        expected,
        "the per-project bytes do not add up to the archive: {value}"
    );
    assert_eq!(
        table_total(&data["by_month"]),
        expected,
        "the per-month bytes do not add up to the archive: {value}"
    );

    // The two nulls each have a bucket of their own, and their bytes are in it.
    let unnamed = |rows: &serde_json::Value, field: &str| -> serde_json::Value {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row[field].is_null())
            .unwrap_or_else(|| panic!("no bucket for the null {field}: {rows}"))
            .clone()
    };
    for (table, field) in [("by_project", "project"), ("by_month", "month")] {
        let bucket = unnamed(&data[table], field);
        assert_eq!(bucket["sessions"], 1, "{table}: {bucket}");
        assert!(
            bucket["bytes"].as_i64().unwrap() > 0,
            "the null bucket carries no bytes, so it is not in the total: {bucket}"
        );
    }

    // The footprint is the other table, and it reconciles to the other total.
    let conn = bench.conn();
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(data["file_bytes"], page_count * page_size, "{value}");
    let footprint: i64 = data["footprint"]
        .as_array()
        .expect("the footprint is an array")
        .iter()
        .map(|row| row["bytes"].as_i64().unwrap())
        .sum();
    assert_eq!(
        footprint,
        page_count * page_size,
        "the footprint rows do not add up to the database file: {value}"
    );
    let freelist = data["footprint"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "(free pages)")
        .unwrap_or_else(|| panic!("the freelist has no row, so `compact` has no number: {value}"))
        .clone();
    assert!(
        freelist["bytes"].as_i64().unwrap() > 0,
        "the delete freed no pages, so this store cannot prove the freelist row \
         is inside the total: {value}"
    );

    // And the two tables really are independent: the archive is a fraction of
    // the file, never the same number.
    assert!(
        data["archive_bytes"].as_i64().unwrap() < data["file_bytes"].as_i64().unwrap(),
        "{value}"
    );
}

/// AC4 and ING-08: a project excluded after its sessions were archived is in
/// neither archive total.
///
/// Exclusion is retroactive and binds every read path alike, so a report of
/// what the store holds is not the one place a user's "never look at this" gets
/// an exception. The month total is the half that catches a project filter
/// written in only one of the two groupings.
#[test]
fn an_excluded_project_is_in_neither_archive_total() {
    let (bench, keys) = stocked();
    const HIDDEN: &str = "/data/projects/excluded-from-usage";

    bench
        .conn()
        .execute(
            "UPDATE session_meta SET project = ?2 WHERE session_key = ?1",
            rusqlite::params![keys[0].to_string_lossy(), HIDDEN],
        )
        .unwrap();
    let everything = archived_bytes(&bench);
    bench.config(&format!("exclude = [\"{HIDDEN}\"]\n"));

    let out = bench.run(&["usage", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    let data = &value["data"];

    assert_eq!(data["sessions"], (SESSIONS - 1) as i64, "{value}");
    let visible = data["archive_bytes"].as_i64().unwrap();
    assert!(
        visible < everything,
        "the excluded session's bytes are still in the total: {visible} of {everything}"
    );
    assert_eq!(table_total(&data["by_project"]), visible, "{value}");
    assert_eq!(table_total(&data["by_month"]), visible, "{value}");
    assert!(
        !data["by_project"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["project"] == HIDDEN),
        "the excluded project is named in the report: {value}"
    );
}

/// A machine that has never ingested: an answer, exit 0, and no store created
/// by the asking.
#[test]
fn usage_against_no_store_is_a_reason_and_creates_nothing() {
    let bench = bench();

    let out = bench.run(&["usage", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert!(
        value["reason"]
            .as_str()
            .is_some_and(|r| r.contains("no verbatim store")),
        "{value}"
    );
    assert_eq!(value["data"]["archive_bytes"], 0, "{value}");
    assert_eq!(value["data"]["file_bytes"], 0, "{value}");

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory"
    );
}
