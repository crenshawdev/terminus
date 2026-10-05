//! Ingest: one named transcript here, a whole tree in [`pass`], and the same
//! tree across a fixed set of worker threads in [`backfill`].
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

pub mod backfill;
pub mod lock;
pub mod pass;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub use lock::{Attempt, IngestLock, LOCK_FILE_NAME};

use rusqlite::{Connection, OptionalExtension};

use crate::blob;
use crate::config::CaptureMode;
use crate::derive;
use crate::error::{Error, Result};
use crate::lineage;
use crate::parse::{self, Scan};
use crate::project::{Project, Resolver};
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
    /// The transcript is inside a project the config excludes, named with the
    /// directory that excluded it. Nothing was opened and nothing was written.
    ///
    /// Only [`run`] produces this: `ingest_locked` is reached from the pass
    /// only for a file discovery already cleared.
    Excluded(PathBuf),
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

/// Ingest one transcript into the store under `data_dir`, honoring the
/// configured exclusions.
pub fn run(data_dir: &Path, transcript: &Path) -> Result<Outcome> {
    let config = crate::config::Config::load()?;
    run_with(data_dir, transcript, &config)
}

/// The single-file ingest, against a config the caller already has.
///
/// The seam tests use, mirroring [`pass::run_with`], so a test never has to
/// place a `terminus.toml` to say which projects are off limits.
pub fn run_with(
    data_dir: &Path,
    transcript: &Path,
    config: &crate::config::Config,
) -> Result<Outcome> {
    let started = Instant::now();

    // Before anything else, and resolving symlinks: `~/.claude` is a symlink
    // chain on the development machine (`DESIGN-BRIEF.md:406`), so the same
    // transcript reached by two paths would otherwise get two archive rows.
    // This resolves CONTEXT's third flagged assumption as "canonicalize".
    let canonical = transcript
        .canonicalize()
        .map_err(|e| Error::io(transcript, e))?;

    // Before the lock, and long before the open. The tree pass decides this at
    // the project directory it declines to descend into (D-22); named one file,
    // there is no walk to decide it, and skipping the test here would make
    // `terminus ingest <path>` the way around an exclusion the pass honors.
    if let Some(project) = crate::discover::excluded_project_of(config, &canonical) {
        return Ok(Outcome::Excluded(project));
    }

    let _guard = match lock::try_acquire(data_dir)? {
        Attempt::Held => return Ok(Outcome::LockHeld),
        Attempt::Acquired(guard) => guard,
    };

    // Recovery first, under the lock, before this transcript is read (ING-03).
    // `ingest::run` is a run: without this it would proceed against a watermark
    // ahead of what this file's committed blob holds, `read_tail` would resume
    // from an offset the archive never reached, and the gap would be lost from
    // the blob in silence. It also brings a store older than this build forward
    // (STOR-05), so a pass never appends turn rows in one shape beside rows
    // written in another.
    let (mut store, _recovered) = crate::recover::recover(data_dir)?;
    // One resolver for one invocation: a single file has one `cwd`, so the memo
    // buys nothing here and costs nothing either, and `ingest_locked` takes the
    // same shape whether its caller named one file or walked two thousand.
    let mut projects = Resolver::new();
    ingest_locked(
        &mut store,
        &canonical,
        started,
        RunRow::PerFile,
        &mut projects,
        config.capture_mode(),
    )
}

/// Who owns the `runs` row for this file's transaction.
///
/// D-10: one row per pass, not per file. `record_run` inserts a literal `1` for
/// `files_seen` from inside the per-file transaction, which is right for a
/// caller that named one file and wrong for a tree walk - reused unchanged it
/// would write roughly 2,071 rows per hook-triggered pass, and ING-09's "the
/// last ingest run" would name one file rather than the pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunRow {
    /// `ingest::run`: this file IS the run, and its row commits in the same
    /// transaction as the pass (STOR-02).
    PerFile,
    /// The tree pass owns the row and writes one after the whole walk.
    PassOwns,
}

/// The pass itself, with the lock already held and the store already open.
pub(crate) fn ingest_locked(
    store: &mut Store,
    path: &Path,
    started: Instant,
    run_row: RunRow,
    projects: &mut Resolver,
    mode: CaptureMode,
) -> Result<Outcome> {
    // The session is keyed on the transcript FILE identity, never on the
    // record's session id (D-01): 812 real sidecar files report their parent's
    // sessionId, and keying on that overwrites a parent session's blob with a
    // 3 KB agent transcript. The watermark is keyed on the same value.
    let session_key = path_key(path)?;
    let existing = Existing::read(store.conn(), &session_key)?;
    let prepared = prepare(path, session_key, &existing, mode)?;
    apply(store, prepared, started, run_row, projects)
}

