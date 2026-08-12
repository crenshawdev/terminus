//! Ingest: one named transcript here, a whole tree in [`pass`].
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
pub mod pass;

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

pub use lock::{Attempt, IngestLock, LOCK_FILE_NAME};

use rusqlite::{Connection, OptionalExtension};

use crate::blob;
use crate::derive;
use crate::error::{Error, Result};
use crate::parse::{self, Scan};
use crate::store::Store;

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

    // Not `Store::open`: a store older than this build has its derived tables
    // rebuilt first (STOR-05), so a pass never appends turn rows in one shape
    // beside rows written in another.
    let mut store = crate::reindex::open_up_to_date(data_dir)?;
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
            (appended.bytes, appended.checksum, appended.uncompressed_len)
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

    fault::stall(fault::AFTER_BLOB);

    // The fault the crash harness must be able to catch, and the reason that
    // harness is worth anything: with this on, the watermark commits in a
    // transaction of its own *before* the pass, so a kill in between leaves a
    // store claiming bytes its blob does not hold. Compiled only under
    // `testkit`; the shipped binary has no such branch.
    if fault::split_watermark() {
        let early = store.conn_mut().transaction()?;
        write_watermark(&early, &session_key, pass.watermark)?;
        early.commit()?;
        fault::stall(fault::AFTER_SPLIT_WATERMARK);
    }

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
    fault::stall(fault::IN_TX_AFTER_SESSION);
    for (record, turn) in scan.turns() {
        // The seam, not an insert of our own: the rebuild path calls the same
        // function, which is what stops it from drifting from what ingest wrote
        // (STOR-04). `fresh` starts at the watermark, so a record's stream
        // offset has to be rebased to index it.
        let from = (record.offset - existing.watermark) as usize;
        derive::derive_turn(
            &tx,
            derive::TurnRow {
                session_key: &session_key,
                session_no,
                turn,
                stream_offset: record.offset,
                byte_len: record.len,
                record: &fresh[from..from + record.len as usize],
            },
        )?;
    }
    fault::stall(fault::IN_TX_AFTER_TURNS);
    write_watermark(&tx, &session_key, pass.watermark)?;
    record_run(&tx, &pass, started.elapsed())?;
    fault::stall(fault::IN_TX_BEFORE_COMMIT);
    tx.commit()?;
    fault::stall(fault::AFTER_COMMIT);

    Ok(Outcome::Committed(pass))
}

/// The byte offset the next pass resumes from: just past the last `\n` (D-14).
fn write_watermark(conn: &Connection, session_key: &str, offset: u64) -> Result<()> {
    conn.execute(
        "INSERT INTO watermarks (transcript_path, byte_offset, updated_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(transcript_path) DO UPDATE SET
            byte_offset = excluded.byte_offset, updated_at = excluded.updated_at",
        rusqlite::params![session_key, offset as i64],
    )?;
    Ok(())
}

/// Where a kill can be aimed, so the crash harness hits the inside of the
/// transaction instead of relying on a race it cannot win.
///
/// A release ingest of a few-KB fixture finishes in single-digit milliseconds,
/// so a harness that only randomizes on elapsed time lands nearly every kill
/// after the commit and passes vacuously. The stall is the fix: the child
/// announces that it has reached a named point, then waits there to be killed,
/// which makes "killed inside the transaction" a fact rather than a hope.
///
/// The whole module is inert without the `testkit` feature - the shipped binary
/// reads no environment variable and takes no branch.
pub mod fault {
    /// Compression and hashing are done; nothing is written yet.
    pub const AFTER_BLOB: &str = "after-blob";
    /// Inside the transaction, after `sessions` and `session_meta`.
    pub const IN_TX_AFTER_SESSION: &str = "in-tx-after-session";
    /// Inside the transaction, after every turn and its derived rows.
    pub const IN_TX_AFTER_TURNS: &str = "in-tx-after-turns";
    /// Inside the transaction, with everything written and nothing committed.
    pub const IN_TX_BEFORE_COMMIT: &str = "in-tx-before-commit";
    /// The pass committed; the process has not exited.
    pub const AFTER_COMMIT: &str = "after-commit";
    /// Only reachable under [`SPLIT`]: the watermark committed on its own.
    pub const AFTER_SPLIT_WATERMARK: &str = "after-split-watermark";

