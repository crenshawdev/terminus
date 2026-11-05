//! Ingest: one named transcript here, a whole tree in [`pass`], and the same
//! tree across a fixed set of worker threads in [`backfill`].
//!
//! The order of the first steps is the design. The path is canonicalized before
//! anything else, the lock is taken **before** the store is opened, and the
//! file's bytes, turns and watermark commit in **one** transaction so a killed
//! process leaves no partially indexed session. The run's `runs` row is written
//! after that transaction, by the driver every pass runs through, so a kill
//! between the two leaves a committed file with no row and nothing worse.
//!
//! Reads happen outside that transaction and writes inside it, which is safe
//! for exactly one reason: the `LOCK` guard above is the only writer, so
//! nothing can change the store between the read and the write. Reading inside
//! the transaction instead would hold it open across compression, and a write
//! transaction held for a whole pass stops MCP readers - the reason redb was
//! rejected.

pub mod backfill;
mod driver;
pub mod lock;
pub mod pass;
pub mod runs;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub use lock::{Attempt, IngestLock, LOCK_FILE_NAME};

use rusqlite::{Connection, OptionalExtension};

use crate::blob::{self, Block, BLOCK_SIZE};
use crate::config::CaptureMode;
use crate::derive;
use crate::error::{Error, Result};
use crate::lineage;
use crate::parse::{self, Scan};
use crate::project::{Project, Resolver};
use crate::store::{archive, Store};

/// What one invocation of [`run`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Another process is mid-pass. Nothing was written, and this is a success:
    /// the hook spawn is the scheduler and a second invocation is expected.
    LockHeld,
    /// The transcript has no complete record past the stored watermark, or it
    /// grew after [`Existing::read`] found it no longer than the watermark, so
    /// its hashes were never fetched and the next pass appends it. Nothing is
    /// added to the archive; the run's one `runs` row records it as one file
    /// seen and none committed. The one archive write it can make is to
    /// the archived session's `session_meta.transcript_path`, and only when
    /// the file was read from somewhere else than the path stored there, so a
    /// moved file's row names where it is now.
    UpToDate,
    /// The transcript is inside a project the config excludes, named with the
    /// directory that excluded it. Nothing was opened and nothing was written.
    ///
    /// Only [`run`] produces this, before the lock is taken. The per-file
    /// result a walk matches, [`Ingested`], cannot carry it: a walk reaches a
    /// file only once discovery has cleared it.
    Excluded(PathBuf),
    /// The pass committed.
    Committed(Pass),
}

/// What one file's ingest did, with the lock already held.
///
/// Two arms and no more. A held lock and an exclusion are answers about the
/// run, decided before any file is read, so they live only on the public
/// [`Outcome`], and a walk that matches this has no arm to wave past for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ingested {
    /// The file's new bytes committed.
    Committed(Pass),
    /// Nothing new past the watermark; see [`Outcome::UpToDate`].
    UpToDate,
}

/// What a committed pass moved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pass {
    /// The session's key: the transcript's path below its project folder,
    /// `<uuid>.jsonl` or a sidecar's `<sessionId>/subagents/...` path.
    pub session_key: String,
    pub bytes_read: u64,
    pub turns_added: usize,
    /// Where the next pass resumes: the byte just past the last `\n`.
    pub watermark: u64,
    /// Existing block rows whose bytes were fetched for this file: at most the
    /// tail row, or every row of a session flagged diverged.
    /// Counted where the read happens, so a pass that read more than the tail
    /// shows up in this number and not only in its timing.
    pub block_rows_read: usize,
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
    // Before anything else, and resolving symlinks: `~/.claude` is a symlink
    // chain on the development machine, so the same transcript reached by two
    // paths would otherwise get two archive rows.
    let canonical = transcript
        .canonicalize()
        .map_err(|e| Error::io(transcript, e))?;

    // Before the lock, and long before the open. The tree pass decides this at
    // the project directory it declines to descend into; named one file,
    // there is no walk to decide it, and skipping the test here would make
    // `terminus ingest <path>` the way around an exclusion the pass honors.
    if let Some(project) = crate::discover::excluded_project_of(config, &canonical) {
        return Ok(Outcome::Excluded(project));
    }

    // A list of one, through the same driver as the tree pass: the lock,
    // recovery, and one `runs` row whether the file commits, is up to date or
    // fails. A failure comes back as its typed error once the row is written.
    let mut one = Named {
        path: &canonical,
        mode: config.capture_mode(),
        session_key: String::new(),
        ingested: Ingested::UpToDate,
    };
    Ok(match driver::drive(data_dir, &mut one)? {
        pass::PassOutcome::LockHeld => Outcome::LockHeld,
        pass::PassOutcome::Ran(_) => match one.ingested {
            Ingested::Committed(pass) => Outcome::Committed(pass),
            Ingested::UpToDate => Outcome::UpToDate,
        },
    })
}

/// One named file, as the driver walks it: that key's row only, and none of
/// the tree pass's other steps.
struct Named<'a> {
    /// The canonical path, never a link.
    path: &'a Path,
    mode: CaptureMode,
    /// What [`path_key`] made of `path`, once the lock is held.
    session_key: String,
    /// What the file did, once the walk has run.
    ingested: Ingested,
}

impl driver::Walk for Named<'_> {
    fn only(&mut self) -> Result<Option<String>> {
        // After the lock, so a path that cannot key a session is this file's
        // failure and leaves its row. This key's row only, read by the driver
        // after recovery, which rewrites rows; the walk and backfill read every
        // row through the same query, and `Existing::read` asks it which file
        // holds the key on all three paths.
        self.session_key = path_key(self.path)?;
        Ok(Some(self.session_key.clone()))
    }

    fn walk(&mut self, store: &mut Store, rows: &Rows, summary: &mut pass::Summary) -> Result<()> {
        summary.files_walked = 1;
        // One resolver for one invocation: a single file has one `cwd`, so the
        // memo buys nothing here and costs nothing either, and `ingest_locked`
        // takes the same shape whether its caller named one file or walked two
        // thousand.
        let ingested = ingest_locked(
            store,
            rows,
            self.path,
            std::mem::take(&mut self.session_key),
            &mut Resolver::new(),
            self.mode,
        )?;
        if let Ingested::Committed(pass) = &ingested {
            summary.files_committed = 1;
            summary.bytes_read = pass.bytes_read;
            summary.turns_added = pass.turns_added;
        }
        self.ingested = ingested;
        Ok(())
    }

    /// Anything that failed after recovery is this file's failure, counted as
    /// one failed file and not repeated as a `pass failed:` line. It sets no
    /// divergence flag: only the walk flags a divergence.
    fn failed(&mut self, summary: &mut pass::Summary, e: &Error) -> Option<String> {
        summary.files_walked = 1;
        summary
            .failures
            .push((self.path.to_path_buf(), e.to_string()));
        None
    }
}

