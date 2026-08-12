//! AC4: a compaction boundary appearing under an already-archived session.
//!
//! The shape of a real compaction is what makes this worth a test file of its
//! own. Compaction is **append-only**: Claude Code writes the boundary into the
//! same transcript, at the end, under the same session id
//! (`DESIGN-BRIEF.md:114`). So the case is not "ingest a file with a boundary in
//! it" - it is "a session that has already been archived, whose blob and turn
//! rows are already committed, grows one more record", and the archive must
//! come out of that with the boundary recorded and *nothing that was already
//! there* disturbed.
//!
//! Byte-identity is asserted at the block level rather than on the blob as a
//! whole, because those are different claims. A blob's header and block table
//! are rewritten on every append (`blob::writer::finish_parts` moves every
//! block's absolute offset when the table grows), so the blob's bytes are
//! *expected* to differ. What must not differ is a completed block: those are
//! immutable once committed (D-07) and copied across untouched.

#![cfg(feature = "testkit")]

use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use verbatim_core::store::{schema, Store, DB_FILE_NAME};
use verbatim_core::{blob, ingest, reindex, testkit};

/// A transcript large enough to fill at least one 64 KB block, so
/// "every completed block is byte-identical" is a claim about something.
/// `session-large-record.jsonl` holds a 204,800-byte record, which is four
/// blocks: three complete plus a partial one.
const BIG_FIXTURE: &str = "session-large-record.jsonl";

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    transcript: PathBuf,
    session_key: String,
}

fn bench(fixture: &str) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let transcript = testkit::copy_fixture_into(fixture, &work);

    let session_key = match ingest::run(&data_dir, &transcript).unwrap() {
        ingest::Outcome::Committed(pass) => pass.session_key,
        other => panic!("{fixture}: {other:?}"),
    };

    Bench {
        _dir: dir,
        data_dir,
        transcript,
        session_key,
    }
}

impl Bench {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn ingest(&self) -> ingest::Outcome {
        ingest::run(&self.data_dir, &self.transcript).unwrap()
    }

    /// Append one whole record, the way a compaction does.
    fn append_line(&self, line: &[u8]) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .unwrap();
        file.write_all(line).unwrap();
        file.write_all(b"\n").unwrap();
    }
}

/// The compressed bytes of every **completed** block of a session's blob.
///
/// The trailing partial block is excluded on purpose: it is the one block an
/// append is allowed to rewrite (D-07), and including it would make the
/// assertion demand something the format explicitly does not promise.
fn completed_blocks(conn: &Connection, session_key: &str) -> Vec<Vec<u8>> {
    let raw = raw_blob(conn, session_key);
    let header = blob::BlobHeader::parse(&raw).unwrap();
    let complete = (header.uncompressed_len / u64::from(header.block_size)) as usize;
    header.blocks[..complete]
        .iter()
        .map(|entry| {
            let from = entry.compressed_offset as usize;
            raw[from..from + entry.compressed_len as usize].to_vec()
        })
        .collect()
}

fn raw_blob(conn: &Connection, session_key: &str) -> Vec<u8> {
    conn.query_row(
        "SELECT blob FROM sessions WHERE session_key = ?1",
        [session_key],
        |r| r.get(0),
    )
    .unwrap()
}

type TurnRowValues = (i64, i64, String, Option<String>, i64, i64);

