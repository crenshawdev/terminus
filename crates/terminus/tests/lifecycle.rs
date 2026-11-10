//! The lifecycle commands at the process boundary: `compact`, `usage`,
//! `export`.
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
use terminus_core::retention::{self, Selection};
use terminus_core::store::{ARCHIVE_FORMAT, DB_FILE_NAME};
use terminus_core::{testkit, Store};

const PROJECT: &str = "-data-projects-cadence";

/// How many transcripts a stocked bench holds.
///
/// Enough that deleting half frees whole SQLite pages rather than slack inside
/// pages already in use - a compaction that reclaims nothing measurable would
/// make the "strictly smaller" assertion depend on the page size.
const SESSIONS: u8 = 8;

/// The random hex in an incompressible session. It compresses to about 16 KiB
/// at zstd level 3.
const INCOMPRESSIBLE_CHARS: usize = 32 * 1024;

/// The key a transcript is archived under: its path below the project folder.
fn key_of(path: &Path) -> String {
    terminus_core::ingest::path_key(path).unwrap()
}

/// The same isolation `cli.rs`, `status.rs` and `retention.rs` use, and for the
/// same reason: a spawn that set only `TERMINUS_DATA_DIR` would resolve the
/// developer's real config and walk the live `~/.claude` tree.
struct Bench {
    _dir: tempfile::TempDir,
    /// The temporary root, which is where an export destination goes.
    root: PathBuf,
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
        root,
        data_dir,
        config_dir,
        claude_dir,
    }
}