/// Everything one file's pass produces before anything is written.
///
/// The split into [`prepare`] and [`apply`] is what makes D-11's backfill
/// possible: prepare touches no connection at all, so it can run on a worker
/// thread, and apply is the only half that writes, so one thread can own the
/// writable connection for the whole run. The sequential path composes them
/// back together in [`ingest_locked`] and behaves exactly as it did before -
/// same order, same faults, same one transaction.
pub(crate) struct Prepared {
    session_key: String,
    path: PathBuf,
    /// D-13's flag is clear again: the file was flagged and its archived prefix
    /// still matches. The clear is a WRITE, so it happens in [`apply`], and it
    /// happens whether or not there are new bytes - a flagged session with
    /// nothing new must still stop being flagged.
    clear_divergence: bool,
    /// `None` when there is no complete record past the watermark.
    work: Option<Work>,
}

/// The compressed, parsed result of reading one file's tail.
struct Work {
    /// Where the bytes this pass stores begin in the ARCHIVED stream: the
    /// uncompressed length the session's blob already holds, and zero for a
    /// session with no blob (ING-07, phase 8 D-05).
    ///
    /// Not the watermark. The watermark is a file offset and this is a stream
    /// offset; under `full` they are the same number and under every other mode
    /// they are not. Every `turns.stream_offset` this pass writes is measured
    /// from here, because that is the coordinate system `blob::BlobReader`,
    /// `recall::get` and `reindex` all read.
    stream_base: u64,
    /// The `session_no` this session already has, or `None` for one that has
    /// never been archived and needs the next free number - which only the
    /// writer can allocate.
    existing_session_no: Option<i64>,
    /// The scan of the STORED bytes, not of the file's. It is what the turn rows
    /// are derived from, so their offsets address what a reader will find.
    scan: Scan,
    /// The bytes this pass appends to the blob: the file's tail with whatever
    /// the capture mode elided already taken out of it.
    stored: Vec<u8>,
    bytes: Vec<u8>,
    checksum: [u8; 32],
    uncompressed_len: u64,
    parent_session_key: Option<String>,
    agent_meta: Option<Vec<u8>>,
    /// The mode this pass's bytes were captured under (ING-07, D-13). Carried
    /// through to `session_meta.capture_mode` rather than re-read at the write,
    /// so the column records the mode the blob in this same `Work` was built
    /// under and cannot drift from it.
    capture_mode: CaptureMode,
    pass: Pass,
}

