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
//!
//! The second half of the file is D-13, which is the same premise inverted: the
//! file underneath an archived session got *shorter*. Both belong here because
//! both are the live tree changing a session the archive already holds, and the
//! two answers are opposite by design - the archive follows a file that grew
//! and refuses a file that shrank.

#![cfg(feature = "testkit")]

use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use terminus_core::config::Config;
use terminus_core::ingest::pass;
use terminus_core::store::{schema, Store, DB_FILE_NAME};
use terminus_core::{blob, ingest, reindex, testkit, verify};

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

// ---------------------------------------------------------------------------
// D-13: the other thing a live tree does to an already-archived session - the
// file underneath it gets *shorter*.
//
// The same premise as AC4 above, inverted. A compaction appends, so an archived
// session's file growing is normal and the archive follows it. A file that
// shrank is not that session's file any more, and there is no way to tell which
// of its bytes are still the ones the blob holds. So the pass refuses, and the
// refusal is the feature: re-reading from offset 0 would overwrite archived
// bytes on the strength of an append-only property observed over one 25-day
// corpus rather than documented anywhere.
//
// What has to be provable is that the refusal is *loud*. A skip that only
// reaches a `runs` row is a skip nobody sees, so the pass flags the session and
// `terminus verify` reports it - and stops reporting it once the file is whole.
// ---------------------------------------------------------------------------

/// A tree the *pass* walks, rather than the single-file entry point.
///
/// D-13's flag is the pass's to set: `ingest::run` names one file and returns
/// that file's error to the caller who asked about it. Two transcripts, so
/// "names that session and no other" has something to be false about.
struct Tree {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    claude_dir: PathBuf,
    /// The transcript the tests damage.
    subject: PathBuf,
    /// Its original bytes, for putting back.
    original: Vec<u8>,
    /// The transcript they leave alone.
    bystander: PathBuf,
}

const PROJECT: &str = "-data-code-verbatim";

fn tree() -> Tree {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let claude_dir = dir.path().join("claude");
    let project = claude_dir.join("projects").join(PROJECT);
    std::fs::create_dir_all(&project).unwrap();

    let subject = place(
        &project,
        "11111111-1111-4111-8111-111111111111.jsonl",
        BIG_FIXTURE,
    );
    let bystander = place(
        &project,
        "22222222-2222-4222-8222-222222222222.jsonl",
        "session-basic.jsonl",
    );
    let original = std::fs::read(&subject).unwrap();

    Tree {
        _dir: dir,
        data_dir,
        claude_dir,
        subject,
        original,
        bystander,
    }
}

fn place(project: &Path, name: &str, fixture: &str) -> PathBuf {
    let dest = project.join(name);
    std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
    dest.canonicalize().unwrap()
}

impl Tree {
    fn conn(&self) -> Connection {
        Connection::open(self.data_dir.join(DB_FILE_NAME)).unwrap()
    }

    fn pass(&self) -> pass::Summary {
        match pass::run_with(
            &self.data_dir,
            &Config::from_parts(vec![self.claude_dir.clone()], Vec::new()),
        )
        .unwrap()
        {
            pass::PassOutcome::Ran(summary) => summary,
            pass::PassOutcome::LockHeld => panic!("nothing else holds the lock"),
        }
    }

    fn key(&self, path: &Path) -> String {
        path.to_str().unwrap().to_owned()
    }