/// A file named like an archived session whose file is gone, and whose first
/// bytes are not the ones archived. Not a divergence: the session's own file
/// may come back, and it must not find its session flagged.
fn not_the_holder(path: &Path, holder: &Path) -> Error {
    Error::io(
        path,
        std::io::Error::other(format!(
            "its session key is held by {}, which is gone, and this file does not \
             begin with the bytes archived from it, so it was not appended",
            holder.display()
        )),
    )
}

/// A second file whose key another file already holds. It is never opened.
pub(crate) fn key_held(path: &Path, holder: &Path) -> Error {
    Error::io(
        path,
        std::io::Error::other(format!(
            "its session key is already held by {}, so it was not read",
            holder.display()
        )),
    )
}

/// What a walk does with one discovered path.
pub(crate) enum Claim {
    /// The first path to yield this key: ingest `file`, the link's target or
    /// the path itself.
    Ingest { file: PathBuf, session_key: String },
    /// Another path to a file this walk already took.
    Same,
    /// A different file whose key an earlier one in this walk took.
    Held(Error),
}

/// Resolve a discovered path once and decide whether this walk ingests it.
///
/// Two paths can share a key: a sidecar link and the file it points at, or two
/// project folders, in one root or two, holding the same session file below the
/// project folder. The first path a walk meets takes the key, and a later one
/// naming a different file is refused without being opened, because ingest
/// appends from the watermark and would mix two files into one stream with
/// nothing to detect it.
///
/// The archived session's row, read at pass start, is asked before `taken` is:
/// a file the archived session was not read from, while that one is still
/// there, never takes the key, so the session's own file can, whichever of the
/// two sorts first.
pub(crate) fn claim(
    rows: &Rows,
    taken: &mut std::collections::HashMap<String, PathBuf>,
    path: &Path,
) -> Result<Claim> {
    let file = link_target(path).unwrap_or_else(|| path.to_path_buf());
    let session_key = path_key(&file)?;
    Ok(match taken.get(&session_key) {
        None => {
            if let Holder::Other(holder) = Holder::of(rows.get(&session_key), &file) {
                return Ok(Claim::Held(key_held(&file, &holder)));
            }
            taken.insert(session_key.clone(), file.clone());
            Claim::Ingest { file, session_key }
        }
        Some(holder) if *holder == file => Claim::Same,
        Some(holder) => Claim::Held(key_held(&file, holder)),
    })
}

/// The pass itself, with the lock already held and the store already open.
///
/// `path` is the file to read, never a link, and `session_key` is what
/// [`path_key`] made of it. The session is keyed on the transcript's path below
/// its project folder, never on the record's session id: 812 real sidecar files
/// report their parent's sessionId, and keying on that writes a 3 KB agent
/// transcript over a parent session's archive. The watermark is keyed on the
/// same value.
pub(crate) fn ingest_locked(
    store: &mut Store,
    rows: &Rows,
    path: &Path,
    session_key: String,
    projects: &mut Resolver,
    mode: CaptureMode,
) -> Result<Ingested> {
    let existing = Existing::read(store.conn(), rows, &session_key, path)?;
    let prepared = prepare(path, session_key, &existing, mode)?;
    apply(store, prepared, projects)
}

/// Everything one file's pass produces before anything is written.
///
/// The split into [`prepare`] and [`apply`] is what makes the parallel backfill
/// possible: prepare touches no connection at all, so it can run on a worker
/// thread, and apply is the only half that writes, so one thread can own the
/// writable connection for the whole run. The sequential path composes them
/// back together in [`ingest_locked`] and behaves exactly as it did before -
/// same order, same faults, same one transaction.
pub(crate) struct Prepared {
    session_key: String,
    path: PathBuf,
    /// The divergence flag is clear again: the file was flagged and its
    /// archived prefix still matches. The clear is a WRITE, so it happens in
    /// [`apply`], and it happens whether or not there are new bytes - a flagged
    /// session with nothing new must still stop being flagged.
    clear_divergence: bool,
    /// The session is archived and `session_meta.transcript_path` names some
    /// other path than this one. Only read when there is no `work`, because
    /// the meta write that comes with `work` stores the path anyway.
    moved: bool,
    /// `None` when there is no complete record past the watermark.
    work: Option<Work>,
}

/// The compressed, parsed result of reading one file's tail.
struct Work {
    /// Where the bytes this pass stores begin in the ARCHIVED stream: the
    /// uncompressed length the session already holds, and zero for a session
    /// never archived.
    ///
    /// Not the watermark. The watermark is a file offset and this is a stream
    /// offset; under `full` they are the same number and under every other mode
    /// they are not. Every `turns.stream_offset` this pass writes is measured
    /// from here, because that is the coordinate system `store::archive`,
    /// `recall::get` and `reindex` all read.
    stream_base: u64,
    /// The `session_no` this session already has, or `None` for one that has
    /// never been archived and needs the next free number - which only the
    /// writer can allocate.
    existing_session_no: Option<i64>,
    /// The scan of the STORED bytes, not of the file's. It is what the turn rows
    /// are derived from, so their offsets address what a reader will find.
    scan: Scan,
    /// The bytes this pass appends to the session: the file's tail with
    /// whatever the capture mode elided already taken out of it.
    stored: Vec<u8>,
    /// The block rows to write: the rewritten tail, if the stream ended
    /// mid-block, then the new blocks.
    blocks: Vec<Block>,
    /// The session's block size, written to `session_meta` on its first pass.
    block_size: u32,
    checksum: [u8; 32],
    uncompressed_len: u64,
    parent_session_key: Option<String>,
    agent_meta: Option<Vec<u8>>,
    /// The mode this pass's bytes were captured under. Carried through to
    /// `session_meta.capture_mode` rather than re-read at the write, so the
    /// column records the mode the blocks in this same `Work` were built under
    /// and cannot drift from it.
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
    // Before anything else, evicted or not. Two project folders, in one root or
    // two, can hold files with one key, and the tail read below checks a file
    // against the watermark by length alone, so a different file would be
    // appended onto the archived session.
    if let Holder::Other(holder) = &existing.holder {
        return Err(key_held(path, holder));
    }