/// Read, parse and compress one transcript's tail. Touches no connection.
///
/// Every expensive thing a pass does is here - the file read, the JSON scan and
/// the zstd compression - and none of the cheap ones that need the store.
pub(crate) fn prepare(
    path: &Path,
    session_key: String,
    existing: &Existing,
    mode: CaptureMode,
) -> Result<Prepared> {
    // Before the file is even opened, and the only safe answer for a session
    // retention emptied (RET-02, phase 8 D-03). `blob::append` parses a header
    // out of the stored bytes and an emptied blob has none, so left to the
    // ordinary path this file becomes a per-file failure on every pass forever;
    // writing the tail as a FRESH blob instead would resurrect a session that
    // was deliberately emptied and leave every stored `turns.stream_offset`
    // pointing into bytes that are no longer there.
    //
    // So: no work, and `apply` answers `Outcome::UpToDate`. No bytes are read,
    // the watermark does not move, and the walk records no failure - the file
    // is simply one this store has nothing more to do with until the policy
    // that emptied it changes. D-13's flag is not cleared either, and cannot
    // be: `prefix_matches` would have to decompress the blob that is gone.
    if existing.evicted {
        return Ok(Prepared {
            session_key,
            path: path.to_path_buf(),
            clear_divergence: false,
            work: None,
        });
    }

    let tail = read_tail(path, existing.watermark)?;

    // A flagged session stops being flagged only when the file is provably the
    // one that was archived (D-13). `read_tail` returning proves the file is no
    // shorter than the watermark and NOTHING about its contents: a transcript
    // truncated to half its length and then written back past the old watermark
    // passes that length test while every byte between the truncation point and
    // the watermark is different. Clearing on length alone would then append at
    // the stale offset - splicing a record fragment onto a blob whose middle no
    // longer exists anywhere, permanently, with `verify` reporting it clean
    // because the blob's own checksum still matches itself.
    //
    // So the archived prefix is compared against the file's. This costs a
    // decompression and a re-read of the session, and it is paid only by a
    // session that is actually flagged - the steady state reads the flag, finds
    // it clear, and issues neither.
    let mut clear_divergence = false;
    if existing.diverged {
        if !prefix_matches(path, existing, existing.watermark)? {
            // Still divergent, and now for a reason a length test cannot see.
            // Skipped exactly like the shortened case, flag left set.
            return Err(Error::TranscriptRewritten {
                path: path.to_path_buf(),
                watermark: existing.watermark,
            });
        }
        clear_divergence = true;
    }

    // The FIRST scan, over the raw tail, and its only job is to say where this
    // pass stops in the FILE. The watermark is a file offset - `read_tail` seeks
    // by it and `read_tail` refuses a file shorter than it - so it has to keep
    // being measured against the file, whatever the capture mode does to the
    // bytes afterwards. `consumed` is likewise file bytes, and is what
    // `runs.bytes_read` and `status` report.
    let file_scan = parse::scan_from(&tail, existing.watermark, existing.turn_count);
    let consumed = file_scan.consumed(existing.watermark);
    if consumed == 0 {
        // No complete record past the watermark. A rerun on an unchanged file
        // lands here, and adds no row to any table.
        return Ok(Prepared {
            session_key,
            path: path.to_path_buf(),
            clear_divergence,
            work: None,
        });
    }
    let fresh = &tail[..consumed];

    // The capture mode acts HERE, between the file and the blob (ING-07, phase 8
    // D-05), line by line and preserving the framing exactly. Under `full` this
    // borrows `fresh` and examines not one byte of it, which is what keeps the
    // default path identical to what it has always been.
    let stored = crate::capture::elide_stream(fresh, mode);

    // Where those bytes land in the archived stream. It is the blob's own
    // uncompressed length and NOT the watermark: `blob::append` appends at the
    // end of the blob, so that is where the new bytes really go, and once a mode
    // elides anything the file offset stops being able to say so. Under `full`
    // the two are the same number in a healthy store, which is why every offset
    // below is arithmetically what it was before this existed.
    let stream_base = if existing.session.is_some() {
        existing.uncompressed_len
    } else {
        0
    };

    // The SECOND scan, over the bytes actually being stored, and it is the one
    // `apply` derives turns from - so `turns.stream_offset` and `byte_len`
    // address the stored stream, which is what `blob::BlobReader`,
    // `recall::get` and `reindex` read. Elision preserves the line framing and
    // every field a record is classified on, so this scan finds the same records
    // in the same order at the same ordinals; only their lengths differ.
    let scan = parse::scan_from(&stored, stream_base, existing.turn_count);

    // Compression and hashing happen here, outside the transaction. `append`
    // takes the checksum already recorded for what the blob holds and verifies
    // it before writing, so a re-ingest of a growing session cannot mint a
    // fresh checksum over corruption.
    let (bytes, checksum, uncompressed_len) = match &existing.session {
        Some(session) => {
            let appended = blob::append(&session.blob, &session.checksum, &stored)?;
            (appended.bytes, appended.checksum, appended.uncompressed_len)
        }
        None => {
            let written = blob::write(&stored)?;
            (written.bytes, written.checksum, written.uncompressed_len)
        }
    };

    // D-03: a sidecar's parent comes from its path and never from its records,
    // which all report the parent's `sessionId` and so distinguish nothing. The
    // path is already the canonical one the walk built, so the derived value is
    // exactly the key the parent's own ingest wrote - whether or not the parent
    // has been ingested yet.
    let parent_session_key =
        lineage::sidecar_parent(path).map(|p| p.to_string_lossy().into_owned());

    // Read once per session, not once per pass: the bytes never change, and a
    // steady-state pass over 818 real sidecars would otherwise reopen 818 meta
    // files to store what it already has.
    let agent_meta = if existing.agent_meta_stored {
        None
    } else {
        read_agent_meta(path)
    };

    // Both numbers here are the FILE's, and deliberately: `bytes_read` is what
    // this pass read past the watermark, and `watermark` is where the next pass
    // seeks to. Only `turns_added` comes off the stored scan, and the two scans
    // agree about it by construction.
    let pass = Pass {
        session_key: session_key.clone(),
        bytes_read: consumed as u64,
        turns_added: scan.turn_count(),
        watermark: file_scan.resume_offset,
    };

    fault::stall(fault::AFTER_BLOB);

    Ok(Prepared {
        path: path.to_path_buf(),
        clear_divergence,
        work: Some(Work {
            stream_base,
            existing_session_no: existing.session.as_ref().map(|s| s.session_no),
            scan,
            stored: stored.into_owned(),
            bytes,
            checksum,
            uncompressed_len,
            parent_session_key,
            agent_meta,
            capture_mode: mode,
            pass,
        }),
        session_key,
    })
}

