//! Exclusivity for an ingest pass: one `LOCK` file, one immediate-fail OS lock.
//!
//! D-15 fixes both halves. It is a **dedicated file** in the data directory,
//! created by the process itself, and never `BEGIN EXCLUSIVE`: a second process
//! blocking on SQLite's busy handler would blow the 50 ms budget ING-02 states,
//! and a write transaction held open for a whole pass stops MCP readers, which
//! is the reason redb was rejected (`DESIGN-BRIEF.md:83`). And the lock is an
//! **OS** lock rather than a PID file, so it is released by process death as
//! well as by drop - no stale-lock wedging, no orphan to reap
//! (`DESIGN-BRIEF.md:135`).
//!
//! [`std::fs::File::try_lock`] is the whole implementation and pulls in no
//! dependency: it is `flock(2)` with `LOCK_EX | LOCK_NB` on Unix and
//! `LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY` on
//! Windows, which is D-15 named exactly. D-20 records why that holds off Linux:
//! APFS honors `flock` with `LOCK_NB`, the Windows path is filesystem-agnostic
//! byte-range locking so NTFS versus ReFS does not matter, and the overlayfs
//! hazard (pre-copy-up shared inodes) cannot arise for a file this process
//! creates in its own data directory.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The lock file inside the data directory.
pub const LOCK_FILE_NAME: &str = "LOCK";

/// A held ingest lock. Released on drop, and by process death.
#[derive(Debug)]
pub struct IngestLock {
    file: File,
    path: PathBuf,
}

impl IngestLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IngestLock {
    fn drop(&mut self) {
        // Closing the file releases the lock on both platforms, so this is
        // explicitness rather than necessity, and an error here has nothing to
        // report to: the process is already on its way out of the pass.
        let _ = self.file.unlock();
    }
}

/// What a lock attempt found.
#[derive(Debug)]
pub enum Attempt {
    /// This process owns the pass.
    Acquired(IngestLock),
    /// Another process is mid-pass. Exit 0 immediately, having written nothing:
    /// the hook spawn is the scheduler, and a second invocation is normal
    /// rather than an error (ING-02).
    Held,
}

/// Try to take the ingest lock, failing immediately if another process holds it.
///
/// Called **before** the store is opened, so a contended run costs one file
/// open and one lock syscall and touches no database at all.
pub fn try_acquire(data_dir: &Path) -> Result<Attempt> {
    // This is the FIRST thing to create the data directory on a machine that
    // has never ingested - `ingest` and `data move` both take the lock before
    // they open the store - so it is where the directory gets its owner-only
    // mode (PRIV-02, PRIV-04). The leaf alone: `~/.local/share` above it keeps
    // whatever mode the user's own umask gives it (D-04).
    crate::owner_only::create_dir_all(data_dir).map_err(|e| Error::io(data_dir, e))?;
    let path = data_dir.join(LOCK_FILE_NAME);

    // Created if missing and never truncated: the file's *contents* carry no
    // meaning, only the lock the OS attaches to it. Truncating would rewrite a
    // file another process is holding. Owner-only from the moment it exists,
    // like everything else in this directory - the mode rides the creating
    // syscall, so there is no instant at which it is wider.
    let file = crate::owner_only::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;

    match file.try_lock() {
        Ok(()) => Ok(Attempt::Acquired(IngestLock { file, path })),
        Err(TryLockError::WouldBlock) => Ok(Attempt::Held),
        // A real failure - an unsupported filesystem, a bad descriptor - is not
        // contention and must not be reported as "someone else is working".
        Err(TryLockError::Error(source)) => Err(Error::io(&path, source)),
    }
}