    /// Everything about a session that a refused pass must not have moved.
    fn archived(&self, path: &Path) -> (Vec<u8>, Vec<u8>, i64, Vec<TurnRowValues>) {
        let conn = self.conn();
        let key = self.key(path);
        let (blob, checksum): (Vec<u8>, Vec<u8>) = conn
            .query_row(
                "SELECT s.blob, m.checksum FROM sessions s
                 JOIN session_meta m USING (session_key) WHERE s.session_key = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let watermark: i64 = conn
            .query_row(
                "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        (blob, checksum, watermark, turn_rows(&conn))
    }

    fn diverged(&self, path: &Path) -> bool {
        self.conn()
            .query_row(
                "SELECT coalesce(transcript_diverged, 0) <> 0
                 FROM session_meta WHERE session_key = ?1",
                [self.key(path)],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// Every session key the store has flagged.
    fn flagged(&self) -> Vec<String> {
        self.conn()
            .prepare(
                "SELECT session_key FROM session_meta
                 WHERE transcript_diverged IS NOT NULL ORDER BY session_key",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The `error` text of the most recent `runs` row.
    fn last_run_error(&self) -> Option<String> {
        self.conn()
            .query_row("SELECT error FROM runs ORDER BY id DESC LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn verify(&self) -> verify::Report {
        let store = Store::open(&self.data_dir).unwrap();
        verify::verify(&store).unwrap()
    }

    /// Cut the transcript back to half its length, below its watermark.
    fn truncate(&self) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&self.subject)
            .unwrap()
            .set_len(self.original.len() as u64 / 2)
            .unwrap();
    }

    fn restore(&self) {
        std::fs::write(&self.subject, &self.original).unwrap();
    }
}

/// D-13 end to end. A shortened transcript is skipped, the archive is left
/// exactly as it was, and the session is flagged so `verify` says so.
#[test]
fn a_transcript_shorter_than_its_watermark_is_skipped_and_flagged() {
    let tree = tree();
    let first = tree.pass();
    assert_eq!(first.files_committed, 2, "{first:?}");
    assert!(first.failures.is_empty(), "{:?}", first.failures);
    assert!(tree.flagged().is_empty(), "nothing is flagged yet");

    let before = tree.archived(&tree.subject);
    let bystander_before = tree.archived(&tree.bystander);
    tree.truncate();

    let second = tree.pass();
    assert_eq!(
        second.files_walked, 2,
        "the walk did not stop at the bad file"
    );
    assert_eq!(second.files_committed, 0);
    assert_eq!(second.failures.len(), 1, "{:?}", second.failures);
    assert_eq!(second.failures[0].0, tree.subject);
    assert!(
        second.failures[0].1.contains("watermark"),
        "{}",
        second.failures[0].1
    );

    // Nothing archived moved: not the blob, not the checksum, not the turn
    // rows, and above all not the watermark - a re-ingest from offset 0 would
    // have reset it and rewritten the blob from the surviving half.
    assert_eq!(
        tree.archived(&tree.subject),
        before,
        "the archive was rewritten"
    );
    assert_eq!(tree.archived(&tree.bystander), bystander_before);

    // The signal, in both places it has to appear.
    assert_eq!(tree.flagged(), vec![tree.key(&tree.subject)]);
    let run = tree.last_run_error().expect("the skip is in the runs row");
    assert!(run.contains(tree.subject.to_str().unwrap()), "{run}");

    let report = tree.verify();
    assert!(!report.is_ok(), "verify passed a diverged store");
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert_eq!(report.failures[0].session_key, tree.key(&tree.subject));
    let text = report.render();
    assert!(text.contains("shorter"), "{text}");
    assert!(
        !text.contains(tree.bystander.to_str().unwrap()),
        "named the bystander: {text}"
    );
}

/// The flag is a state, not a verdict: a file that was mid-write stops being
/// reported once it is whole again. Without this, one interrupted write would
/// make `verify` complain forever and the signal would stop meaning anything.
#[test]
fn restoring_the_transcript_clears_the_flag() {
    let tree = tree();
    tree.pass();
    let before = tree.archived(&tree.subject);
    tree.truncate();
    tree.pass();
    assert!(tree.diverged(&tree.subject));

    tree.restore();
    let third = tree.pass();
    assert!(third.failures.is_empty(), "{:?}", third.failures);
    // Nothing new past the watermark, so this commits nothing - which is the
    // case the clearing has to survive, because it opens no transaction.
    assert_eq!(third.files_committed, 0);
    assert!(
        !tree.diverged(&tree.subject),
        "the flag outlived the damage"
    );
    assert_eq!(
        tree.archived(&tree.subject),
        before,
        "the repaired file was re-ingested rather than left alone"
    );

    let report = tree.verify();
    assert!(report.is_ok(), "{:?}", report.failures);
    assert_eq!(report.render(), "");
}

/// A transcript that grows again after being flagged clears the flag too, and
/// on the path that *does* open a transaction - the one the boundary rides in.
#[test]
fn a_flagged_transcript_that_grows_again_is_ingested_and_cleared() {
    let tree = tree();
    tree.pass();
    let turns_before = turn_rows(&tree.conn()).len();
    tree.truncate();
    tree.pass();
    assert!(tree.diverged(&tree.subject));

    tree.restore();
    let line = testkit::boundary_line();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&tree.subject)
        .unwrap();
    file.write_all(&line).unwrap();
    file.write_all(b"\n").unwrap();
    drop(file);

    let summary = tree.pass();
    assert_eq!(summary.files_committed, 1, "{summary:?}");
    assert!(!tree.diverged(&tree.subject));
    assert_eq!(turn_rows(&tree.conn()).len(), turns_before + 1);
    assert_eq!(boundaries(&tree.conn()).len(), 1);
    assert!(tree.verify().is_ok());
}

/// A file that is long enough and is not the file that was archived stays
/// flagged, and the archive stays exactly as it was.
///
/// The case a length test cannot see. A transcript truncated to half its length
/// and then written back past its old watermark satisfies "no shorter than the
/// archive" while every byte between the truncation point and the watermark is
/// different. Clearing the flag on that would append at the stale offset, so the
/// archived stream would carry the first half of one file, a hole, and a
/// fragment of a record - permanently, with `verify` reporting it clean because
/// the blob still matches its own checksum.
#[test]
fn a_transcript_rewritten_under_its_watermark_stays_flagged() {
    let tree = tree();
    tree.pass();
    let before = tree.archived(&tree.subject);
    tree.truncate();
    tree.pass();
    assert!(tree.diverged(&tree.subject));

    // Not `restore`: different bytes, and more of them than the watermark, so
    // the file passes the length test and fails on content.
    let mut rewritten = Vec::new();
    for _ in 0..(tree.original.len() / 40 + 20) {
        rewritten.extend_from_slice(&testkit::boundary_line());
        rewritten.push(b'\n');
    }
    assert!(
        rewritten.len() > tree.original.len(),
        "the rewritten file must clear the watermark, or this tests the length \
         path instead"
    );
    std::fs::write(&tree.subject, &rewritten).unwrap();

    let summary = tree.pass();
    assert_eq!(
        summary.files_committed, 0,
        "a file that is not the archived one must not be appended to: {summary:?}"
    );
    assert_eq!(summary.failures.len(), 1, "{summary:?}");
    assert!(
        tree.diverged(&tree.subject),
        "the flag was cleared on a length test alone"
    );
    assert_eq!(
        tree.archived(&tree.subject),
        before,
        "the archive moved for a file whose earlier bytes are gone"
    );

    // The message names the content, not a length the user would check and
    // find correct.
    let error = tree.last_run_error().unwrap_or_default();
    assert!(
        error.contains("are not the ones archived"),
        "the failure must say the bytes differ: {error}"
    );
}

/// Half of the discrimination the dedicated error variant exists for: the
/// failure D-13's refusal used to be indistinguishable from.
///
/// `path_key` refuses a non-UTF-8 transcript path with the same `InvalidData`
/// io kind `read_tail` used to carry, so a pass that told the two apart by kind
/// would flag a session for a failure that says nothing about the file's
/// length. The failure is recorded like any other and the store's flag count
/// stays zero.
///
/// This one cannot fail by itself if the pass flagged *every* failure, because
/// a path that is not text has no session key to flag - which is exactly why
/// `another_per_file_failure_flags_no_session` exists beside it.
///
/// Unix only: this needs a path that is bytes and not text, and Windows paths
/// are UTF-16 where the equivalent is an unpaired surrogate.
#[cfg(unix)]
#[test]
fn a_failure_that_is_not_a_divergence_flags_no_session() {
    use std::os::unix::ffi::OsStrExt;

    let tree = tree();
    tree.pass();
    assert!(tree.flagged().is_empty());

    // `agent-<0xff>.jsonl`: discovery matches on the lossy name, so it is
    // picked up as a sidecar, and ingest then finds the real path is not text.
    let project = tree.claude_dir.join("projects").join(PROJECT);
    let name = std::ffi::OsStr::from_bytes(b"agent-\xff.jsonl");
    let bad = project.join(name);
    std::fs::copy(testkit::fixture_path("session-basic.jsonl"), &bad).unwrap();

    let summary = tree.pass();
    assert_eq!(summary.failures.len(), 1, "{:?}", summary.failures);
    assert_eq!(summary.failures[0].0, bad);
    let reason = &summary.failures[0].1;
    assert!(reason.contains("not valid UTF-8"), "{reason}");
    assert!(!reason.contains("watermark"), "{reason}");

    assert!(
        tree.flagged().is_empty(),
        "a non-divergence failure flagged {:?}",
        tree.flagged()
    );
    assert!(tree.verify().is_ok(), "{:?}", tree.verify().failures);
    let run = tree
        .last_run_error()
        .expect("the failure is in the runs row");
    assert!(run.contains("not valid UTF-8"), "{run}");
}

/// The other half, and the one that bites: a per-file failure at a perfectly
/// good path, against a session that has a `session_meta` row to flag.
///
/// A pass that raised D-13's flag on any error rather than on the one variant
/// would set it here, and the archive would be reported as diverged from a
/// transcript that is sitting on disk at its full length.
#[test]
fn another_per_file_failure_flags_no_session() {
    let tree = tree();
    tree.pass();

    // A stored checksum that is not 32 bytes. `Existing::read` refuses it
    // before the file is even opened, so the failure is nothing to do with the
    // transcript's length - and the session_meta row it would be flagged on is
    // right there.
    let changed = tree
        .conn()
        .execute(
            "UPDATE session_meta SET checksum = x'0102030405' WHERE session_key = ?1",
            [tree.key(&tree.subject)],
        )
        .unwrap();
    assert_eq!(changed, 1);

    let summary = tree.pass();
    assert_eq!(summary.failures.len(), 1, "{:?}", summary.failures);
    assert_eq!(summary.failures[0].0, tree.subject);
    assert!(
        !summary.failures[0].1.contains("watermark"),
        "{}",
        summary.failures[0].1
    );
    assert!(
        tree.flagged().is_empty(),
        "a failure that is not a divergence flagged {:?}",
        tree.flagged()
    );
}