/// Write what [`prepare`] produced, in one transaction (STOR-02).
///
/// The only half that touches the store, and the only half a backfill runs on
/// its writer thread. Everything it does that is not the transaction itself
/// needs the connection: allocating a `session_no`, resolving a predecessor,
/// clearing D-13's flag.
pub(crate) fn apply(
    store: &mut Store,
    prepared: Prepared,
    started: Instant,
    run_row: RunRow,
    projects: &mut Resolver,
) -> Result<Outcome> {
    let Prepared {
        session_key,
        path,
        clear_divergence,
        work,
    } = prepared;

    // Before anything else, and on both arms: a session that is whole again
    // stops being flagged even when the pass has nothing new to add (D-13).
    if clear_divergence {
        set_divergence(store.conn(), &session_key, false)?;
    }

    let Some(work) = work else {
        return Ok(Outcome::UpToDate);
    };
    let Work {
        stream_base,
        existing_session_no,
        scan,
        stored,
        bytes,
        checksum,
        uncompressed_len,
        parent_session_key,
        agent_meta,
        capture_mode,
        pass,
    } = work;
    let path = path.as_path();

    let session_no = match existing_session_no {
        Some(session_no) => session_no,
        None => next_session_no(store.conn())?,
    };
    let continues_from = continues_from(store.conn(), &session_key, &scan)?;

    // D-20: the project comes from the session's FIRST `cwd` record, and the
    // upsert below coalesces it, so a tail pass never revises what the first
    // pass established - 19 of 1,253 real transcripts carry more than one
    // distinct `cwd` and belong to the directory the session started in.
    // Resolution happens here, on the writer, and outside the transaction: it
    // may spawn git, and a write transaction held across a subprocess would
    // stop MCP readers. Keeping it off the workers also keeps the memo a plain
    // `&mut Resolver` rather than a lock every worker would queue on for the
    // 63 distinct `cwd` values a whole real corpus has.
    let project = first_cwd(&scan).map(|cwd| projects.resolve(&cwd));

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
        MetaRow {
            session_key: &session_key,
            path,
            checksum: &checksum,
            uncompressed_len,
            continues_from: continues_from.as_deref(),
            project: project.as_ref(),
            parent_session_key: parent_session_key.as_deref(),
            agent_meta: agent_meta.as_deref(),
            capture_mode,
            scan: &scan,
        },
    )?;
    fault::stall(fault::IN_TX_AFTER_SESSION);
    for (record, turn) in scan.turns() {
        // The seam, not an insert of our own: the rebuild path calls the same
        // function, which is what stops it from drifting from what ingest wrote
        // (STOR-04). `stored` starts at `stream_base`, so a record's stream
        // offset has to be rebased to index it - and the offset and the buffer
        // are in the SAME coordinate system, which is the whole reason the
        // second scan exists.
        let from = (record.offset - stream_base) as usize;
        derive::derive_turn(
            &tx,
            derive::TurnRow {
                session_key: &session_key,
                session_no,
                turn,
                stream_offset: record.offset,
                byte_len: record.len,
                record: &stored[from..from + record.len as usize],
                subtype: record.subtype.as_deref(),
                compact_metadata: record.compact_metadata.as_deref(),
            },
        )?;
    }
    fault::stall(fault::IN_TX_AFTER_TURNS);
    write_watermark(&tx, &session_key, pass.watermark)?;
    if run_row == RunRow::PerFile {
        record_run(&tx, &pass, started.elapsed())?;
    }
    fault::stall(fault::IN_TX_BEFORE_COMMIT);
    tx.commit()?;
    fault::stall(fault::AFTER_COMMIT);

    Ok(Outcome::Committed(pass))
}

/// The `agent-*.meta.json` that would sit beside a sidecar transcript (D-04).
///
/// Same directory, same stem, a different extension - which is exactly why
/// discovery's filename filter never picks it up and it can never become a
/// session of its own (D-16). `None` for anything that is not an
/// `agent-*.jsonl`.
fn agent_meta_path(transcript: &Path) -> Option<PathBuf> {
    let stem = transcript
        .file_name()?
        .to_str()?
        .strip_suffix(".jsonl")?
        .to_owned();
    if !stem.starts_with("agent-") {
        return None;
    }
    Some(transcript.with_file_name(format!("{stem}.meta.json")))
}

