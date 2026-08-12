//! Resolving the data directory and opening the store behind the D-09 version
//! gate.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::error::{Error, Result};

/// The archive format this build writes and understands.
///
/// Bumped only when the bytes in `sessions` change meaning. The archive table
/// never migrates (`DESIGN-BRIEF.md:94`), so a bump rebuilds derived tables.
pub const ARCHIVE_FORMAT: i64 = 1;

/// The derived-table schema this build writes.
///
/// Bumped whenever anything rebuildable from the blobs changes shape: `turns`,
/// `turns_fts`, `entities`, `paths`. Two integers rather than one, because one
/// cannot tell "derived tables need a rebuild" from "the archive itself
/// changed" (D-09).
pub const DERIVED_SCHEMA: i64 = 1;

/// The store file inside the data directory.
pub const DB_FILE_NAME: &str = "verbatim.db";

/// `meta` key holding [`ARCHIVE_FORMAT`].
pub const META_ARCHIVE_FORMAT: &str = "archive_format";
/// `meta` key holding [`DERIVED_SCHEMA`].
pub const META_DERIVED_SCHEMA: &str = "derived_schema";

/// How long a writer waits behind another writer before giving up. Exclusivity
/// is the `LOCK` file's job (D-15), not the busy handler's; this is only there
/// so a short overlap with a reader is not an error.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// The store is older than this build and its derived tables need rebuilding.
///
/// Returned as an inspectable outcome rather than acted on here: the rebuild
/// lives in `reindex`, which does not exist yet (PLAN-2 task 6 wires this to
/// it). The archive is never part of the rebuild (STOR-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildRequired {
    pub store_archive_format: i64,
    pub store_derived_schema: i64,
    pub binary_archive_format: i64,
    pub binary_derived_schema: i64,
}

/// An open verbatim store.
pub struct Store {
    conn: Connection,
    data_dir: PathBuf,
    path: PathBuf,
    rebuild: Option<RebuildRequired>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("rebuild_required", &self.rebuild)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Open (or create) the store inside `data_dir`.
    ///
    /// The gate runs before any write. On an existing store the version
    /// integers are read over a **read-only** connection that is closed again
    /// before a writable one is opened: AC5 requires a refused open to leave
    /// `verbatim.db`, its WAL and the `runs` table byte-for-byte unchanged, and
    /// read-only is what makes that true at the SQLite level rather than by
    /// discipline. A writable connection would checkpoint the WAL when the last
    /// one closes, which rewrites both files without a single statement of ours.
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).map_err(|e| Error::io(data_dir, e))?;
        let path = data_dir.join(DB_FILE_NAME);

        let existing = match std::fs::metadata(&path) {
            Ok(m) => m.len() > 0,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(Error::io(&path, e)),
        };

        let rebuild = if existing { Some(gate(&path)?) } else { None }.flatten();

        let conn = Connection::open(&path).map_err(Error::Sqlite)?;
        conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;

        let store = Store {
            conn,
            data_dir: data_dir.to_path_buf(),
            path,
            rebuild,
        };
        if !existing {
            store.initialize()?;
        }
        Ok(store)
    }

    /// Write the tables and the version integers a fresh store carries.
    fn initialize(&self) -> Result<()> {
        let conn = &self.conn;
        conn.execute_batch(crate::store::schema::CREATE_SQL)?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2), (?3, ?4)",
            rusqlite::params![
                META_ARCHIVE_FORMAT,
                ARCHIVE_FORMAT.to_string(),
                META_DERIVED_SCHEMA,
                DERIVED_SCHEMA.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// `Some` when the store predates this build and its derived tables need
    /// rebuilding. Nothing has been rebuilt: this is the caller's cue.
    pub fn rebuild_required(&self) -> Option<RebuildRequired> {
        self.rebuild
    }

    /// Read an integer out of `meta`.
    pub fn meta_int(&self, key: &str) -> Result<Option<i64>> {
        let text: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        Ok(text.and_then(|t| t.parse().ok()))
    }

    /// Write an integer into `meta`.
    pub fn set_meta_int(&self, key: &str, value: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value.to_string()],
        )?;
        Ok(())
    }
}

