//! Walking every session's block rows and the checksum recorded for it
//! (STOR-03).
//!
//! D-06 fixes what is compared: hashes over the **uncompressed** bytes, so
//! neither a codec-level bug nor a format bump invalidates a stored hash and
//! `verify` can still tell corruption from a decoder change. Since phase 7
//! (D-16) each session gets three checks, and every one that fails is
//! reported: the rows are numbered 0 to the count `uncompressed_len` and the
//! block size give, with no gap; each block decodes to its length and its
//! stored hash; and `session_meta.checksum` is BLAKE3 over the stored hashes in
//! `block_no` order (D-04). The first two name the block. The checksum names
//! none, because it covers every hash at once.
//!
//! D-16 fixes what is *not* compared: `PRAGMA integrity_check` belongs to
//! `doctor` (INST-06, phase 4). A page-level complaint carries no session
//! attribution, so it would ride alongside the named session and break AC3's
//! "and no other" on a store that is otherwise fine.
//!
//! Phase 2 adds one further failure, and it passes the same test: D-13's
//! divergence, raised when the transcript on disk turned out shorter than the
//! bytes the archive holds. It is read off `session_meta.transcript_diverged`,
//! which the pass sets against exactly one session, so it is attributed by
//! `session_key` like every other failure here and names no other. It is a
//! statement about the *file*, not about the archive - the archive is intact,
//! and saying so is most of the message.
//!
//! Phase 8 adds one *silence*, on the same terms: a session
//! `session_meta.is_evicted` marks (RET-02, D-03) has had its block rows
//! deleted on purpose, so there is nothing to decompress and nothing to hash. It is counted
//! and it is never a failure. The divergence check stays independent of it,
//! exactly as it is independent of the checksum verdict - "the file on disk is
//! shorter than what was archived" is a statement about the FILE, and an
//! evicted session's file can still be wrong about it.
//!
//! # Which identifier names a failure
//!
//! The `session_key` - the canonical transcript path - and never the record's
//! `session_id`. D-01 is the reason: 812 real sidecar files report their
//! *parent's* `sessionId`, so a report naming session ids cannot say which of
//! two sessions is corrupt, and "prints no other session id" is unfalsifiable
//! when two sessions share one. The key is the primary key of `sessions` and is
//! unique by construction.

use rusqlite::Connection;

use crate::blob::BlockReader;
use crate::error::{Error, Result};
use crate::store::{archive, Store};

/// One thing about one session that did not verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The canonical transcript path: the unique identity of the session.
    pub session_key: String,
    /// The block the failure is pinned to, or `None` for one about the whole
    /// session: the checksum, the metadata row, the block size, divergence.
    pub block: Option<i64>,
    pub detail: String,
}

/// The outcome of a walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub checked: usize,
    pub failures: Vec<Failure>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }

    /// The exact text `terminus verify` writes to stdout: one line per failure,
    /// nothing when the store is clean.
    ///
    /// Rendering lives here rather than in the command so the library tests
    /// assert on the same bytes the process prints.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for failure in &self.failures {
            out.push_str(&failure.session_key);
            out.push_str(": ");
            if let Some(block) = failure.block {
                out.push_str(&format!("block {block}: "));
            }
            out.push_str(&failure.detail);
            out.push('\n');
        }
        out
    }
}

/// Check every session's block rows and its recorded checksum.
///
/// A session that fails is a failure for *that* session and never an abort: a
/// second corrupt session must still be reported, which is the whole
/// difference between "these two sessions are damaged" and "the store is gone"
/// (`DESIGN-BRIEF.md:98`).
pub fn verify(store: &Store) -> Result<Report> {
    walk(store.conn())
}

/// What a diverged session is told (D-13).
///
/// "The archive was left untouched" is the load-bearing half. The obvious
/// response to `verify` naming a session is to re-run ingest, and for this
/// failure that is neither necessary nor sufficient: the pass already refused
/// to re-read the file from offset 0, on purpose, because doing so would
/// overwrite archived bytes.
const DIVERGED: &str = "the transcript on disk is shorter than the bytes the archive holds \
                        for it; the archive was left untouched";

/// What `session_meta` says about one session.
struct Meta {
    checksum: Option<Vec<u8>>,
    block_size: Option<i64>,
    uncompressed_len: Option<i64>,
}

