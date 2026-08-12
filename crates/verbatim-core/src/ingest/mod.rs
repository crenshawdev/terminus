//! One ingest pass over one named transcript.
//!
//! The order of the first steps is the design. The path is canonicalized before
//! anything else, the lock is taken **before** the store is opened (D-15), and
//! everything the pass writes commits in **one** transaction (STOR-02) so a
//! killed process leaves no partially indexed session.
//!
//! Reads happen outside that transaction and writes inside it, which is safe
//! for exactly one reason: the `LOCK` guard above is the only writer, so
//! nothing can change the store between the read and the write. Reading inside
//! the transaction instead would hold it open across compression, and a write
//! transaction held for a whole pass stops MCP readers - the reason redb was
//! rejected (`DESIGN-BRIEF.md:83`).

pub mod lock;

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

pub use lock::{Attempt, IngestLock, LOCK_FILE_NAME};

use rusqlite::{Connection, OptionalExtension};

use crate::blob;
use crate::error::{Error, Result};
use crate::parse::{self, Scan};
use crate::store::{schema, Store};

/// What one invocation of [`run`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Another process is mid-pass. Nothing was written, and this is a success:
    /// the hook spawn is the scheduler and a second invocation is expected
    /// (ING-02).
    LockHeld,
    /// The transcript has no complete record past the stored watermark. No row
    /// is added anywhere, not even to `runs`.
    UpToDate,
    /// The pass committed.
    Committed(Pass),
}

/// What a committed pass moved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pass {
    /// The canonical transcript path, which is also the session's key (D-01).
    pub session_key: String,
    pub bytes_read: u64,
    pub turns_added: usize,
    /// Where the next pass resumes: the byte just past the last `\n` (D-14).
    pub watermark: u64,
}

/// Ingest one transcript into the store under `data_dir`.
pub fn run(data_dir: &Path, transcript: &Path) -> Result<Outcome> {
    let started = Instant::now();

    // Before anything else, and resolving symlinks: `~/.claude` is a symlink
    // chain on the development machine (`DESIGN-BRIEF.md:406`), so the same
    // transcript reached by two paths would otherwise get two archive rows.
    // This resolves CONTEXT's third flagged assumption as "canonicalize".
    let canonical = transcript
        .canonicalize()
        .map_err(|e| Error::io(transcript, e))?;

    let _guard = match lock::try_acquire(data_dir)? {
        Attempt::Held => return Ok(Outcome::LockHeld),
        Attempt::Acquired(guard) => guard,
    };

    let mut store = Store::open(data_dir)?;
    ingest_locked(&mut store, &canonical, started)
}

/// The pass itself, with the lock already held and the store already open.
pub(crate) fn ingest_locked(store: &mut Store, path: &Path, started: Instant) -> Result<Outcome> {
    // The session is keyed on the transcript FILE identity, never on the
    // record's session id (D-01): 812 real sidecar files report their parent's
    // sessionId, and keying on that overwrites a parent session's blob with a
    // 3 KB agent transcript. The watermark is keyed on the same value.
    let session_key = path_key(path)?;

    let existing = Existing::read(store.conn(), &session_key)?;
    let tail = read_tail(path, existing.watermark)?;
    let scan = parse::scan_from(&tail, existing.watermark, existing.turn_count);
    let consumed = scan.consumed(existing.watermark);
    if consumed == 0 {
        // No complete record past the watermark. A rerun on an unchanged file
        // lands here, and adds no row to any table.
        return Ok(Outcome::UpToDate);
    }
    let fresh = &tail[..consumed];

    // Compression and hashing happen here, outside the transaction. `append`
    // takes the checksum already recorded for what the blob holds and verifies
    // it before writing, so a re-ingest of a growing session cannot mint a
    // fresh checksum over corruption.
    let (bytes, checksum, uncompressed_len) = match &existing.session {
        Some(session) => {
            let appended = blob::append(&session.blob, &session.checksum, fresh)?;
            (
                appended.bytes,
                appended.checksum,
                appended.uncompressed_len,
            )
        }
        None => {
            let written = blob::write(fresh)?;
            (written.bytes, written.checksum, written.uncompressed_len)
        }
    };

    let session_no = match &existing.session {
        Some(session) => session.session_no,
        None => next_session_no(store.conn())?,
    };
    let continues_from = continues_from(store.conn(), &session_key, &scan)?;

    let pass = Pass {
        session_key: session_key.clone(),
        bytes_read: consumed as u64,
        turns_added: scan.turn_count(),
        watermark: scan.resume_offset,
    };

    // One transaction, and nothing outside it mutates the store (STOR-02).
    let tx = store.conn_mut().transaction()?;
    tx.execute(
        "INSERT INTO sessions (session_key, session_no, blob) VALUES (?1, ?2, ?3)
         ON CONFLICT(session_key) DO UPDATE SET blob = excluded.blob",
        rusqlite::params![session_key, session_no, bytes],
    )?;
    write_session_meta(
        &tx,
        &session_key,
        path,
        &checksum,
        uncompressed_len,
        continues_from.as_deref(),
        &scan,
    )?;
    for (record, turn) in scan.turns() {
        let id = schema::turn_id(session_no, turn.turn_seq);
        tx.execute(
            "INSERT INTO turns (
                id, session_key, turn_seq, uuid, parent_uuid, record_type, tool_name, ts,
                stream_offset, byte_len
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                uuid = excluded.uuid, parent_uuid = excluded.parent_uuid,
                record_type = excluded.record_type, tool_name = excluded.tool_name,
                ts = excluded.ts, stream_offset = excluded.stream_offset,
                byte_len = excluded.byte_len",
            rusqlite::params![
                id,
                session_key,
                turn.turn_seq,
                turn.uuid,
                turn.parent_uuid,
                turn.record_type,
                turn.tool_name,
                turn.timestamp,
                record.offset as i64,
                record.len as i64,
            ],
        )?;
    }
    tx.execute(
        "INSERT INTO watermarks (transcript_path, byte_offset, updated_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(transcript_path) DO UPDATE SET
            byte_offset = excluded.byte_offset, updated_at = excluded.updated_at",
        rusqlite::params![session_key, pass.watermark as i64],
    )?;
    record_run(&tx, &pass, started.elapsed())?;
    tx.commit()?;

    Ok(Outcome::Committed(pass))
}