impl Bench {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_terminus"))
            .args(args)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .output()
            .expect("spawn terminus")
    }

    fn place(&self, project: &str, n: u8, fixture: &str) -> PathBuf {
        let name = format!("{n:08x}-1111-4111-8111-111111111111.jsonl");
        let dest = self.claude_dir.join("projects").join(project).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest.canonicalize().unwrap()
    }

    /// Place a one-record transcript of random hex. Its block can't compress
    /// below a page, so it spills onto overflow pages a delete always frees.
    fn place_incompressible(&self, project: &str, n: u8) -> PathBuf {
        let fixture = testkit::fixture_bytes("session-large-record.jsonl");
        let first = fixture.split(|b| *b == b'\n').next().unwrap();
        let mut record: serde_json::Value = serde_json::from_slice(first).unwrap();
        let mut rng = testkit::Rng(u64::from(n));
        let text: String = (0..INCOMPRESSIBLE_CHARS / 16)
            .map(|_| format!("{:016x}", rng.next_u64()))
            .collect();
        record["message"]["content"][0]["text"] = text.into();

        let name = format!("{n:08x}-1111-4111-8111-111111111111.jsonl");
        let dest = self.claude_dir.join("projects").join(project).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, format!("{record}\n")).unwrap();
        dest.canonicalize().unwrap()
    }

    /// Write `terminus.toml` into the config directory this bench points the
    /// binary at.
    fn config(&self, text: &str) {
        std::fs::write(self.config_dir.join("terminus.toml"), text).unwrap();
    }

    fn ingest(&self) {
        let out = self.run(&["ingest"]);
        assert!(out.status.success(), "ingest: {}", stderr(&out));
    }

    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    /// The footprint `terminus status` reports: `terminus.db` and its two WAL
    /// sidecars, read back through the binary rather than measured here.
    fn size_bytes(&self) -> u64 {
        let out = self.run(&["status", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        document(&out)["data"]["size_bytes"]
            .as_u64()
            .expect("status reports size_bytes")
    }

    /// Evict these sessions through the product's own eviction: the
    /// row and every derived row stay, the archived bytes go.
    fn evict(&self, keys: &[&Path]) {
        let mut store = Store::open(&self.data_dir).unwrap();
        let selection = Selection {
            evict: keys.iter().map(|k| key_of(k)).collect(),
            ..Selection::default()
        };
        let applied = retention::apply(&mut store, &selection);
        assert_eq!(applied.evicted.len(), keys.len(), "{applied:?}");
    }

    /// Delete these sessions through the product's own delete, not
    /// through hand-written SQL: what `compact` reclaims is exactly the pages
    /// `retention::apply` freed, and a test that deleted differently would be
    /// compacting a store no user can produce.
    ///
    /// Each transcript is removed first, because `retention::apply` keeps a
    /// session whose transcript is still on disk (#43). The tests after this
    /// only run `compact`, `sessions`, `status` and `usage`, never another
    /// ingest, so nothing finds the files gone.
    fn delete(&self, keys: &[&Path]) {
        for key in keys {
            std::fs::remove_file(key).unwrap();
        }
        let mut store = Store::open(&self.data_dir).unwrap();
        let selection = Selection {
            delete: keys.iter().map(|k| key_of(k)).collect(),
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
// `compact`
// ---------------------------------------------------------------------------

/// After a delete pass, the number the product itself reports
/// for the store's size is strictly smaller once `compact` has run.
///
/// Read through `terminus status --json` on both sides rather than off the
/// filesystem, because `size_bytes` is the size the product reports and it sums
/// three files - a bare `VACUUM` halves `terminus.db` while moving the freed
/// pages into the WAL, and this assertion is what fails if the truncating
/// checkpoint is dropped.
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

/// How `compact` gets there: the write-ahead log is empty the moment `compact`
/// returns.
///
/// Measured 2026-08-22 on a synthetic store: a bare `VACUUM` left `terminus.db`
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

/// `compact` under contention: a compaction that did not happen is an
/// operational failure that says so, not an empty answer.
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
    let guard = match terminus_core::ingest::lock::try_acquire(&bench.data_dir).unwrap() {
        terminus_core::ingest::Attempt::Held => panic!("the lock should have been free"),
        terminus_core::ingest::Attempt::Acquired(guard) => guard,
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
/// a data directory behind would be the bug the read path refuses - a store
/// created by asking about one - arriving through a write command.
#[test]
fn compact_against_no_store_is_an_answer_and_creates_nothing() {
    let bench = bench();

    let out = bench.run(&["compact"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("no terminus store"),
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
// `usage`
// ---------------------------------------------------------------------------

/// Every session's archived bytes as SQLite reports them, over the whole store.
///
/// The reconciliation target `usage` documents, computed the one way that is
/// not a second implementation of the report: one sum over what is stored, in
/// testkit and never through `usage`'s own code path.
fn archived_bytes(bench: &Bench) -> i64 {
    testkit::stored_total(&bench.conn())
}

/// The bytes in one of the two archive tables, added up by the reader.
fn table_total(rows: &serde_json::Value) -> i64 {
    rows.as_array()
        .unwrap_or_else(|| panic!("an archive table is an array: {rows}"))
        .iter()
        .map(|row| row["bytes"].as_i64().expect("bytes is a number"))
        .sum()
}

/// Both archive tables reconcile to the sum of every block
/// row's length, the footprint reconciles to the database file,
/// and the two nulls the live store actually has are inside those totals rather
/// than dropped out of them.
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
///
/// The two deleted are incompressible ones placed for this. Deleting two
/// stocked sessions freed a page only on a short temp path, because the path is
/// in every row and its length decides how the small rows pack.
#[test]
fn both_archive_tables_and_the_footprint_each_reconcile_to_their_own_total() {
    let (bench, keys) = stocked();
    let extra: Vec<PathBuf> = (SESSIONS..SESSIONS + 2)
        .map(|n| bench.place_incompressible(PROJECT, n))
        .collect();
    bench.ingest();
    let freed: Vec<&Path> = extra.iter().map(PathBuf::as_path).collect();
    bench.delete(&freed);
    {
        let conn = bench.conn();
        conn.execute(
            "UPDATE session_meta SET project = NULL WHERE session_key = ?1",
            [key_of(&keys[0])],
        )
        .unwrap();
        conn.execute(
            "UPDATE session_meta SET first_turn_at = NULL WHERE session_key = ?1",
            [key_of(&keys[1])],
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

    assert_eq!(data["sessions"], SESSIONS as i64, "{value}");
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

/// A project excluded after its sessions were archived is in
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
            rusqlite::params![key_of(&keys[0]), HIDDEN],
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
            .is_some_and(|r| r.contains("no terminus store")),
        "{value}"
    );
    assert_eq!(value["data"]["archive_bytes"], 0, "{value}");
    assert_eq!(value["data"]["file_bytes"], 0, "{value}");

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory"
    );
}

// ---------------------------------------------------------------------------
// `export`
// ---------------------------------------------------------------------------

/// The manifest one export wrote.
fn manifest(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("manifest.json"))
        .unwrap_or_else(|e| panic!("no manifest at {}: {e}", dir.display()));
    serde_json::from_str(&text).expect("the manifest is JSON")
}

/// Every `.jsonl` file in an export directory, by name.
fn exported_files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".jsonl"))
        .collect();
    out.sort();
    out
}

/// The exported files ARE the archived streams, byte for byte, and the
/// manifest accounts for every one of them and says in words what they are.
///
/// Two comparisons. Against the stream testkit reads out of the block rows,
/// because that is what export claims to write. And, for a session captured
/// under `full`, against the transcript it came from up to its watermark, the
/// end of its last complete record: that is the end-to-end claim
/// the block rows have to keep.
#[test]
fn an_export_is_the_archived_streams_byte_for_byte_with_a_manifest_over_them() {
    let (bench, keys) = stocked();
    let dest = bench.root.join("export");

    let out = bench.run(&["export", dest.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stderr(&out), "", "the document already carries the notice");
    let value = document(&out);
    assert_eq!(value["data"]["sessions"], SESSIONS as i64, "{value}");
    assert!(
        value["data"]["notice"]
            .as_str()
            .is_some_and(|n| n.contains("unredacted")),
        "the document must say what it wrote: {value}"
    );

    // Every session's file holds exactly what the archive holds.
    let conn = bench.conn();
    for key in &keys {
        let expected = testkit::stream(&conn, &key_of(key)).unwrap();
        let named = manifest(&dest)["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["session_key"] == key_of(key).as_str())
            .unwrap_or_else(|| panic!("{} is not in the manifest", key.display()))
            .clone();
        let written = std::fs::read(dest.join(named["file"].as_str().unwrap())).unwrap();
        assert_eq!(
            written,
            expected,
            "{} was not written byte for byte",
            key.display()
        );
        assert_eq!(named["bytes"], expected.len() as i64, "{named}");

        let (mode, watermark): (Option<String>, i64) = conn
            .query_row(
                "SELECT m.capture_mode, w.byte_offset
                   FROM session_meta m JOIN watermarks w ON w.transcript_path = m.session_key
                  WHERE m.session_key = ?1",
                [key_of(key)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            mode.as_deref().unwrap_or("full"),
            "full",
            "{}",
            key.display()
        );
        let transcript = std::fs::read(key).unwrap();
        assert_eq!(
            written,
            transcript[..watermark as usize],
            "{} is not its transcript up to the watermark",
            key.display()
        );
    }

    // The manifest accounts for the directory and for nothing that is not in it.
    let doc = manifest(&dest);
    assert_eq!(doc["sessions"], SESSIONS as i64, "{doc}");
    assert_eq!(
        doc["files"].as_array().unwrap().len(),
        exported_files(&dest).len(),
        "the manifest and the directory disagree about how many files there are"
    );
    assert!(
        doc["notice"]
            .as_str()
            .is_some_and(|n| n.contains("unredacted") && n.contains("Not in this export")),
        "the manifest must state what it is and what it is not: {doc}"
    );
    assert_eq!(doc["archive_format"], ARCHIVE_FORMAT, "{doc}");
    assert_eq!(ARCHIVE_FORMAT, 2);
    assert!(doc["first_turn_at"].is_string(), "{doc}");
    assert!(doc["last_turn_at"].is_string(), "{doc}");
    assert!(doc["turns"].as_i64().unwrap() > 0, "{doc}");
}

/// An evicted session is named and empty, never missing.
///
/// It has no block rows, so there is no stream to write, and the choice is
/// between a file with nothing in it and no file at all. A missing file
/// would read as an export that dropped a session; an empty one beside a
/// manifest line saying why is the honest account of what retention did.
#[test]
fn an_evicted_session_is_exported_as_an_empty_file_the_manifest_explains() {
    let (bench, keys) = stocked();
    bench.evict(&[keys[0].as_path()]);
    let dest = bench.root.join("export");

    let out = bench.run(&["export", dest.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(document(&out)["data"]["evicted"], 1, "{}", stdout(&out));

    let doc = manifest(&dest);
    assert_eq!(
        doc["sessions"], SESSIONS as i64,
        "the evicted session vanished"
    );
    let named = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["session_key"] == key_of(&keys[0]).as_str())
        .expect("the evicted session is still in the manifest")
        .clone();
    assert_eq!(named["evicted"], true, "{named}");
    assert_eq!(named["bytes"], 0, "{named}");

    let path = dest.join(named["file"].as_str().unwrap());
    assert!(path.exists(), "the evicted session has no file at all");
    assert_eq!(std::fs::read(&path).unwrap(), Vec::<u8>::new());
}

/// A project excluded after its sessions were archived is in neither
/// the manifest nor the directory.
///
/// Export is the read path where this matters most: the output is a directory a
/// user may hand to someone else, and "exclusion honoured on write and ignored
/// on read" is the incumbent's bug this product exists to not have.
#[test]
fn an_excluded_project_is_in_neither_the_manifest_nor_the_directory() {
    let (bench, keys) = stocked();
    const HIDDEN: &str = "/data/projects/excluded-from-export";

    bench
        .conn()
        .execute(
            "UPDATE session_meta SET project = ?2 WHERE session_key = ?1",
            rusqlite::params![key_of(&keys[0]), HIDDEN],
        )
        .unwrap();
    bench.config(&format!("exclude = [\"{HIDDEN}\"]\n"));
    let dest = bench.root.join("export");

    let out = bench.run(&["export", dest.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(document(&out)["data"]["sessions"], (SESSIONS - 1) as i64);

    let doc = manifest(&dest);
    assert_eq!(exported_files(&dest).len(), (SESSIONS - 1) as usize);
    assert!(
        !doc["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["session_key"] == key_of(&keys[0]).as_str()),
        "the excluded session is in the manifest: {doc}"
    );
    assert!(
        !doc["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == HIDDEN),
        "{doc}"
    );
}

/// A destination that already holds something is refused, and nothing in it
/// moves.
///
/// Both halves matter. Exit non-zero is what a script reads; the untouched
/// bytes are what the user keeps. An export that merged into an older one would
/// leave a manifest describing some of the files beside it and no way to tell
/// which.
#[test]
fn exporting_into_an_occupied_directory_refuses_and_overwrites_nothing() {
    let (bench, _keys) = stocked();
    let dest = bench.root.join("export");

    let out = bench.run(&["export", dest.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let first = manifest(&dest);
    let files = exported_files(&dest);
    let sample = files.first().expect("an export wrote files").clone();
    let sample_bytes = std::fs::read(dest.join(&sample)).unwrap();

    let out = bench.run(&["export", dest.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("already holds an export"),
        "{}",
        stderr(&out)
    );
    assert_eq!(manifest(&dest), first, "the manifest was rewritten");
    assert_eq!(exported_files(&dest), files, "the directory changed");
    assert_eq!(std::fs::read(dest.join(&sample)).unwrap(), sample_bytes);

    // And a directory that holds something that is NOT an export is refused
    // just as hard: this command never overwrites a file it did not write.
    let occupied = bench.root.join("not-an-export");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("notes.txt"), b"mine").unwrap();
    let out = bench.run(&["export", occupied.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(stderr(&out).contains("is not empty"), "{}", stderr(&out));
    assert_eq!(std::fs::read(occupied.join("notes.txt")).unwrap(), b"mine");
    assert!(!occupied.join("manifest.json").exists());
}

/// A machine that has never ingested: an answer, exit 0, and neither a store
/// nor an export directory brought into being by the asking.
#[test]
fn export_against_no_store_creates_neither_a_store_nor_a_directory() {
    let bench = bench();
    let dest = bench.root.join("export");

    let out = bench.run(&["export", dest.to_str().unwrap(), "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value = document(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["data"]["sessions"], 0, "{value}");

    assert!(
        !bench.data_dir.exists(),
        "a read created the data directory"
    );
    assert!(!dest.exists(), "an empty export still made a directory");
}

/// Misuse, not a default: there is no sensible place to put an unredacted copy
/// of every transcript that the user did not name.
#[test]
fn export_without_a_destination_is_misuse() {
    let (bench, _keys) = stocked();

    let out = bench.run(&["export"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("needs a destination"),
        "{}",
        stderr(&out)
    );
}

// ---------------------------------------------------------------------------
// `export --stdout`
// ---------------------------------------------------------------------------

impl Bench {
    fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        use std::io::Write as _;
        use std::process::Stdio;

        let mut child = Command::new(env!("CARGO_BIN_EXE_terminus"))
            .args(args)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn terminus");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().expect("wait for terminus")
    }
}

/// The header lines and the bytes after each, in the order they came.
fn streamed(out: &Output) -> Vec<(serde_json::Value, Vec<u8>)> {
    let mut sessions: Vec<(serde_json::Value, Vec<u8>)> = Vec::new();
    for line in out.stdout.split_inclusive(|b| *b == b'\n') {
        let value: serde_json::Value = serde_json::from_slice(line)
            .unwrap_or_else(|e| panic!("a line is not JSON ({e}): {line:?}"));
        match value.get("terminus") {
            Some(header) => sessions.push((header.clone(), Vec::new())),
            None => sessions
                .last_mut()
                .expect("a transcript line came before any header")
                .1
                .extend_from_slice(line),
        }
    }
    sessions
}

/// An empty stdin is every session, each its archived stream behind a header
/// carrying the watermark those bytes were read under.
#[test]
fn export_stdout_with_no_keys_streams_every_session_behind_its_header() {
    let (bench, keys) = stocked();

    let out = bench.run_with_stdin(&["export", "--stdout"], "");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("unredacted"), "{}", stderr(&out));

    let sessions = streamed(&out);
    assert_eq!(sessions.len(), SESSIONS as usize);
    let conn = bench.conn();
    for key in &keys {
        let key = key_of(key);
        let (header, body) = sessions
            .iter()
            .find(|(h, _)| h["session_key"] == key.as_str())
            .unwrap_or_else(|| panic!("{key} was not streamed"));
        assert_eq!(*body, testkit::stream(&conn, &key).unwrap(), "{key}");
        let (watermark, project): (i64, String) = conn
            .query_row(
                "SELECT w.byte_offset, m.project
                   FROM watermarks w JOIN session_meta m ON m.session_key = w.transcript_path
                  WHERE w.transcript_path = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(header["watermark"], watermark, "{header}");
        assert_eq!(header["project"], project, "{header}");
        assert!(header.get("evicted").is_none(), "{header}");
    }
}

/// Only the keys asked for, in that order. An evicted one is a bare header, and
/// an unknown or excluded one is named on stderr and skipped.
#[test]
fn export_stdout_streams_only_the_keys_on_stdin() {
    let (bench, keys) = stocked();
    const HIDDEN: &str = "/data/projects/excluded-from-export";
    bench.evict(&[keys[0].as_path()]);
    bench
        .conn()
        .execute(
            "UPDATE session_meta SET project = ?2 WHERE session_key = ?1",
            rusqlite::params![key_of(&keys[1]), HIDDEN],
        )
        .unwrap();
    bench.config(&format!("exclude = [\"{HIDDEN}\"]\n"));

    let stdin = format!(
        "{}\n{}\nno-such-session.jsonl\n{}\n",
        key_of(&keys[2]),
        key_of(&keys[0]),
        key_of(&keys[1]),
    );
    let out = bench.run_with_stdin(&["export", "--stdout"], &stdin);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let sessions = streamed(&out);
    let names: Vec<&str> = sessions
        .iter()
        .map(|(h, _)| h["session_key"].as_str().unwrap())
        .collect();
    assert_eq!(names, [key_of(&keys[2]), key_of(&keys[0])]);
    assert_eq!(sessions[1].0["evicted"], true, "{}", sessions[1].0);
    assert!(sessions[1].1.is_empty(), "an evicted session carried lines");

    let err = stderr(&out);
    assert!(err.contains("no-such-session.jsonl: not found"), "{err}");
    assert!(
        err.contains(&format!("{}: not found", key_of(&keys[1]))),
        "{err}"
    );
}

#[test]
fn export_stdout_with_a_directory_or_json_is_misuse() {
    let (bench, _keys) = stocked();
    let dest = bench.root.join("export");

    for args in [
        vec!["export", "--stdout", dest.to_str().unwrap()],
        vec!["export", "--stdout", "--json"],
    ] {
        let out = bench.run_with_stdin(&args, "");
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
        assert_eq!(stdout(&out), "", "{args:?}");
    }
    assert!(!dest.exists());
}