fn turn_rows(conn: &Connection) -> Vec<TurnRowValues> {
    conn.prepare(
        "SELECT id, turn_seq, record_type, uuid, stream_offset, byte_len
         FROM turns ORDER BY id",
    )
    .unwrap()
    .query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// Every boundary row in the store, as `(turn_id, metadata bytes)`.
fn boundaries(conn: &Connection) -> Vec<(i64, Vec<u8>)> {
    conn.prepare("SELECT turn_id, metadata FROM compaction_boundaries ORDER BY turn_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The `compactMetadata` of a boundary line, read independently of the parser
/// under test: `serde_json` deserializes the whole line and hands back that
/// field's value, so an equality against it cannot be satisfied by the
/// extraction agreeing with itself.
fn expected_metadata(line: &[u8]) -> serde_json::Value {
    let record: serde_json::Value = serde_json::from_slice(line).unwrap();
    record["compactMetadata"].clone()
}

fn drop_derived(conn: &Connection) {
    // Children first: the bundled SQLite enforces foreign keys, so
    // `compaction_boundaries` has to go before `turns`.
    for table in schema::DERIVED_TABLES.iter().rev() {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))
            .unwrap();
    }
}

/// AC4 in full.
///
/// Ingest a transcript, remember every turn row and the exact bytes of every
/// completed blob block, append a real boundary record, re-run ingest: one
/// boundary row carrying that record's `compactMetadata` bytes verbatim, every
/// pre-existing turn row unchanged, every completed block byte-identical.
#[test]
fn appending_a_boundary_records_it_and_disturbs_nothing_already_archived() {
    let bench = bench(BIG_FIXTURE);
    let conn = bench.conn();

    let turns_before = turn_rows(&conn);
    let blocks_before = completed_blocks(&conn, &bench.session_key);
    let stream_before = blob::read_all(&raw_blob(&conn, &bench.session_key)).unwrap();
    assert!(
        !turns_before.is_empty() && blocks_before.len() >= 3,
        "the fixture must archive turns and fill several blocks: {} turns, {} blocks",
        turns_before.len(),
        blocks_before.len()
    );
    assert!(
        boundaries(&conn).is_empty(),
        "the fixture carries no boundary before one is appended"
    );

    let line = testkit::boundary_line();
    bench.append_line(&line);
    match bench.ingest() {
        ingest::Outcome::Committed(_) => {}
        other => panic!("the appended record must commit: {other:?}"),
    }

    // One boundary row, and its bytes are the appended record's own.
    let rows = boundaries(&conn);
    assert_eq!(rows.len(), 1, "one boundary appeared, exactly once");
    let (turn_id, metadata) = &rows[0];
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(metadata).unwrap(),
        expected_metadata(&line),
        "the stored metadata is not the record's compactMetadata"
    );
    assert!(
        line.windows(metadata.len()).any(|w| w == metadata),
        "the stored metadata is not a byte slice of the record, so it was re-serialized"
    );

    // It is keyed on the turn that IS the boundary: reading that turn out of
    // the blob returns the record that was appended (D-21).
    let (bytes, _) = testkit::read_turn(&conn, *turn_id);
    assert_eq!(bytes, line, "the boundary row names the wrong turn");

    // Nothing the first pass wrote moved.
    let turns_after = turn_rows(&conn);
    assert_eq!(
        turns_after.len(),
        turns_before.len() + 1,
        "the boundary is one new turn and no more"
    );
    assert_eq!(
        &turns_after[..turns_before.len()],
        &turns_before[..],
        "an already-archived turn row changed"
    );

    let blocks_after = completed_blocks(&conn, &bench.session_key);
    assert!(blocks_after.len() >= blocks_before.len());
    assert_eq!(
        &blocks_after[..blocks_before.len()],
        &blocks_before[..],
        "a completed blob block was rewritten"
    );

    // And the stream the archive holds still starts with exactly what it held.
    let stream_after = blob::read_all(&raw_blob(&conn, &bench.session_key)).unwrap();
    assert_eq!(&stream_after[..stream_before.len()], &stream_before[..]);
    assert_eq!(
        &stream_after[stream_before.len()..],
        &[line.as_slice(), b"\n"].concat()[..]
    );
}

/// The boundary row is derived, so a rebuild that drops every derived table
/// must reproduce it from the blob alone - same turn id, same bytes.
///
/// This is the property `reindex` would silently break by forgetting to carry
/// the parser's two new fields into its own `TurnRow`: every count would still
/// match and every boundary would be gone.
#[test]
fn a_rebuild_reproduces_the_boundary_row_from_the_blob_alone() {
    let bench = bench(BIG_FIXTURE);
    let line = testkit::boundary_line();
    bench.append_line(&line);
    bench.ingest();

    let conn = bench.conn();
    let before = boundaries(&conn);
    assert_eq!(before.len(), 1);

    drop_derived(&conn);
    drop(conn);
    let mut store = Store::open(&bench.data_dir).unwrap();
    let rebuilt = reindex::reindex(&mut store).unwrap();
    drop(store);
    assert_eq!(rebuilt.sessions, 1);

    let conn = bench.conn();
    assert_eq!(
        boundaries(&conn),
        before,
        "the rebuild did not reproduce the boundary row"
    );

    // Twice, because the seam clears before it writes: a second rebuild must
    // not double the row or drop it.
    let mut store = Store::open(&bench.data_dir).unwrap();
    reindex::reindex(&mut store).unwrap();
    drop(store);
    assert_eq!(boundaries(&bench.conn()), before);
}