/// A sidecar's meta bytes, read verbatim and never parsed (D-04).
///
/// 816 such files carry `agentType`, `description`, `toolUseId`, `spawnDepth`
/// and `model` against 818 sidecar transcripts, so a missing one is normal:
/// the column stays null and nothing is recorded. The format is undocumented
/// and may drift, which is the whole reason the bytes are stored rather than
/// typed columns - phase 3 can extract `description` for ranking without a
/// reingest, and the blob stays transcript bytes only (D-13).
///
/// Through the counted open, like every other read of the transcript tree, so
/// an excluded project's meta files are counted the same way its transcripts
/// are. Any read error is the same answer as an absent file: this is metadata
/// about a session, not the session, and it must not fail the file.
fn read_agent_meta(transcript: &Path) -> Option<Vec<u8>> {
    let path = agent_meta_path(transcript)?;
    let mut file = crate::discover::open_transcript(&path).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Raise D-13's divergence flag on the session this transcript belongs to.
///
/// Called by the pass, from outside any transaction: the file's own transaction
/// has already rolled back by then, and this records a fact about the file on
/// disk rather than about the archive - which is untouched, and is the point.
///
/// The path is the one `read_tail` refused, so it has already been through
/// [`path_key`] and is UTF-8 by construction; deriving the key here rather than
/// taking it keeps one definition of what keys a session (D-01).
pub(crate) fn flag_divergence(conn: &Connection, path: &Path) -> Result<()> {
    let session_key = path_key(path)?;
    set_divergence(conn, &session_key, true)
}

/// `session_meta.transcript_diverged`, both directions.
///
/// Nulled rather than zeroed when it clears, so the column has two states and
/// not three and every reader can test it the same way.
fn set_divergence(conn: &Connection, session_key: &str, diverged: bool) -> Result<()> {
    conn.execute(
        "UPDATE session_meta SET transcript_diverged = ?2 WHERE session_key = ?1",
        rusqlite::params![session_key, diverged.then_some(1)],
    )?;
    Ok(())
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

    /// The pass level, and the only point that is about the *walk* rather than
    /// about one file: [`AFTER_FILES_COUNT`] files have committed and the next
    /// has not been opened.
    ///
    /// It is not in [`POINTS`], because it is not interchangeable with them: it
    /// needs a count as well as a name, and a pass that never commits that many
    /// files never reaches it. The per-file points still fire underneath it, in
    /// whichever file is in flight, so between the two a kill can be aimed at
    /// any (file, moment) pair rather than only at a moment.
    pub const AFTER_FILES: &str = "after-files";

    /// How many files must have committed before [`AFTER_FILES`] fires.
    ///
    /// An environment variable and not a file in the data directory, unlike
    /// [`PASS_FAIL_AFTER_FILE`]: this point is reached through [`stall`], which
    /// is already aimed by [`AT`] on the child's own environment, so the count
    /// travels the same way the name does and a fault aimed at one child still
    /// cannot reach another.
    pub const AFTER_FILES_COUNT: &str = "TERMINUS_FAULT_AFTER_FILES";

    /// A transcript with this file name makes the worker that prepares it
    /// panic, which is how "one worker panicking does not wedge the pass"
    /// becomes testable.
    ///
    /// The fault rides the transcript's own NAME rather than an environment
    /// variable or a file in the data directory. A worker holds the path it was
    /// handed and nothing else - no data directory, no config - so a file-based
    /// fault has nowhere to be read from; and an env var is process-global,
    /// which would let a fault aimed at one test reach another running beside it
    /// in the same binary. A name is scoped to the one tree a test built.
    ///
    /// It is spelled as a sidecar (`agent-*.jsonl`) because `discover` admits a
    /// file on its NAME alone (D-16) - a `<uuid>.jsonl` at project depth, or an
    /// `agent-*.jsonl` below one, and nothing else. A fault file named anything
    /// more legible is never discovered, so it is never walked, so it never
    /// reaches a worker and the test that armed it passes vacuously.
    pub const PANIC_PREPARING: &str = "agent-fault-panic-preparing.jsonl";

    /// Panic if this is the transcript a test armed. Inert without the feature,
    /// and inert on every other name.
    #[cfg(feature = "testkit")]
    pub fn panic_preparing(path: &std::path::Path) {
        if path.file_name().is_some_and(|name| name == PANIC_PREPARING) {
            panic!("fault: preparing {} panicked on purpose", path.display());
        }
    }

    #[cfg(not(feature = "testkit"))]
    #[inline(always)]
    pub fn panic_preparing(_path: &std::path::Path) {}

    /// Stall the walk once `committed` files have committed, if a kill is aimed
    /// there. Inert without the count, and inert without [`AT`].
    #[cfg(feature = "testkit")]
    pub fn stall_after_files(committed: usize) {
        let aimed: Option<usize> = std::env::var(AFTER_FILES_COUNT)
            .ok()
            .and_then(|v| v.parse().ok());
        if aimed == Some(committed) {
            stall(AFTER_FILES);
        }
    }

    #[cfg(not(feature = "testkit"))]
    #[inline(always)]
    pub fn stall_after_files(_committed: usize) {}

    /// A file inside the data directory holding a file count. The tree pass
    /// fails its walk once it has walked that many transcripts, which is how
    /// D-14's "a pass that died still leaves its `runs` row" becomes testable.
    ///
    /// A file rather than an environment variable on purpose: every test owns
    /// its own data directory, so a fault aimed at one pass cannot reach a test
    /// running beside it. An env var is process-global and would.
    pub const PASS_FAIL_AFTER_FILE: &str = "FAULT_PASS_FAIL_AFTER";

    /// How many files the pass may walk before it fails, if a fault is armed.
    #[cfg(feature = "testkit")]
    pub fn pass_fails_after(data_dir: &std::path::Path) -> Option<usize> {
        std::fs::read_to_string(data_dir.join(PASS_FAIL_AFTER_FILE))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    #[cfg(not(feature = "testkit"))]
    #[inline(always)]
    pub fn pass_fails_after(_data_dir: &std::path::Path) -> Option<usize> {
        None
    }

    /// Every point a kill may be aimed at, in the order a pass reaches them.
    pub const POINTS: &[&str] = &[
        AFTER_BLOB,
        IN_TX_AFTER_SESSION,
        IN_TX_AFTER_TURNS,
        IN_TX_BEFORE_COMMIT,
        AFTER_COMMIT,
    ];

    /// Names the point to stall at.
    pub const AT: &str = "TERMINUS_FAULT_AT";
    /// Names a file to create on arrival, which is what the parent polls for.
    pub const READY: &str = "TERMINUS_FAULT_READY";
    /// Moves the watermark into its own earlier transaction.
    pub const SPLIT: &str = "TERMINUS_FAULT_SPLIT_WATERMARK";

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
///
/// Read by the writer - it is four queries - and handed to [`prepare`], which
/// is what lets prepare run on a thread with no connection of its own.
pub(crate) struct Existing {
    watermark: u64,
    /// Whether `session_meta.agent_meta` already holds this sidecar's meta
    /// bytes, so a steady-state pass reopens nothing to rewrite them (D-04).
    agent_meta_stored: bool,
    /// Turns already stored for this session, which is the ordinal the next one
    /// takes: a tail pass numbers on from here and a rebuild from 0 lands on
    /// the same numbers, because both count in byte order (D-02).
    turn_count: i64,
    /// Whether a previous pass found this transcript shorter than its watermark
    /// and flagged the session (D-13). Read so the flag can be *cleared* when
    /// the file is whole again, and read here so the steady state costs no
    /// extra query and no write at all.
    diverged: bool,
    /// Whether retention emptied this session's blob on purpose (RET-02, phase
    /// 8 D-03). Read off the same `session_meta` row as the two flags above, so
    /// the steady state still costs one query and no write.
    evicted: bool,
    /// How many uncompressed bytes this session's blob already holds - the base
    /// of the STORED stream's coordinate system (ING-07, phase 8 D-05).
    ///
    /// Not the same number as [`Existing::watermark`] once a capture mode elides
    /// anything: the watermark is an offset into the transcript FILE and this is
    /// an offset into the archived stream, and elision is exactly what makes the
    /// two diverge. Under `full` they are equal, which is why nothing needed
    /// this column before. Read off the same `session_meta` row as the flags
    /// above, so it costs no extra query.
    uncompressed_len: u64,
    /// The mode this session's STORED bytes were captured under, as
    /// `session_meta.capture_mode` holds it - null for a session written before
    /// the column existed, which means `full`.
    ///
    /// Read for one question only: whether the archived prefix and the file's
    /// prefix are comparable at all. See [`prefix_matches`].
    stored_capture_mode: Option<String>,
    session: Option<ExistingSession>,
}

struct ExistingSession {
    session_no: i64,
    blob: Vec<u8>,
    checksum: [u8; 32],
}

impl Existing {
    pub(crate) fn read(conn: &Connection, session_key: &str) -> Result<Existing> {
        let watermark: Option<i64> = conn
            .query_row(
                "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
                [session_key],
                |r| r.get(0),
            )
            .optional()?;
        // All three flags off the one `session_meta` row rather than three
        // round trips: a pass reads this for every one of two thousand
        // transcripts.
        let (agent_meta_stored, diverged, evicted, uncompressed_len, stored_capture_mode): (
            bool,
            bool,
            bool,
            i64,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT agent_meta IS NOT NULL, coalesce(transcript_diverged, 0) <> 0,
                        coalesce(is_evicted, 0) <> 0, uncompressed_len, capture_mode
                 FROM session_meta WHERE session_key = ?1",
                [session_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?
            .unwrap_or((false, false, false, 0, None));
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
                             run `terminus verify` - refusing to overwrite its blob"
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
            agent_meta_stored,
            turn_count,
            diverged,
            evicted,
            uncompressed_len: uncompressed_len.max(0) as u64,
            stored_capture_mode,
            session,
        })
    }
}

/// The session key: the canonical path, as text.
pub(crate) fn path_key(path: &Path) -> Result<String> {
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

/// Are the file's first `watermark` bytes the ones already archived?
///
/// The question a length test cannot answer, asked only of a session that is
/// already flagged. `false` means the file at this path is a different file
/// that happens to be long enough - truncated and rewritten, or replaced
/// outright - and the archive must not be appended to at the stale offset.
///
/// A session with no archived blob cannot disagree with one, so it is treated
/// as matching: the flag was set for a session whose bytes are gone, and
/// refusing forever would be a permanent failure with nothing to compare.
///
/// A session NOT captured under `full` answers `false` (ING-07, phase 8 D-05).
/// Its archived stream and the file's first `watermark` bytes are not the same
/// bytes by construction - elision is what makes them differ - so the comparison
/// cannot say anything, and the safe answer is the one the surrounding code
/// already chooses: still divergent. The flag stays set, the file stays skipped,
/// and `terminus verify` keeps naming it. The alternative is appending at a
/// stale offset onto a blob whose middle no longer exists anywhere, with
/// `verify` reporting it clean because the blob's checksum still matches itself.
fn prefix_matches(path: &Path, existing: &Existing, watermark: u64) -> Result<bool> {
    let Some(session) = existing.session.as_ref() else {
        return Ok(true);
    };
    // Null is `full`: a row written before the column existed was written by a
    // binary that had no other mode.
    if !matches!(existing.stored_capture_mode.as_deref(), None | Some("full")) {
        return Ok(false);
    }
    let archived = crate::blob::read_all(&session.blob)?;
    if (archived.len() as u64) < watermark {
        // The blob is shorter than the watermark claims. That is a store
        // inconsistency rather than a transcript one, and `verify` is the
        // command that reports it - here it simply is not a match.
        return Ok(false);
    }

    let mut file = crate::discover::open_transcript(path).map_err(|e| Error::io(path, e))?;
    let mut head = vec![0u8; watermark as usize];
    file.read_exact(&mut head).map_err(|e| Error::io(path, e))?;
    Ok(head == archived[..watermark as usize])
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
        // blob - so the refusal stands and there is deliberately no fallback to
        // offset 0, which would overwrite archived bytes on the strength of an
        // append-only property observed over one 25-day corpus rather than
        // documented anywhere (D-13).
        //
        // What phase 2 adds is not a recovery but a signal: this variant is the
        // one the pass matches on to flag the session, so `terminus verify`
        // can say the transcript and the archive have diverged.
        return Err(Error::TranscriptDiverged {
            path: path.to_path_buf(),
            len,
            watermark,
        });
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

/// D-11's lineage signal, in the session-id namespace ([`lineage`]).
///
/// The record's foreign `session_id` field is the reliable one: 169 of 1,253
/// real top-level transcripts name another session that way, an order of
/// magnitude above the 1.2% `parentUuid` resume rate. `parentUuid` is the
/// fallback and only that - it is message-level threading, so it is resolved
/// through `turns` to the session that holds the parent record, keeping this
/// column in one namespace.
///
/// Both paths reject a link to this transcript's own session id (D-01).
fn continues_from(conn: &Connection, session_key: &str, scan: &Scan) -> Result<Option<String>> {
    if let Some(foreign) = lineage::foreign_predecessor(scan) {
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

    let resolved: Option<String> = conn
        .query_row(
            "SELECT m.session_id
             FROM turns t JOIN session_meta m USING (session_key)
             WHERE t.uuid = ?1 AND t.session_key <> ?2
             LIMIT 1",
            rusqlite::params![parent_uuid, session_key],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();

    // A different FILE may still report the same session id - every sidecar
    // reports its parent's - so the self-link test applies to the fallback too.
    Ok(resolved.filter(|id| !lineage::is_self_link(id, lineage::own_session_id(scan))))
}

/// Everything one `session_meta` upsert writes.
///
/// A struct rather than ten positional arguments: every phase adds a column to
/// this table, and a caller that passes `checksum` where `path` belongs still
/// compiles when both are borrowed strings.
struct MetaRow<'a> {
    session_key: &'a str,
    path: &'a Path,
    checksum: &'a [u8; 32],
    uncompressed_len: u64,
    /// In the session-id namespace, never the session-key one - see
    /// [`continues_from`].
    continues_from: Option<&'a str>,
    /// Resolved from the first `cwd` in this scan (D-20), or `None` for a
    /// transcript that carries no `cwd` at all. One real transcript does.
    project: Option<&'a Project>,
    /// The `session_key` of the transcript this sidecar ran under (D-03), or
    /// `None` for a top-level transcript. In the session-KEY namespace, unlike
    /// [`MetaRow::continues_from`].
    parent_session_key: Option<&'a str>,
    /// The bytes of the sidecar's `agent-*.meta.json`, stored opaquely (D-04).
    agent_meta: Option<&'a [u8]>,
    /// Which `[capture]` mode produced the bytes this pass is writing (ING-07,
    /// D-13). Not `coalesce`d like the columns above - see [`write_session_meta`].
    capture_mode: CaptureMode,
    scan: &'a Scan,
}

/// The session's first `cwd`, in byte order.
fn first_cwd(scan: &Scan) -> Option<String> {
    scan.records.iter().find_map(|r| r.cwd.clone())
}

/// Upsert the archive's metadata row.
///
/// Every column that describes the session as a whole is `coalesce`d onto what
/// is already there: a tail pass sees only the tail, and `session_id`, `cwd`,
/// `gitBranch`, the project, the sidecar's parent and the first turn's
/// timestamp were established by the first pass.
///
/// `is_final` is not written here and is not written by any ingest of a file:
/// it is the idle rule's, evaluated once at the end of a whole pass over
/// `last_turn_at` (D-06, [`crate::feedback::finalize`]), which is what lets one
/// mechanism cover a clean exit and a crash alike. `is_evicted` is not written
/// here either, and must not be: it is retention's (RET-02), and an ingest that
/// cleared it would un-evict a session on the pass after the one that emptied
/// it. `prepare` never reaches this function for an evicted session at all.
///
/// `capture_mode` is the one column that is neither `coalesce`d onto what is
/// there nor overwritten outright (ING-07, D-13). It reads `full` only while
/// EVERY append to the session has been full, so:
///
/// - a stored null or `full` is replaced by whatever this pass captured under;
/// - a stored `lean` or `minimal` is never moved back to `full` by a later full
///   pass, because `blob::append` copies completed blocks across untouched and
///   the bytes elided under the earlier mode are never revisited;
/// - a session appended to under two different REDUCED modes reads as the later
///   one. The guarantee the column carries is exactly "`full` means every append
///   was full", and a mixed reduced session has no single honest answer beyond
///   "not full".
fn write_session_meta(tx: &Connection, row: MetaRow<'_>) -> Result<()> {
    let scan = row.scan;
    let session_id = scan.records.iter().find_map(|r| r.session_id.clone());
    let cwd = first_cwd(scan);
    let branch = scan.records.iter().find_map(|r| r.git_branch.clone());
    // Byte order, not clock order (D-02): the "first" turn is the first in the
    // file even when its timestamp is later than its neighbour's.
    let first_turn_at = scan.turns().next().map(|(_, t)| t.timestamp.clone());
    let last_turn_at = scan.turns().last().map(|(_, t)| t.timestamp.clone());
    let project = row.project.map(|p| p.key.as_str());
    // Only when folding actually happened, so the column answers "was this a
    // worktree" as well as "which one" (D-06, ING-05).
    let pre_worktree = row.project.and_then(|p| p.pre_worktree.as_deref());

    tx.execute(
        "INSERT INTO session_meta (
            session_key, session_id, transcript_path, checksum, uncompressed_len,
            continues_from, first_turn_at, last_turn_at, cwd, branch,
            project, project_pre_worktree, parent_session_key, agent_meta,
            capture_mode
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(session_key) DO UPDATE SET
            session_id = coalesce(session_meta.session_id, excluded.session_id),
            transcript_path = excluded.transcript_path,
            checksum = excluded.checksum,
            uncompressed_len = excluded.uncompressed_len,
            continues_from = coalesce(session_meta.continues_from, excluded.continues_from),
            first_turn_at = coalesce(session_meta.first_turn_at, excluded.first_turn_at),
            last_turn_at = coalesce(excluded.last_turn_at, session_meta.last_turn_at),
            cwd = coalesce(session_meta.cwd, excluded.cwd),
            branch = coalesce(session_meta.branch, excluded.branch),
            project = coalesce(session_meta.project, excluded.project),
            project_pre_worktree = coalesce(
                session_meta.project_pre_worktree, excluded.project_pre_worktree),
            parent_session_key = coalesce(
                session_meta.parent_session_key, excluded.parent_session_key),
            agent_meta = coalesce(session_meta.agent_meta, excluded.agent_meta),
            capture_mode = CASE
                WHEN excluded.capture_mode = 'full'
                    THEN coalesce(session_meta.capture_mode, excluded.capture_mode)
                ELSE excluded.capture_mode
            END",
        rusqlite::params![
            row.session_key,
            session_id,
            row.path.to_string_lossy(),
            row.checksum.as_slice(),
            row.uncompressed_len as i64,
            row.continues_from,
            first_turn_at,
            last_turn_at,
            cwd,
            branch,
            project,
            pre_worktree,
            row.parent_session_key,
            row.agent_meta,
            row.capture_mode.as_str(),
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