    // Before the file is even opened, and the only safe answer for a session
    // retention emptied. An append checks the stored hashes and tail, and an
    // evicted session has no block rows left, so left to the ordinary path this
    // file becomes a per-file failure on every pass forever; writing the tail
    // as a FRESH session instead would resurrect one that was deliberately
    // emptied and leave every stored `turns.stream_offset` pointing into bytes
    // that are no longer there.
    //
    // So: no work, and `apply` answers `Outcome::UpToDate`. No bytes are read,
    // the watermark does not move, and the walk records no failure - the file
    // is simply one this store has nothing more to do with until the policy
    // that emptied it changes. The divergence flag is not cleared either, and
    // cannot be: `prefix_matches` would have to decompress the blocks that are
    // gone.
    if existing.evicted {
        return Ok(Prepared {
            session_key,
            path: path.to_path_buf(),
            clear_divergence: false,
            moved: existing.moved(path),
            work: None,
        });
    }

    // A flagged session stops being flagged only when the file is provably the
    // one that was archived. A length test proves the file is no shorter than
    // the watermark and NOTHING about its contents: a transcript truncated to
    // half its length and then written back past the old watermark passes it
    // while every byte between the truncation point and the watermark is
    // different. Clearing on length alone would then append at the stale offset
    // - splicing a record fragment onto a session whose middle no longer exists
    // anywhere, permanently, with `verify` reporting it clean because the
    // session's own checksum still matches itself.
    //
    // So the archived prefix is compared against the file's. This costs every
    // block row and a read of the whole file, and it is paid only by a session
    // that is actually flagged - the steady state reads the flag, finds it
    // clear, and fetches at most the tail row of a file that grew, and no row
    // of one that did not (#85).
    //
    // A file standing in for one that is gone gets the same comparison: a moved
    // transcript begins with the archived bytes, and any other file named like
    // it does not. Its refusal flags nothing, because the archive still matches
    // the file it was read from.
    let tail = if existing.diverged {
        read_flagged(path, existing)?
    } else if let Holder::Gone(holder) = &existing.holder {
        read_flagged(path, existing).map_err(|e| match e {
            Error::TranscriptDiverged { .. } | Error::TranscriptRewritten { .. } => {
                not_the_holder(path, holder)
            }
            e => e,
        })?
    } else {
        read_tail(path, existing.watermark)?
    };
    let clear_divergence = existing.diverged;

    // Where this pass stops in the FILE: `parse::complete_len`, the resume rule
    // its scan applies too. A search and not a parse, because nothing else
    // from the raw tail is used. The watermark is a file offset - the tail read
    // seeks by it and `open_at_least` refuses a file shorter than it - so it
    // has to keep being measured against the file, whatever the capture mode
    // does to the bytes afterwards. `consumed` is likewise file bytes, and is
    // what `runs.bytes_read` and `status` report.
    let consumed = parse::complete_len(&tail);
    // No complete record past the watermark, which is where a rerun on an
    // unchanged file lands. Or there is one, and the file grew after
    // `Existing::read` found it no longer than the watermark, so nothing an
    // append needs was fetched: it is left for the next pass, which fetches it,
    // rather than appended without its stored hashes. Either way no row is
    // added to any table.
    let Some(fetched) = existing.fetched.as_ref().filter(|_| consumed > 0) else {
        return Ok(Prepared {
            session_key,
            path: path.to_path_buf(),
            clear_divergence,
            moved: existing.moved(path),
            work: None,
        });
    };
    let fresh = &tail[..consumed];

    // The capture mode acts HERE, between the file and the archive, line by
    // line and preserving the framing exactly. Under `full` this borrows
    // `fresh` and examines not one byte of it, which is what keeps the default
    // path identical to what it has always been.
    let stored = crate::capture::elide_stream(fresh, mode);

    // Where those bytes land in the archived stream. It is the session's own
    // uncompressed length and NOT the watermark: an append adds at the end of
    // the stream, so that is where the new bytes really go, and once a mode
    // elides anything the file offset stops being able to say so. Under `full`
    // the two are the same number in a healthy store, which is why every offset
    // below is arithmetically what it was before this existed.
    let stream_base = if existing.session.is_some() {
        existing.uncompressed_len
    } else {
        0
    };

    // The one scan, over the bytes actually being stored, not the file's: `apply`
    // derives turns from it, so `turns.stream_offset` and `byte_len`
    // address the stored stream, which is what `store::archive`, `recall::get`
    // and `reindex` read. Elision preserves the line framing and
    // every field a record is classified on, so this scan finds the same records
    // in the same order at the same ordinals; only their lengths differ.
    let scan = parse::scan_from(&stored, stream_base, fetched.next_turn);

    // Compression and hashing happen here, outside the transaction. The append
    // is handed the stored hashes, the recorded checksum and the tail row, and
    // checks the first two against each other and the tail against its hash
    // before cutting anything, so a re-ingest of a growing session cannot mint
    // a fresh checksum over corruption. A session keeps the block size it was
    // first cut at; a new one is cut at this build's.
    let (written, block_size) = match &existing.session {
        Some(session) => {
            let block_size = archive::block_size(existing.block_size)?;
            let hashes = fetched
                .archive
                .hashes
                .as_ref()
                .map_err(|detail| Error::BlobFormat {
                    detail: detail.clone(),
                })?;
            let appended = blob::append(
                block_size,
                existing.uncompressed_len,
                hashes,
                &session.checksum,
                fetched.archive.tail.as_deref(),
                &stored,
            )?;
            (appended, block_size)
        }
        None => {
            let block_size = BLOCK_SIZE as u32;
            (blob::write(&stored, block_size)?, block_size)
        }
    };

    // A sidecar's parent comes from its path and never from its records, which
    // all report the parent's `sessionId` and so distinguish nothing. The
    // parent's path goes through the same key function the parent's own ingest
    // uses, so the two agree whether or not the parent has been ingested yet.
    let parent_session_key = lineage::sidecar_parent(path).and_then(|p| path_key(&p).ok());

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
    // seeks to. Only `turns_added` comes off the stored scan.
    let pass = Pass {
        session_key: session_key.clone(),
        bytes_read: consumed as u64,
        turns_added: scan.turn_count(),
        watermark: existing.watermark + consumed as u64,
        block_rows_read: fetched.archive.rows_read,
    };

    fault::stall(fault::AFTER_BLOB);

    Ok(Prepared {
        path: path.to_path_buf(),
        clear_divergence,
        moved: false,
        work: Some(Work {
            stream_base,
            existing_session_no: existing.session.as_ref().map(|s| s.session_no),
            scan,
            stored: stored.into_owned(),
            blocks: written.blocks,
            block_size,
            checksum: written.checksum,
            uncompressed_len: written.uncompressed_len,
            parent_session_key,
            agent_meta,
            capture_mode: mode,
            pass,
        }),
        session_key,
    })
}