/// What the store already holds for this transcript.
struct Existing {
    watermark: u64,
    /// Turns already stored for this session, which is the ordinal the next one
    /// takes: a tail pass numbers on from here and a rebuild from 0 lands on
    /// the same numbers, because both count in byte order (D-02).
    turn_count: i64,
    session: Option<ExistingSession>,
}

struct ExistingSession {
    session_no: i64,
    blob: Vec<u8>,
    checksum: [u8; 32],
}

impl Existing {
    fn read(conn: &Connection, session_key: &str) -> Result<Existing> {
        let watermark: Option<i64> = conn
            .query_row(
                "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
                [session_key],
                |r| r.get(0),
            )
            .optional()?;
        let turn_count: i64 = conn.query_row(
            "SELECT count(*) FROM turns WHERE session_key = ?1",
            [session_key],
            |r| r.get(0),
        )?;

        let session = conn
            .query_row(
                "SELECT s.session_no, s.blob, m.checksum
                 FROM sessions s JOIN session_meta m USING (session_key)
                 WHERE s.session_key = ?1",
                [session_key],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;

        let session = match session {
            Some((session_no, blob, checksum)) => Some(ExistingSession {
                session_no,
                blob,
                checksum: checksum.as_slice().try_into().map_err(|_| {
                    Error::BlobChecksumMismatch {
                        expected: "32 bytes".into(),
                        actual: format!("{} bytes", checksum.len()),
                    }
                })?,
            }),
            None => None,
        };

        Ok(Existing {
            watermark: watermark.unwrap_or(0) as u64,
            turn_count,
            session,
        })
    }
}

/// The session key: the canonical path, as text.
fn path_key(path: &Path) -> Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        Error::io(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transcript path is not valid UTF-8, so it cannot key a session",
            ),
        )
    })
}

/// Read from the stored watermark to the end of the file.
fn read_tail(path: &Path, watermark: u64) -> Result<Vec<u8>> {
    let mut file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let len = file.metadata().map_err(|e| Error::io(path, e))?.len();
    if len < watermark {
        // A transcript never shrinks: compaction appends to the same file
        // (`DESIGN-BRIEF.md:114`). A shorter file is a different file at the
        // same path, and guessing which bytes are still ours would corrupt the
        // blob. Phase 2 owns re-identification; phase 1 refuses.
        return Err(Error::io(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("is {len} bytes but its watermark is at {watermark}"),
            ),
        ));
    }
    file.seek(SeekFrom::Start(watermark))
        .map_err(|e| Error::io(path, e))?;
    let mut buffer = Vec::with_capacity((len - watermark) as usize);
    file.read_to_end(&mut buffer).map_err(|e| Error::io(path, e))?;
    Ok(buffer)
}

fn next_session_no(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT coalesce(max(session_no) + 1, 0) FROM sessions",
        [],
        |r| r.get(0),
    )?)
}