/// Read the two version integers off an existing store and decide.
///
/// Runs no DDL and no DML. Returns `Ok(None)` when the store matches this
/// build, `Ok(Some(..))` when it is older, and an error when it is newer.
fn gate(path: &Path) -> Result<Option<RebuildRequired>> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(path, flags).map_err(|source| Error::NotAStore {
        path: path.to_path_buf(),
        detail: format!("cannot open it for reading: {source}"),
    })?;
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))?;

    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let read = |key: &str| -> Result<Option<i64>> {
        let text: Option<String> = tx
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(|source| Error::NotAStore {
                path: path.to_path_buf(),
                detail: format!("reading meta.{key}: {source}"),
            })?;
        Ok(text.and_then(|t| t.parse().ok()))
    };

    let store_archive = read(META_ARCHIVE_FORMAT)?.ok_or_else(|| Error::NotAStore {
        path: path.to_path_buf(),
        detail: format!("no meta.{META_ARCHIVE_FORMAT}"),
    })?;
    let store_derived = read(META_DERIVED_SCHEMA)?.ok_or_else(|| Error::NotAStore {
        path: path.to_path_buf(),
        detail: format!("no meta.{META_DERIVED_SCHEMA}"),
    })?;

    if store_archive > ARCHIVE_FORMAT {
        return Err(Error::ArchiveFormatTooNew {
            path: path.to_path_buf(),
            store_format: store_archive,
            binary_format: ARCHIVE_FORMAT,
        });
    }

    let outcome = if store_archive < ARCHIVE_FORMAT || store_derived != DERIVED_SCHEMA {
        // A *newer* derived_schema lands here too, and deliberately: derived
        // tables are rebuildable from the blobs by definition, so rebuilding
        // them down to this build's shape is always available. Only the archive
        // can be too new to read.
        Some(RebuildRequired {
            store_archive_format: store_archive,
            store_derived_schema: store_derived,
            binary_archive_format: ARCHIVE_FORMAT,
            binary_derived_schema: DERIVED_SCHEMA,
        })
    } else {
        None
    };

    // Explicit: the read transaction and the read-only connection are both gone
    // before any writable connection is opened.
    drop(tx);
    drop(conn);
    Ok(outcome)
}

/// The data directory, resolved once at startup and passed down explicitly
/// afterwards, never re-derived (`DESIGN-BRIEF.md:404`).
pub fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_var("VERBATIM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    platform_data_dir()
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Result<PathBuf> {
    if let Some(xdg) = non_empty_var("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg).join("verbatim"));
    }
    let home = non_empty_var("HOME").ok_or_else(|| Error::DataDirUnresolved {
        detail: "neither VERBATIM_DATA_DIR, XDG_DATA_HOME nor HOME is set".into(),
    })?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("verbatim"))
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Result<PathBuf> {
    let home = non_empty_var("HOME").ok_or_else(|| Error::DataDirUnresolved {
        detail: "neither VERBATIM_DATA_DIR nor HOME is set".into(),
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("verbatim"))
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Result<PathBuf> {
    let local = non_empty_var("LOCALAPPDATA").ok_or_else(|| Error::DataDirUnresolved {
        detail: "neither VERBATIM_DATA_DIR nor LOCALAPPDATA is set".into(),
    })?;
    Ok(PathBuf::from(local).join("verbatim"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_data_dir() -> Result<PathBuf> {
    // Every other Unix follows the XDG layout; only the three above are
    // first-class targets (PROJECT.md constraints).
    if let Some(xdg) = non_empty_var("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg).join("verbatim"));
    }
    let home = non_empty_var("HOME").ok_or_else(|| Error::DataDirUnresolved {
        detail: "neither VERBATIM_DATA_DIR, XDG_DATA_HOME nor HOME is set".into(),
    })?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("verbatim"))
}

/// An environment variable set to the empty string is treated as unset: an
/// empty path would resolve to the process's current directory.
fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}