/// Write what [`prepare`] produced, in one transaction.
///
/// The only half that touches the store, and the only half a backfill runs on
/// its writer thread. Everything it does that is not the transaction itself
/// needs the connection: allocating a `session_no`, resolving a predecessor,
/// clearing the divergence flag.
pub(crate) fn apply(
    store: &mut Store,
    prepared: Prepared,
    projects: &mut Resolver,
) -> Result<Ingested> {
    let Prepared {
        session_key,
        path,
        clear_divergence,
        moved,
        work,
    } = prepared;

    // Before anything else, and on both arms: a session that is whole again
    // stops being flagged even when the pass has nothing new to add.
    if clear_divergence {
        set_divergence(store.conn(), &session_key, false)?;
    }

    let Some(work) = work else {
        // Nothing new, but the file may have moved, as when `.claude` moves
        // with no link left behind. Retention asks whether the stored path is
        // gone, so it has to name the file that is still on disk.
        if moved {
            store.conn().execute(
                "UPDATE session_meta SET transcript_path = ?2 WHERE session_key = ?1",
                rusqlite::params![session_key, path.to_string_lossy()],
            )?;
        }
        return Ok(Ingested::UpToDate);
    };
    let Work {
        stream_base,
        existing_session_no,
        scan,
        stored,
        blocks,
        block_size,
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

    // The project comes from the session's FIRST `cwd` record, and the upsert
    // below coalesces it, so a tail pass never revises what the first pass
    // established - 19 of 1,253 real transcripts carry more than one distinct
    // `cwd` and belong to the directory the session started in. Resolution
    // happens here, on the writer, and outside the transaction: it may spawn
    // git, and a write transaction held across a subprocess would stop MCP
    // readers. Keeping it off the workers also keeps the memo a plain `&mut
    // Resolver` rather than a lock every worker would queue on for the 63
    // distinct `cwd` values a whole real corpus has.
    let project = first_cwd(&scan).map(|cwd| projects.resolve(&cwd));

    // The fault the crash harness must be able to catch, and the reason that
    // harness is worth anything: with this on, the watermark commits in a
    // transaction of its own *before* the pass, so a kill in between leaves a
    // store claiming bytes its block rows do not hold. Compiled only under
    // `testkit`; the shipped binary has no such branch.
    if fault::split_watermark() {
        let early = store.conn_mut().transaction()?;
        write_watermark(&early, &session_key, pass.watermark)?;
        early.commit()?;
        fault::stall(fault::AFTER_SPLIT_WATERMARK);
    }

    // One transaction, and nothing outside it mutates the store. The `sessions`
    // row goes in before its blocks, because the block table's reference to it
    // is enforced.
    let tx = store.conn_mut().transaction()?;
    if existing_session_no.is_none() {
        tx.execute(
            "INSERT INTO sessions (session_key, session_no) VALUES (?1, ?2)",
            rusqlite::params![session_key, session_no],
        )?;
    }
    archive::write(&tx, session_no, &blocks)?;
    write_session_meta(
        &tx,
        MetaRow {
            session_key: &session_key,
            path,
            checksum: &checksum,
            uncompressed_len,
            block_size,
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
        // function, which is what stops it from drifting from what ingest
        // wrote. `stored` starts at `stream_base`, so a record's stream offset
        // has to be rebased to index it - and the offset and the buffer are in
        // the SAME coordinate system, which is the whole reason the scan is of
        // the stored bytes and not the file's.
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
    // A boundary with no `uuid` or `timestamp` is not a turn, so the loop above
    // never sees it. Its row goes in this same transaction, at its offset in
    // the stored stream, which is the coordinate `derive_turn` got.
    for record in &scan.records {
        derive::derive_boundary(&tx, &session_key, record)?;
        derive::derive_title(&tx, &session_key, record)?;
    }
    fault::stall(fault::IN_TX_AFTER_TURNS);
    write_watermark(&tx, &session_key, pass.watermark)?;
    fault::stall(fault::IN_TX_BEFORE_COMMIT);
    tx.commit()?;
    fault::stall(fault::AFTER_COMMIT);

    Ok(Ingested::Committed(pass))
}

/// The `agent-*.meta.json` that would sit beside a sidecar transcript.
///
/// Same directory, same stem, a different extension - which is exactly why
/// discovery's filename filter never picks it up and it can never become a
/// session of its own. `None` for anything that is not an `agent-*.jsonl`.
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

/// A sidecar's meta bytes, read verbatim and never parsed.
///
/// 816 such files carry `agentType`, `description`, `toolUseId`, `spawnDepth`
/// and `model` against 818 sidecar transcripts, so a missing one is normal: the
/// column stays null and nothing is recorded. The format is undocumented and
/// may drift, which is the whole reason the bytes are stored rather than typed
/// columns - nothing parses them, `description` can still be extracted later
/// without a reingest, and the archive stays transcript bytes only.
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

/// Raise the divergence flag on the session this transcript belongs to.
///
/// Called by the pass, from outside any transaction: the file's own transaction
/// has already rolled back by then, and this records a fact about the file on
/// disk rather than about the archive - which is untouched, and is the point.
///
/// The path is the one `open_at_least` refused, so it has already been through
/// [`path_key`] and is UTF-8 by construction; deriving the key here rather than
/// taking it keeps one definition of what keys a session.
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

/// The byte offset the next pass resumes from: just past the last `\n`.
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
    /// Inside the transaction, after the archive rows and `session_meta`.
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
    /// file on its NAME alone - a `<uuid>.jsonl` at project depth, or an
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
    /// fails its walk once it has walked that many transcripts, which is how "a
    /// pass that died still leaves its `runs` row" becomes testable.
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
/// Built by the writer from the file's [`Row`] and handed to [`prepare`], which
/// is what lets prepare run on a thread with no connection of its own. A file
/// no longer than its watermark costs a stat and no query; one that grew adds
/// the next turn number, every hash and at most the tail row (#85).
pub(crate) struct Existing {
    watermark: u64,
    /// Whether `session_meta.agent_meta` already holds this sidecar's meta
    /// bytes, so a steady-state pass reopens nothing to rewrite them.
    agent_meta_stored: bool,
    /// Whether a previous pass found this transcript shorter than its watermark
    /// and flagged the session. Read so the flag can be *cleared* when the file
    /// is whole again, and read here so the steady state costs no extra query
    /// and no write at all.
    diverged: bool,
    /// Whether retention emptied this session's block rows on purpose. Read off
    /// the same `session_meta` row as the two flags above, so the steady state
    /// still costs one query and no write.
    evicted: bool,
    /// How many uncompressed bytes this session already holds - the base of the
    /// STORED stream's coordinate system.
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
    /// `session_meta.block_size` as stored. Checked where it is used, so a bad
    /// value fails only a pass that has something to append.
    block_size: Option<i64>,
    /// `session_meta.transcript_path` as stored: where the file was last read
    /// from. Off the same row as the flags, so knowing whether it moved costs
    /// no extra query.
    transcript_path: Option<String>,
    /// Whether that file is the one this pass reads.
    holder: Holder,
    session: Option<ExistingSession>,
    /// What an append needs, fetched only when there may be one. `None` is "not
    /// fetched", never "nothing archived", so [`prepare`] cannot append without
    /// the stored hashes.
    fetched: Option<Fetched>,
}

/// What [`Existing::read`] fetches for a file that is longer than its
/// watermark, for a session flagged diverged, and for one whose stored file is
/// gone. Never for an evicted session, which has no rows and is never appended.
struct Fetched {
    /// The `turn_seq` the next turn takes: a tail pass numbers on from here and
    /// a rebuild from 0 lands on the same numbers, because both count in byte
    /// order.
    next_turn: i64,
    /// The hashes and at most the tail row, or every row of a session flagged
    /// diverged or whose file is gone. Empty for a session never archived.
    archive: archive::AppendState,
}

/// The file an archived session was read from, against the file a pass is
/// about to read under the same key.
enum Holder {
    /// The same file, or no archived session to disagree with.
    This,
    /// The stored file no longer exists. The file read now may be it, moved.
    Gone(PathBuf),
    /// Another file, still there, or one that cannot be told apart from
    /// another.
    Other(PathBuf),
}

impl Holder {
    fn of(row: Option<&Row>, path: &Path) -> Holder {
        let Some(stored) = row
            .and_then(|r| r.transcript_path.as_deref())
            .filter(|s| *s != &*path.to_string_lossy())
        else {
            return Holder::This;
        };
        let held = PathBuf::from(stored);
        match (held.canonicalize(), path.canonicalize()) {
            (Ok(a), Ok(b)) if a == b => Holder::This,
            (Err(e), _) if e.kind() == std::io::ErrorKind::NotFound => Holder::Gone(held),
            _ => Holder::Other(held),
        }
    }
}

struct ExistingSession {
    session_no: i64,
    checksum: [u8; 32],
}

/// One archived session's `sessions`, `session_meta` and `watermarks` rows, as
/// [`read_rows`] found them.
#[derive(Default)]
pub(crate) struct Row {
    session_no: i64,
    /// `None` only when the session has no `session_meta` row, since the
    /// column is `NOT NULL`. Every flag below then reads as unset.
    checksum: Option<Vec<u8>>,
    agent_meta_stored: bool,
    diverged: bool,
    evicted: bool,
    uncompressed_len: i64,
    stored_capture_mode: Option<String>,
    block_size: Option<i64>,
    transcript_path: Option<String>,
    watermark: Option<i64>,
}

/// Every [`Row`] a pass reads, by session key.
pub(crate) type Rows = std::collections::HashMap<String, Row>;

/// The one definition of a session's row. It runs from `sessions`, so a key
/// with no row here is a file never archived, and its watermark is 0: recovery
/// removes every non-zero watermark with no `sessions` row before any file is
/// read, which is why this is read after `recover::recover` and never before.
///
/// LEFT JOIN on `session_meta`, and the difference is the archive. An inner
/// join returns no row in two very different situations - this transcript has
/// never been ingested, and this transcript IS archived but lost its
/// `session_meta` row - and the caller reads "no row" as "brand new". A pass
/// over a grown transcript then wrote the TAIL as the whole session, destroying
/// every archived byte before it, allocated a second `session_no` that no
/// longer matched the `sessions` row, and left the watermark ahead of the
/// committed stream: the one state a store must never reach, reached by a pass
/// that exits 0. `verify` then certified it, because the checksum had been
/// minted over the truncated stream - so the obvious response to `verify`
/// naming a session (re-run ingest) was what destroyed it.
const ROWS: &str = "SELECT s.session_key, s.session_no, m.checksum, m.agent_meta IS NOT NULL,
        coalesce(m.transcript_diverged, 0) <> 0, coalesce(m.is_evicted, 0) <> 0,
        coalesce(m.uncompressed_len, 0), m.capture_mode, m.block_size, m.transcript_path,
        w.byte_offset
 FROM sessions s
 LEFT JOIN session_meta m USING (session_key)
 LEFT JOIN watermarks w ON w.transcript_path = s.session_key";

/// Read every archived session's row in one query, or with `only` just that
/// key's, so ingesting one named file does not read the whole store.
///
/// Read once, at pass start, and never stale inside the pass: `claim` lets a
/// key through once per walk, so no file's apply changes a row a later file
/// reads.
pub(crate) fn read_rows(conn: &Connection, only: Option<&str>) -> Result<Rows> {
    let one;
    let sql = match only {
        None => ROWS,
        Some(_) => {
            one = format!("{ROWS} WHERE s.session_key = ?1");
            one.as_str()
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(only), |r| {
            Ok((
                r.get::<_, String>(0)?,
                Row {
                    session_no: r.get(1)?,
                    checksum: r.get(2)?,
                    agent_meta_stored: r.get(3)?,
                    diverged: r.get(4)?,
                    evicted: r.get(5)?,
                    uncompressed_len: r.get(6)?,
                    stored_capture_mode: r.get(7)?,
                    block_size: r.get(8)?,
                    transcript_path: r.get(9)?,
                    watermark: r.get(10)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<Rows>>()?;
    Ok(rows)
}

impl Existing {
    /// `path` is the file the pass will read under `session_key`, and `rows`
    /// is the pass's [`read_rows`] map, which holds no row for a file never
    /// archived.
    pub(crate) fn read(
        conn: &Connection,
        rows: &Rows,
        session_key: &str,
        path: &Path,
    ) -> Result<Existing> {
        let row = rows.get(session_key);

        // An archived session with no metadata row is damaged, not absent.
        // Ingest refuses rather than guessing: there is no checksum to verify
        // the stored hashes against, and an append requires one precisely so
        // that a growing session cannot have corruption written into it.
        let session =
            match row {
                None => None,
                Some(Row { checksum: None, .. }) => {
                    return Err(Error::NotAStore {
                        path: session_key.into(),
                        detail: "session is archived but has no session_meta row; \
                             run `terminus verify` - refusing to overwrite its archive"
                            .into(),
                    })
                }
                Some(Row {
                    session_no,
                    checksum: Some(checksum),
                    ..
                }) => {
                    let checksum = checksum.as_slice().try_into().map_err(|_| {
                        Error::BlobChecksumMismatch {
                            expected: "32 bytes".into(),
                            actual: format!("{} bytes", checksum.len()),
                        }
                    })?;
                    Some(ExistingSession {
                        session_no: *session_no,
                        checksum,
                    })
                }
            };

        // A file never archived reads every field as unset.
        let fallback = Row::default();
        let r = row.unwrap_or(&fallback);
        let watermark = r.watermark;
        let agent_meta_stored = r.agent_meta_stored;
        let diverged = r.diverged;
        let evicted = r.evicted;
        let uncompressed_len = r.uncompressed_len.max(0) as u64;
        let stored_capture_mode = r.stored_capture_mode.clone();
        let block_size = r.block_size;
        let transcript_path = r.transcript_path.clone();

        let holder = Holder::of(row, path);
        // A file standing in for a gone one is compared against every archived
        // byte, the way a flagged one is.
        let whole = diverged || matches!(holder, Holder::Gone(_));

        // What an append needs, fetched only for a file that may have grown,
        // and after every refusal above so an unchanged file still gets each
        // one. A stat and not an open: the pass's opens are counted, and the
        // tail read opens the file once anyway. A file no longer than its
        // watermark has nothing to append, and a shorter one still fails at
        // that open, so neither fetches a row. Nor does a failed stat: the open
        // fails the same way. No rows at all for an evicted session: `prepare`
        // never appends to one, and it has none to read. The stat is skipped
        // for an evicted or `whole` session, where its answer would not be
        // read.
        let watermark = watermark.unwrap_or(0) as u64;
        let fetched = if !evicted
            && (whole || std::fs::metadata(path).is_ok_and(|m| m.len() > watermark))
        {
            // `max` is a seek on `UNIQUE (session_key, turn_seq)`, where
            // `count(*)` walked the session's index range. The two agree,
            // because turns are numbered from 0 with no gap.
            let next_turn: i64 = conn.query_row(
                "SELECT coalesce(max(turn_seq) + 1, 0) FROM turns WHERE session_key = ?1",
                [session_key],
                |r| r.get(0),
            )?;
            let archive = match &session {
                Some(s) => {
                    archive::append_state(conn, s.session_no, block_size, uncompressed_len, whole)?
                }
                None => archive::AppendState::empty(),
            };
            Some(Fetched { next_turn, archive })
        } else {
            None
        };

        Ok(Existing {
            watermark,
            agent_meta_stored,
            diverged,
            evicted,
            uncompressed_len,
            stored_capture_mode,
            block_size,
            transcript_path,
            holder,
            session,
            fetched,
        })
    }

    /// Is this an archived session whose stored path is not `path`? Compared
    /// the way `write_session_meta` writes the path.
    fn moved(&self, path: &Path) -> bool {
        self.session.is_some() && self.transcript_path.as_deref() != Some(&*path.to_string_lossy())
    }
}

/// The session key for a transcript path: the path below its project folder,
/// so moving `.claude`, reaching it through a symlink or moving the project
/// folder keeps the key.
///
/// `<uuid>.jsonl` for a session, and
/// `<sessionId>/subagents/[workflows/wf_*/]agent-*.jsonl` for a sidecar. The
/// shape is read from the end of the path, which must sit in a project folder
/// inside a `projects` folder, never by stripping a configured root, so a
/// stored key from a root or project folder that no longer exists reduces the
/// same way. Any other path keys on its own text, which every caller hands
/// in canonical.
///
/// A path whose last component is a link keys on the file it points at, found
/// from link metadata alone. No byte of the file is read, and a path that does
/// not exist gets the shape test and is not an error.
pub fn path_key(path: &Path) -> Result<String> {
    let target = link_target(path);
    let path = target.as_deref().unwrap_or(path);
    let text = path.to_str().ok_or_else(|| {
        Error::io(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transcript path is not valid UTF-8, so it cannot key a session",
            ),
        )
    })?;
    Ok(below_project_folder(path).unwrap_or_else(|| text.to_owned()))
}

/// The file a link points at, when the path's last component is a link.
///
/// Only an absolute path is looked at, so a key that is already relative never
/// resolves against the working directory. A link that will not resolve is
/// keyed as itself.
pub(crate) fn link_target(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_symlink() {
        return None;
    }
    path.canonicalize().ok()
}

/// The part of a UTF-8 path below its project folder, when the path has one of
/// the two transcript shapes discovery yields and the project folder sits in a
/// `projects` folder. The project folder's name is left out of the key.
fn below_project_folder(path: &Path) -> Option<String> {
    use std::path::Component;
    let parts: Vec<&str> = path
        .components()
        .rev()
        .map_while(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    // `parts` runs from the file name upwards and stops at the first component
    // that is not a plain name, such as the root or a `..`.
    let file = *parts.first()?;
    // `depth` counts the components up to and including the project folder,
    // which must sit in a folder named `projects`. The key keeps all of them
    // but the project folder.
    let depth = if crate::discover::is_uuid_transcript(file) {
        2
    } else if crate::discover::is_agent_transcript(file) {
        match parts.get(1..4)? {
            [crate::lineage::SIDECAR_DIR, _, _] => 4,
            [wf, "workflows", crate::lineage::SIDECAR_DIR] if wf.starts_with("wf_") => 6,
            _ => return None,
        }
    } else {
        return None;
    };
    if *parts.get(depth)? != "projects" {
        return None;
    }
    let mut key: Vec<&str> = parts[..depth - 1].to_vec();
    key.reverse();
    Some(key.join("/"))
}

/// A flagged session's tail, read only once its prefix proves the file is the
/// one that was archived.
///
/// One open does all three reads, in this order: the length check
/// ([`Error::TranscriptDiverged`] for a file shorter than the watermark), the
/// prefix comparison ([`Error::TranscriptRewritten`] when it fails, flag left
/// set), and the tail from the watermark to EOF (#15). A file replaced at this
/// path between the prefix read and the tail read, by rename or by unlink and
/// recreate, cannot put its tail after the other file's prefix, because both
/// come from the same handle.
///
/// It does not protect against the same file being rewritten in place while it
/// is read, and adds no identity or length re-check for it.
fn read_flagged(path: &Path, existing: &Existing) -> Result<Vec<u8>> {
    let watermark = existing.watermark;
    let (mut file, len) = open_at_least(path, watermark)?;
    if !prefix_matches(&mut file, path, existing, watermark)? {
        // Still divergent, and now for a reason a length test cannot see.
        // Skipped exactly like the shortened case, flag left set.
        return Err(Error::TranscriptRewritten {
            path: path.to_path_buf(),
            watermark,
        });
    }
    rest_of(&mut file, path, watermark, len)
}

/// Are the file's first `watermark` bytes the ones already archived?
///
/// The question a length test cannot answer, asked only of a session that is
/// already flagged, and read from the handle [`read_flagged`] opened. `false`
/// means the file at this path is a different file that happens to be long
/// enough - truncated and rewritten, or replaced outright - and the archive
/// must not be appended to at the stale offset.
///
/// A session with nothing archived cannot disagree with the file, so it is
/// treated as matching: the flag was set for a session whose bytes are gone,
/// and refusing forever would be a permanent failure with nothing to compare.
///
/// A session NOT captured under `full` answers `false`. Its archived stream and
/// the file's first `watermark` bytes are not the same bytes by construction -
/// elision is what makes them differ - so the comparison cannot say anything,
/// and the safe answer is the one the surrounding code already chooses: still
/// divergent. The flag stays set, the file stays skipped, and `terminus verify`
/// keeps naming it. The alternative is appending at a stale offset onto a
/// session whose middle no longer exists anywhere, with `verify` reporting it
/// clean because its checksum still matches itself.
///
/// Those two answers, and an archive shorter than the watermark, read no byte
/// of the file.
///
/// The one reader on the ingest path that decodes every block, from the rows
/// [`Existing::read`] fetched for a flagged session.
fn prefix_matches(
    file: &mut std::fs::File,
    path: &Path,
    existing: &Existing,
    watermark: u64,
) -> Result<bool> {
    if existing.session.is_none() {
        return Ok(true);
    }
    // Null is `full`: a row written before the column existed was written by a
    // binary that had no other mode.
    if !matches!(existing.stored_capture_mode.as_deref(), None | Some("full")) {
        return Ok(false);
    }
    let rows = existing
        .fetched
        .as_ref()
        .and_then(|f| f.archive.every_row.as_deref())
        .unwrap_or_default();
    let archived = archive::whole(existing.block_size, existing.uncompressed_len, rows)?;
    if (archived.len() as u64) < watermark {
        // The archive is shorter than the watermark claims. That is a store
        // inconsistency rather than a transcript one, and `verify` is the
        // command that reports it - here it simply is not a match.
        return Ok(false);
    }

    file.seek(SeekFrom::Start(0))
        .map_err(|e| Error::io(path, e))?;
    let mut head = vec![0u8; watermark as usize];
    file.read_exact(&mut head).map_err(|e| Error::io(path, e))?;
    Ok(head == archived[..watermark as usize])
}

/// Read from the stored watermark to the end of the file, for a session that
/// is not flagged. It seeks past the prefix and never reads it.
fn read_tail(path: &Path, watermark: u64) -> Result<Vec<u8>> {
    let (mut file, len) = open_at_least(path, watermark)?;
    rest_of(&mut file, path, watermark, len)
}

/// Open the transcript and refuse one shorter than the watermark.
///
/// Through `discover::open_transcript` and not `File::open`, so the pass's
/// opens are counted: "zero opens of any file under an excluded directory" has
/// to be a fact a test can read off a log.
fn open_at_least(path: &Path, watermark: u64) -> Result<(std::fs::File, u64)> {
    let file = crate::discover::open_transcript(path).map_err(|e| Error::io(path, e))?;
    let len = file.metadata().map_err(|e| Error::io(path, e))?.len();
    if len < watermark {
        // A transcript never shrinks: compaction appends to the same file. A
        // shorter file is a different file at the same path, and guessing which
        // bytes are still ours would corrupt the archive - so the refusal
        // stands and there is deliberately no fallback to offset 0, which would
        // overwrite archived bytes on the strength of an append-only property
        // observed over one 25-day corpus rather than documented anywhere.
        //
        // What this adds is not a recovery but a signal: this variant is the
        // one the pass matches on to flag the session, so `terminus verify` can
        // say the transcript and the archive have diverged.
        return Err(Error::TranscriptDiverged {
            path: path.to_path_buf(),
            len,
            watermark,
        });
    }
    Ok((file, len))
}

/// The bytes from the watermark to EOF, from a handle [`open_at_least`] gave.
fn rest_of(file: &mut std::fs::File, path: &Path, watermark: u64, len: u64) -> Result<Vec<u8>> {
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

/// The lineage signal, in the session-id namespace ([`lineage`]).
///
/// The record's foreign `session_id` field is the reliable one: 169 of 1,253
/// real top-level transcripts name another session that way, an order of
/// magnitude above the 1.2% `parentUuid` resume rate. `parentUuid` is the
/// fallback and only that - it is message-level threading, so it is resolved
/// through `turns` to the session that holds the parent record, keeping this
/// column in one namespace.
///
/// Both paths reject a link to this transcript's own session id.
///
/// A `parentUuid` predecessor that is not archived yet leaves the column NULL
/// here, and the tree pass's sweep fills it once the predecessor is archived
/// (#11).
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

    parent_session_id(
        conn,
        session_key,
        parent_uuid,
        lineage::own_session_id(scan),
    )
}

/// The session id of the other file holding the message `parent_uuid`, or
/// `None`: the `parentUuid` half of [`continues_from`], written once so every
/// caller resolves a predecessor by the same rules.
///
/// `own` is the session id the transcript reports for itself.
fn parent_session_id(
    conn: &Connection,
    session_key: &str,
    parent_uuid: &str,
    own: Option<&str>,
) -> Result<Option<String>> {
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
    Ok(resolved.filter(|id| !lineage::is_self_link(id, own)))
}

/// Fill the continuation link of every session that has none yet, for the
/// `parentUuid` predecessors that were archived after it (#11).
///
/// Ingest resolves the link once, when a session's first turn is written, so a
/// session archived before its predecessor keeps a NULL link for good unless
/// something looks again. This looks again: every session with a NULL
/// `continues_from` whose first turn names a `parentUuid` goes through
/// [`parent_session_id`], the rule ingest uses. A foreign `session_id` is
/// recorded at ingest, so such a session is never selected, and the UPDATE
/// fills a NULL only, so no link is ever overwritten.
///
/// The turns are reached through `UNIQUE (session_key, turn_seq)` for the first
/// turn and `idx_turns_uuid` for the parent, never by scanning `turns`: this
/// runs on every tree pass, and so on every hook spawn.
pub(crate) fn fill_continuations(conn: &Connection) -> Result<()> {
    let pending: Vec<(String, Option<String>, String)> = conn
        .prepare(
            "SELECT m.session_key, m.session_id, t.parent_uuid
             FROM session_meta m CROSS JOIN turns t
             WHERE m.continues_from IS NULL
               AND t.session_key = m.session_key AND t.turn_seq = 0
               AND t.parent_uuid IS NOT NULL",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;

    for (session_key, own, parent_uuid) in pending {
        let resolved = parent_session_id(conn, &session_key, &parent_uuid, own.as_deref())?;
        let Some(predecessor) = resolved else {
            continue;
        };
        conn.execute(
            "UPDATE session_meta SET continues_from = ?1
             WHERE session_key = ?2 AND continues_from IS NULL",
            rusqlite::params![predecessor, session_key],
        )?;
    }
    Ok(())
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
    /// Written on the session's first pass and kept after.
    block_size: u32,
    /// In the session-id namespace, never the session-key one - see
    /// [`continues_from`].
    continues_from: Option<&'a str>,
    /// Resolved from the first `cwd` in this scan, or `None` for a transcript
    /// that carries no `cwd` at all. One real transcript does.
    project: Option<&'a Project>,
    /// The `session_key` of the transcript this sidecar ran under, or `None`
    /// for a top-level transcript. In the session-KEY namespace, unlike
    /// [`MetaRow::continues_from`].
    parent_session_key: Option<&'a str>,
    /// The bytes of the sidecar's `agent-*.meta.json`, stored opaquely.
    agent_meta: Option<&'a [u8]>,
    /// Which `[capture]` mode produced the bytes this pass is writing. Not
    /// `coalesce`d like the columns above - see [`write_session_meta`].
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
/// the project, the sidecar's parent and the first turn's timestamp were
/// established by the first pass.
///
/// `branch` runs the other way, like `last_turn_at`: it is the `gitBranch` of the
/// last record that carried one, so a tail that names a branch replaces what is
/// stored and a tail that names none leaves it. The brief reports it as the
/// branch the session ended on.
///
/// `is_final` is not written here and is not written by any ingest of a file:
/// it is the idle rule's, evaluated once at the end of a whole pass over
/// `last_turn_at` ([`crate::feedback::finalize`]), which is what lets one
/// mechanism cover a clean exit and a crash alike. `is_evicted` is not written
/// here either, and must not be: it is retention's, and an ingest that cleared
/// it would un-evict a session on the pass after the one that emptied it.
/// `prepare` never reaches this function for an evicted session at all.
///
/// `capture_mode` is the one column that is neither `coalesce`d onto what is
/// there nor overwritten outright. It reads `full` only while EVERY append to
/// the session has been full, so:
///
/// - a stored null or `full` is replaced by whatever this pass captured under;
/// - a stored `lean` or `minimal` is never moved back to `full` by a later full
///   pass, because an append never rewrites a completed block and the bytes
///   elided under the earlier mode are never revisited;
/// - a session appended to under two different REDUCED modes reads as the later
///   one. The guarantee the column carries is exactly "`full` means every append
///   was full", and a mixed reduced session has no single honest answer beyond
///   "not full".
fn write_session_meta(tx: &Connection, row: MetaRow<'_>) -> Result<()> {
    let scan = row.scan;
    let session_id = scan.records.iter().find_map(|r| r.session_id.clone());
    let cwd = first_cwd(scan);
    let branch = scan.records.iter().rev().find_map(|r| r.git_branch.clone());
    // Byte order, not clock order: the "first" turn is the first in the file
    // even when its timestamp is later than its neighbour's.
    let first_turn_at = scan.turns().next().map(|(_, t)| t.timestamp.clone());
    let last_turn_at = scan.turns().last().map(|(_, t)| t.timestamp.clone());
    let project = row.project.map(|p| p.key.as_str());
    // Only when folding actually happened, so the column answers "was this a
    // worktree" as well as "which one".
    let pre_worktree = row.project.and_then(|p| p.pre_worktree.as_deref());

    tx.execute(
        "INSERT INTO session_meta (
            session_key, session_id, transcript_path, checksum, uncompressed_len,
            continues_from, first_turn_at, last_turn_at, cwd, branch,
            project, project_pre_worktree, parent_session_key, agent_meta,
            capture_mode, block_size
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(session_key) DO UPDATE SET
            session_id = coalesce(session_meta.session_id, excluded.session_id),
            transcript_path = excluded.transcript_path,
            checksum = excluded.checksum,
            uncompressed_len = excluded.uncompressed_len,
            block_size = coalesce(session_meta.block_size, excluded.block_size),
            continues_from = coalesce(session_meta.continues_from, excluded.continues_from),
            first_turn_at = coalesce(session_meta.first_turn_at, excluded.first_turn_at),
            last_turn_at = coalesce(excluded.last_turn_at, session_meta.last_turn_at),
            cwd = coalesce(session_meta.cwd, excluded.cwd),
            branch = coalesce(excluded.branch, session_meta.branch),
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
            row.block_size,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/session-basic.jsonl"
    ));

    fn watermark(conn: &Connection, key: &str) -> i64 {
        conn.query_row(
            "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
            [key],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn block_rows(conn: &Connection) -> Vec<(i64, i64, Vec<u8>, Vec<u8>)> {
        conn.prepare("SELECT session_no, block_no, bytes, hash FROM blocks ORDER BY 1, 2")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    /// A file that grows between `Existing::read`'s length check and the tail
    /// read is reported up to date with nothing written, and the next read
    /// fetches what the append needs and commits it.
    #[test]
    fn a_file_that_grows_after_its_check_is_left_for_the_next_pass() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let path = dir
            .path()
            .join("77777777-7777-4777-8777-777777777777.jsonl");
        std::fs::write(&path, FIXTURE).unwrap();
        let path = path.canonicalize().unwrap();
        let config = crate::config::Config::from_parts(Vec::new(), Vec::new());
        assert!(matches!(
            run_with(&data_dir, &path, &config).unwrap(),
            Outcome::Committed(_)
        ));

        let mut store = Store::open(&data_dir).unwrap();
        let key = path_key(&path).unwrap();
        let rows = read_rows(store.conn(), Some(&key)).unwrap();
        let existing = Existing::read(store.conn(), &rows, &key, &path).unwrap();
        assert!(existing.fetched.is_none(), "an unchanged file fetched rows");
        let mark = watermark(store.conn(), &key);
        let rows = block_rows(store.conn());

        let body = &FIXTURE[..FIXTURE.len() - 1];
        let start = body.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let record = &FIXTURE[start..];
        let mut grown = FIXTURE.to_vec();
        grown.extend_from_slice(record);
        std::fs::write(&path, &grown).unwrap();

        let mut projects = Resolver::new();
        let prepared = prepare(&path, key.clone(), &existing, CaptureMode::Full).unwrap();
        let outcome = apply(&mut store, prepared, &mut projects).unwrap();
        assert_eq!(outcome, Ingested::UpToDate);
        assert_eq!(watermark(store.conn(), &key), mark);
        assert_eq!(block_rows(store.conn()), rows);

        let rows = read_rows(store.conn(), Some(&key)).unwrap();
        let existing = Existing::read(store.conn(), &rows, &key, &path).unwrap();
        let prepared = prepare(&path, key.clone(), &existing, CaptureMode::Full).unwrap();
        let outcome = apply(&mut store, prepared, &mut projects).unwrap();
        let Ingested::Committed(pass) = outcome else {
            panic!("the next read did not commit: {outcome:?}");
        };
        assert_eq!(pass.bytes_read, record.len() as u64);
        assert_eq!(watermark(store.conn(), &key), grown.len() as i64);
    }
}
