//! Walking every blob against the checksum recorded for it (STOR-03).
//!
//! D-06 fixes what is compared: BLAKE3 over the **uncompressed** session bytes,
//! held in `session_meta`, so neither a codec-level bug nor a format bump
//! invalidates a stored checksum and `verify` can still tell corruption from a
//! decoder change.
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
//! statement about the *file*, not about the blob - the blob is intact, and
//! saying so is most of the message.
//!
//! Phase 8 adds one *silence*, on the same terms: a session
//! `session_meta.is_evicted` marks (RET-02, D-03) holds a deliberately emptied
//! blob, so there is nothing to decompress and nothing to hash. It is counted
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

use crate::blob;
use crate::error::Result;
use crate::store::Store;

/// One session that did not verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The canonical transcript path: the unique identity of the session.
    pub session_key: String,
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

    /// The exact text `verbatim verify` writes to stdout: one line per failure,
    /// nothing when the store is clean.
    ///
    /// Rendering lives here rather than in the command so the library tests
    /// assert on the same bytes the process prints.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for failure in &self.failures {
            out.push_str(&failure.session_key);
            out.push_str(": ");
            out.push_str(&failure.detail);
            out.push('\n');
        }
        out
    }
}

/// Check every session's blob against its recorded checksum.
///
/// A blob that fails to decompress is a failure for *that* session and never an
/// abort: a second corrupt session must still be reported, which is the whole
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

fn walk(conn: &Connection) -> Result<Report> {
    let mut statement = conn.prepare(
        "SELECT s.session_key, s.blob, m.checksum, m.transcript_diverged, m.is_evicted
         FROM sessions s LEFT JOIN session_meta m USING (session_key)
         ORDER BY s.session_key",
    )?;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, Option<Vec<u8>>>(2)?,
            r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
            r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
        ))
    })?;

    let mut report = Report::default();
    for row in rows {
        let (session_key, bytes, expected, diverged, evicted) = row?;
        report.checked += 1;
        // Counted, and never a failure. An evicted session's blob was emptied
        // on purpose (RET-02, phase 8 D-03), so `blob::read_all` has no header
        // to find and would report every one of them as "does not decompress" -
        // which is the confusion the divergence arm above exists to prevent,
        // arriving from the other side. The count still moves because the walk
        // did visit the session and can say what it is; the check is what has
        // nothing left to check.
        if !evicted {
            if let Err(detail) = check(&bytes, expected.as_deref()) {
                report.failures.push(Failure {
                    session_key: session_key.clone(),
                    detail,
                });
            }
        }
        // Independent of the checksum verdict, not an alternative to it. A
        // session can have both a diverged transcript and a damaged blob, and
        // collapsing them would report the lesser fact and hide the graver one.
        if diverged {
            report.failures.push(Failure {
                session_key,
                detail: DIVERGED.into(),
            });
        }
    }
    Ok(report)
}

/// One session's verdict. The error is the message the report prints, so it
/// says what is wrong rather than what failed.
fn check(bytes: &[u8], expected: Option<&[u8]>) -> std::result::Result<(), String> {
    let Some(expected) = expected else {
        // The archive is two tables written in one transaction (STOR-02), so a
        // blob with no metadata row is not a state ingest can produce.
        return Err("no session_meta row, so there is no checksum to check it against".into());
    };

    let stream = blob::read_all(bytes).map_err(|e| format!("blob does not decompress: {e}"))?;
    let actual = blake3::hash(&stream);
    if actual.as_bytes().as_slice() != expected {
        return Err(format!(
            "checksum mismatch: expected {}, found {}",
            hex(expected),
            actual.to_hex()
        ));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