/// D-11's lineage signal, in the session-id namespace.
///
/// The record's foreign `session_id` field is the reliable one: 187 of 1,221
/// real top-level transcripts carry one, an order of magnitude above the 1.2%
/// `parentUuid` resume rate. `parentUuid` is the fallback and only that - it is
/// message-level threading, so it is resolved through `turns` to the session
/// that holds the parent record, keeping this column in one namespace.
fn continues_from(conn: &Connection, session_key: &str, scan: &Scan) -> Result<Option<String>> {
    if let Some(foreign) = scan
        .records
        .iter()
        .find_map(|r| r.foreign_session_id.clone())
    {
        return Ok(Some(foreign));
    }

    // Only the transcript's very first turn can name a predecessor file; a
    // parentUuid anywhere later threads within this same file.
    let Some((_, first)) = scan.turns().next() else {
        return Ok(None);
    };
    if first.turn_seq != 0 {
        return Ok(None);
    }
    let Some(parent_uuid) = first.parent_uuid.as_deref() else {
        return Ok(None);
    };

    Ok(conn
        .query_row(
            "SELECT m.session_id
             FROM turns t JOIN session_meta m USING (session_key)
             WHERE t.uuid = ?1 AND t.session_key <> ?2
             LIMIT 1",
            rusqlite::params![parent_uuid, session_key],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// Upsert the archive's metadata row.
///
/// Every column that describes the session as a whole is `coalesce`d onto what
/// is already there: a tail pass sees only the tail, and `session_id`, `cwd`,
/// `gitBranch` and the first turn's timestamp were established by the first
/// pass. `project`, `is_final`, `is_evicted` and `parent_session_key` stay null
/// - project identity from `cwd` and session linking are phase 2.
#[allow(clippy::too_many_arguments)]
fn write_session_meta(
    tx: &Connection,
    session_key: &str,
    path: &Path,
    checksum: &[u8; 32],
    uncompressed_len: u64,
    continues_from: Option<&str>,
    scan: &Scan,
) -> Result<()> {
    let session_id = scan.records.iter().find_map(|r| r.session_id.clone());
    let cwd = scan.records.iter().find_map(|r| r.cwd.clone());
    let branch = scan.records.iter().find_map(|r| r.git_branch.clone());
    // Byte order, not clock order (D-02): the "first" turn is the first in the
    // file even when its timestamp is later than its neighbour's.
    let first_turn_at = scan.turns().next().map(|(_, t)| t.timestamp.clone());
    let last_turn_at = scan.turns().last().map(|(_, t)| t.timestamp.clone());

    tx.execute(
        "INSERT INTO session_meta (
            session_key, session_id, transcript_path, checksum, uncompressed_len,
            continues_from, first_turn_at, last_turn_at, cwd, branch
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(session_key) DO UPDATE SET
            session_id = coalesce(session_meta.session_id, excluded.session_id),
            transcript_path = excluded.transcript_path,
            checksum = excluded.checksum,
            uncompressed_len = excluded.uncompressed_len,
            continues_from = coalesce(session_meta.continues_from, excluded.continues_from),
            first_turn_at = coalesce(session_meta.first_turn_at, excluded.first_turn_at),
            last_turn_at = coalesce(excluded.last_turn_at, session_meta.last_turn_at),
            cwd = coalesce(session_meta.cwd, excluded.cwd),
            branch = coalesce(session_meta.branch, excluded.branch)",
        rusqlite::params![
            session_key,
            session_id,
            path.to_string_lossy(),
            checksum.as_slice(),
            uncompressed_len as i64,
            continues_from,
            first_turn_at,
            last_turn_at,
            cwd,
            branch,
        ],
    )?;
    Ok(())
}

/// Write the one `runs` row a committed pass leaves behind.
///
/// There is no log file; `status` (phase 2) surfaces this table. A *failed*
/// pass needs a second transaction after a rollback and is phase 2's (ING-03,
/// ING-09): phase 1 records only a pass that committed, and it records it
/// inside the same transaction as the pass.
pub(crate) fn record_run(
    conn: &Connection,
    pass: &Pass,
    elapsed: std::time::Duration,
) -> Result<()> {
    conn.execute(
        "INSERT INTO runs (started_at, finished_at, duration_ms, files_seen, bytes_read, turns_added)
         VALUES (
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            ?2, 1, ?3, ?4
         )",
        rusqlite::params![
            format!("-{} seconds", elapsed.as_secs_f64()),
            elapsed.as_millis() as i64,
            pass.bytes_read as i64,
            pass.turns_added as i64,
        ],
    )?;
    Ok(())
}
