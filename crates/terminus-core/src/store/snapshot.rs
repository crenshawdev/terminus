//! Rolling snapshots of the store: a consistent copy taken without stopping an
//! ingest, and a prune that keeps only the newest few (STOR-06).
//!
//! **`VACUUM INTO`, never `rusqlite::backup` (D-07).** That module is gated
//! behind a `backup` cargo feature this workspace does not enable
//! (rusqlite 0.40's own `Cargo.toml` declares it and `src/lib.rs` gates
//! `pub mod backup;` on it), so a snapshot reaching for it does not compile -
//! and enabling the feature is an unbudgeted addition to a binary whose cold
//! start is the product. `VACUUM INTO` needs nothing that is not already
//! linked, and it reads ONE consistent view of the database through WAL:
//! measured 2026-08-22 against the live 1.10 GB store it succeeded while a
//! second connection held an open `BEGIN IMMEDIATE`, excluding that
//! transaction's uncommitted row, where a plain `VACUUM` failed with
//! `database is locked`.
//!
//! **Neither function takes the ingest lock,** and that is the whole point: a
//! copy that stopped the pass would be a copy nobody could afford to take on
//! the schedule STOR-06 asks for. `VACUUM INTO` is read-only with respect to
//! the database it copies.
//!
//! **The name carries the instant, so ordering by name is ordering by time.**
//! The prune therefore needs no filesystem mtime - which a copy, a restore or a
//! `tar -x` would each have rewritten - and `VACUUM INTO` refusing a non-empty
//! destination becomes a property rather than a hazard. The instant is
//! read from SQLite, the same clock every stored timestamp in this store comes
//! from, in ISO-8601 **basic** format: `:` is not a legal filename character on
//! Windows, which is a first-class target, so the extended form every column
//! holds cannot be a file name here. Basic format is fixed-width and still
//! orders lexicographically, which is the property the prune actually uses.
//!
//! **Written to a temporary name and renamed into place.** A process killed
//! mid-copy leaves a half-written file, and a half-written file that matched
//! the snapshot name would be counted by the prune as one of the N kept - so
//! the two most recent good snapshots would be dropped for it.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::store::open::DB_FILE_NAME;

/// The subdirectory of the data directory that holds the snapshots.
///
/// Inside the data directory rather than beside it, so `terminus data move`
/// carries the snapshots with the store it copies and `uninstall --purge`
/// removes everything it says it is removing.
pub const DIR_NAME: &str = "snapshots";

/// What a snapshot file's name starts with.
const PREFIX: &str = "terminus-";
/// What a snapshot file's name ends with.
const SUFFIX: &str = ".db";

/// Take one snapshot of the store in `data_dir`, and return where it landed.
///
/// Takes no lock and holds no transaction of its own. An ingest pass may be
/// mid-walk: what the snapshot holds is the database as it stood at the instant
/// `VACUUM INTO` began, which excludes whatever transaction was open then and
/// includes everything committed before it.
pub fn take(data_dir: &Path) -> Result<PathBuf> {
    let db = data_dir.join(DB_FILE_NAME);
    if !db.is_file() {
        return Err(Error::StoreNotFound { path: db });
    }

    let dir = data_dir.join(DIR_NAME);
    // Owner-only at creation, like every directory terminus makes: a snapshot is
    // a copy of the whole archive and must not be readable by group or world for
    // even an instant (PRIV-02, PRIV-04).
    crate::owner_only::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;

    let conn = Connection::open(&db).map_err(Error::Sqlite)?;

    // The store's own clock, so a snapshot's name and the timestamps inside it
    // are the same reading of the same source. `%f` is `SS.SSS`, so this is
    // `YYYYMMDDTHHMMSS.sssZ` - twenty fixed-width characters, no colon.
    let stamp: String =
        conn.query_row("SELECT strftime('%Y%m%dT%H%M%fZ', 'now')", [], |r| r.get(0))?;

    let path = dir.join(format!("{PREFIX}{stamp}{SUFFIX}"));
    // A leading dot and a `.tmp` tail, so it matches neither the snapshot name
    // nor anything the prune counts.
    let temp = dir.join(format!(".{PREFIX}{stamp}{SUFFIX}.tmp"));
    // `VACUUM INTO` refuses a destination that already holds bytes - measured
    // 2026-08-30, it fails with `file is not a database` - and a previous kill
    // can have left this one behind holding a partial copy. The real name is
    // never removed here.
    if temp.exists() {
        std::fs::remove_file(&temp).map_err(|e| Error::io(&temp, e))?;
    }

    // The destination is created here, EMPTY and 0600, and `VACUUM INTO` writes
    // into it rather than making it itself (D-03). A file SQLite creates lands
    // at the umask - 0644 on a stock machine - and the only way to narrow it
    // afterwards would be a chmod over a file that already holds the whole
    // archive, which is the window AC2 bars. Measured 2026-08-30: `VACUUM INTO`
    // onto a pre-created zero-length file succeeds and keeps the mode it finds,
    // and the rename below carries that mode to the real name. The handle is
    // closed immediately: what is wanted from it is the inode and its mode, not
    // a writer.
    let reserved = crate::owner_only::options()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| Error::io(&temp, e))?;
    drop(reserved);

    // Bound, not interpolated: the destination is a path, and a path that
    // cannot be a UTF-8 SQL string literal must fail rather than be mangled
    // into one.
    let destination = temp.to_str().ok_or_else(|| {
        Error::io(
            &temp,
            std::io::Error::other("the snapshot path is not valid UTF-8"),
        )
    })?;
    conn.execute("VACUUM INTO ?1", [destination])?;

    std::fs::rename(&temp, &path).map_err(|e| Error::io(&path, e))?;
    Ok(path)
}

/// Keep the `keep` newest snapshots in `dir` and remove the rest. Returns how
/// many were removed.
///
/// Only files this module wrote are considered, and only they are ever removed:
/// the directory is inside the user's data directory and a prune that deleted
/// "everything else in there" would be a destructive step nobody asked for.
/// A directory that is not there yet is nothing to prune rather than an error.
pub fn prune(dir: &Path, keep: usize) -> Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(Error::io(dir, e)),
    };

    let mut snapshots: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(PREFIX) && name.ends_with(SUFFIX) && entry.path().is_file() {
            snapshots.push(entry.path());
        }
    }
    // By name, which is by time: the instant is fixed-width and the prefix is
    // constant, so the two orders are the same one.
    snapshots.sort();

    let mut removed = 0;
    let over = snapshots.len().saturating_sub(keep);
    for path in snapshots.into_iter().take(over) {
        std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
        removed += 1;
    }
    Ok(removed)
}