    /// Every point a kill may be aimed at, in the order a pass reaches them.
    pub const POINTS: &[&str] = &[
        AFTER_BLOB,
        IN_TX_AFTER_SESSION,
        IN_TX_AFTER_TURNS,
        IN_TX_BEFORE_COMMIT,
        AFTER_COMMIT,
    ];

    /// Names the point to stall at.
    pub const AT: &str = "VERBATIM_FAULT_AT";
    /// Names a file to create on arrival, which is what the parent polls for.
    pub const READY: &str = "VERBATIM_FAULT_READY";
    /// Moves the watermark into its own earlier transaction.
    pub const SPLIT: &str = "VERBATIM_FAULT_SPLIT_WATERMARK";

    /// Long enough that the parent always gets to kill first, short enough that
    /// a harness bug times out instead of hanging a test run forever.
    #[cfg(feature = "testkit")]
    const STALL: std::time::Duration = std::time::Duration::from_secs(30);

    #[cfg(feature = "testkit")]
    pub fn stall(point: &str) {
        if std::env::var(AT).ok().as_deref() != Some(point) {
            return;
        }
        if let Ok(path) = std::env::var(READY) {
            let _ = std::fs::write(path, point.as_bytes());
        }
        std::thread::sleep(STALL);
    }

    #[cfg(not(feature = "testkit"))]
    #[inline(always)]
    pub fn stall(_point: &str) {}

    #[cfg(feature = "testkit")]
    pub fn split_watermark() -> bool {
        std::env::var_os(SPLIT).is_some_and(|v| !v.is_empty())
    }

    #[cfg(not(feature = "testkit"))]
    #[inline(always)]
    pub fn split_watermark() -> bool {
        false
    }
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

        // LEFT JOIN, and the difference is the archive. An inner join returns
        // no row in two very different situations - this transcript has never
        // been ingested, and this transcript IS archived but lost its
        // `session_meta` row - and the caller reads "no row" as "brand new". A
        // pass over a grown transcript then wrote the TAIL as the whole blob,
        // destroying every archived byte before it, allocated a second
        // `session_no` that no longer matched the `sessions` row, and left the
        // watermark ahead of the committed blob: the one state STOR-02 says a
        // store must never reach, reached by a pass that exits 0. `verify` then
        // certified it, because the checksum had been minted over the truncated
        // stream - so the obvious response to `verify` naming a session (re-run
        // ingest) was what destroyed it.
        let session = conn
            .query_row(
                "SELECT s.session_no, s.blob, m.checksum
                 FROM sessions s LEFT JOIN session_meta m USING (session_key)
                 WHERE s.session_key = ?1",
                [session_key],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Option<Vec<u8>>>(2)?,
                    ))
                },
            )
            .optional()?;

        // An archived session with no metadata row is damaged, not absent.
        // Phase 1 refuses rather than guessing: there is no checksum to verify
        // the stored bytes against, and `blob::append` requires one precisely
        // so that a growing session cannot have corruption written into it.
        let session = match session {
            Some((_, _, None)) => {
                return Err(Error::NotAStore {
                    path: session_key.into(),
                    detail: "session is archived but has no session_meta row; \
                             run `verbatim verify` - refusing to overwrite its blob"
                        .into(),
                })
            }
            Some((session_no, blob, Some(checksum))) => Some((session_no, blob, checksum)),
            None => None,
        };

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
///
/// Through `discover::open_transcript` and not `File::open`, so the pass's
/// opens are the number AC5 measures: "zero opens of any file under an excluded
/// directory" has to be a fact a test can read off a log.
fn read_tail(path: &Path, watermark: u64) -> Result<Vec<u8>> {
    let mut file = crate::discover::open_transcript(path).map_err(|e| Error::io(path, e))?;
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
    file.read_to_end(&mut buffer)
        .map_err(|e| Error::io(path, e))?;
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