/// Only the boundary turn gets a row. Every other turn in the same session -
/// and every turn of a session that never compacted - has none.
#[test]
fn no_ordinary_turn_gets_a_boundary_row() {
    let bench = bench(BIG_FIXTURE);
    let conn = bench.conn();
    assert!(boundaries(&conn).is_empty());
    assert!(!turn_rows(&conn).is_empty());

    bench.append_line(&testkit::boundary_line());
    bench.ingest();

    let rows = boundaries(&conn);
    assert_eq!(rows.len(), 1);
    let boundary_id = rows[0].0;
    let others: Vec<i64> = turn_rows(&conn)
        .into_iter()
        .map(|row| row.0)
        .filter(|id| *id != boundary_id)
        .collect();
    assert!(
        !others.is_empty(),
        "there must be turns to be negative about"
    );
}

/// The whole fixture, ingested in one pass rather than appended: the boundary
/// lands on the same turn with the same bytes either way. A boundary that only
/// appeared on the append path would leave every backfilled session without one.
#[test]
fn ingesting_a_compacted_transcript_whole_records_the_same_boundary() {
    let appended = bench(BIG_FIXTURE);
    let line = testkit::boundary_line();
    appended.append_line(&line);
    appended.ingest();
    let appended_metadata = boundaries(&appended.conn())[0].1.clone();

    let whole = bench(testkit::COMPACTED_FIXTURE);
    let conn = whole.conn();
    let rows = boundaries(&conn);
    assert_eq!(rows.len(), 1, "the whole-file path found no boundary");
    assert_eq!(rows[0].1, appended_metadata, "two paths, two answers");

    // The boundary is the fixture's last turn, and reading it back gives the
    // record verbatim.
    let (bytes, _) = testkit::read_turn(&conn, rows[0].0);
    assert_eq!(bytes, line);
    let last = turn_rows(&conn).last().unwrap().0;
    assert_eq!(last, rows[0].0, "the boundary is the last turn of the file");
}

/// A session with no boundary contributes no row, whatever else it holds. The
/// control for every assertion above.
#[test]
fn a_session_that_never_compacted_has_no_boundary_row() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();

    for fixture in testkit::TRANSCRIPT_FIXTURES {
        if *fixture == testkit::COMPACTED_FIXTURE {
            continue;
        }
        let path: PathBuf = testkit::copy_fixture_into(fixture, &work);
        assert!(matches!(
            ingest::run(&data_dir, &path).unwrap(),
            ingest::Outcome::Committed(_)
        ));
    }

    let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
    assert!(!turn_rows(&conn).is_empty());
    assert!(boundaries(&conn).is_empty());
}

/// Not a compaction test: the helper other tests lean on has to be right, so
/// the "completed blocks" split is checked against the format's own arithmetic
/// once, here, rather than assumed at five call sites.
#[test]
fn the_completed_block_helper_excludes_only_the_partial_tail() {
    let bench = bench(BIG_FIXTURE);
    let conn = bench.conn();
    let raw = raw_blob(&conn, &bench.session_key);
    let header = blob::BlobHeader::parse(&raw).unwrap();

    let complete = completed_blocks(&conn, &bench.session_key).len();
    assert_eq!(
        header.blocks.len(),
        complete
            + usize::from(
                !header
                    .uncompressed_len
                    .is_multiple_of(u64::from(header.block_size)),
            ),
        "the helper's split does not match the block table"
    );
    assert!(
        complete > 0,
        "the fixture must fill at least one whole block"
    );
    assert_eq!(Path::new(&bench.session_key), bench.transcript);
}