fn walk(conn: &Connection) -> Result<Report> {
    // Keys and metadata first, rows one session at a time, for the memory
    // bound `reindex` documents: selecting every row in one statement holds as
    // much of the archive as the cursor has reached.
    type Row = (String, Meta, bool, bool);
    let sessions: Vec<Row> = conn
        .prepare(
            "SELECT s.session_key, m.checksum, m.transcript_diverged, m.is_evicted,
                    m.block_size, m.uncompressed_len
             FROM sessions s LEFT JOIN session_meta m USING (session_key)
             ORDER BY s.session_key",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Meta {
                    checksum: r.get(1)?,
                    block_size: r.get(4)?,
                    uncompressed_len: r.get(5)?,
                },
                r.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;

    let mut report = Report::default();
    for (session_key, meta, diverged, evicted) in sessions {
        report.checked += 1;
        // Counted, and never a failure. An evicted session's block rows were
        // deleted on purpose (RET-02, phase 8 D-03), so it would report every
        // block as missing - the confusion the divergence arm exists to
        // prevent, arriving from the other side.
        if !evicted {
            for (block, detail) in check(conn, &session_key, &meta)? {
                report.failures.push(Failure {
                    session_key: session_key.clone(),
                    block,
                    detail,
                });
            }
        }
        // Independent of the archive's verdict, not an alternative to it. A
        // session can have both a diverged transcript and a damaged archive, and
        // collapsing them would report the lesser fact and hide the graver one.
        if diverged {
            report.failures.push(Failure {
                session_key,
                block: None,
                detail: DIVERGED.into(),
            });
        }
    }
    Ok(report)
}

/// Every failing check for one session, each with the block it names (D-16).
/// The outer error is the database's and ends the walk.
fn check(conn: &Connection, session_key: &str, meta: &Meta) -> Result<Vec<(Option<i64>, String)>> {
    let Some(expected) = meta.checksum.as_deref() else {
        // The archive's tables are written in one transaction (STOR-02), so a
        // session with no metadata row is not a state ingest can produce.
        return Ok(vec![(
            None,
            "no session_meta row, so there is no checksum to check it against".into(),
        )]);
    };
    let rows = archive::rows(conn, session_key)?;
    let mut failures = Vec::new();

    // Numbering and each block need the layout; the checksum does not, so a
    // bad layout still gets its checksum checked.
    match layout(meta) {
        Err(detail) => failures.push((None, detail)),
        Ok(reader) => {
            let count = reader.block_count();
            let mut next = 0u64;
            for row in &rows {
                let Some(block_no) = u64::try_from(row.block_no).ok().filter(|n| *n < count) else {
                    failures.push((
                        Some(row.block_no),
                        format!(
                            "unexpected row: {} bytes at block size {} make {count} blocks",
                            reader.uncompressed_len(),
                            meta.block_size.unwrap_or_default()
                        ),
                    ));
                    continue;
                };
                if block_no > next {
                    failures.push(missing(next, block_no));
                }
                next = block_no + 1;
                if let Err(detail) = check_block(&reader, block_no, row) {
                    failures.push((Some(row.block_no), detail));
                }
            }
            if next < count {
                failures.push(missing(next, count));
            }
        }
    }

    // D-04: BLAKE3 over the stored hashes in `block_no` order, the definition
    // `blob::block::checksum` writes, taken over the rows as they are.
    let mut hasher = blake3::Hasher::new();
    for row in &rows {
        hasher.update(&row.hash);
    }
    let actual = hasher.finalize();
    if actual.as_bytes().as_slice() != expected {
        failures.push((
            None,
            format!(
                "checksum mismatch: expected {}, found {}",
                hex(expected),
                actual.to_hex()
            ),
        ));
    }
    Ok(failures)
}

/// A reader over the recorded layout, or why there is none. A bad block size
/// is refused here before anything is sized from it.
fn layout(meta: &Meta) -> std::result::Result<BlockReader, String> {
    let size = archive::block_size(meta.block_size).map_err(|e| e.to_string())?;
    let len = match meta.uncompressed_len {
        None => return Err("no stream length recorded".into()),
        Some(len) => u64::try_from(len).map_err(|_| format!("stream length {len} is negative"))?,
    };
    BlockReader::new(size, len).map_err(|e| e.to_string())
}

/// The rows `from..to` are absent. A run is one failure naming its first
/// block, so a corrupt length cannot turn into millions of lines.
fn missing(from: u64, to: u64) -> (Option<i64>, String) {
    let detail = if to - from == 1 {
        "row is missing".to_owned()
    } else {
        format!(
            "row is missing, and so are the rows through block {}",
            to - 1
        )
    };
    (Some(from as i64), detail)
}

/// One block decodes to its expected length and hashes to its stored hash.
fn check_block(
    reader: &BlockReader,
    block_no: u64,
    row: &archive::Row,
) -> std::result::Result<(), String> {
    let plain = reader.decompress(block_no, &row.bytes).map_err(|e| {
        // The reader names the block in its own message, and the report line
        // already does.
        let text = match e {
            Error::BlobFormat { detail } => detail,
            other => other.to_string(),
        };
        let text = text
            .strip_prefix(&format!("block {block_no}: "))
            .unwrap_or(&text);
        format!("does not decompress: {text}")
    })?;
    let actual = blake3::hash(&plain);
    if actual.as_bytes().as_slice() != row.hash.as_slice() {
        return Err(format!(
            "hash mismatch: expected {}, found {}",
            hex(&row.hash),
            actual.to_hex()
        ));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
