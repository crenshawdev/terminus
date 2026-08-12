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
/// `compaction_boundaries`, `turns_fts`, `entities`, `paths`. Two integers
/// rather than one, because one cannot tell "derived tables need a rebuild"
/// from "the archive itself changed" (D-09).
///
/// 2 as of phase 2: `compaction_boundaries` joined the derived set (D-21), so
/// the first ingest run against a store written by a phase 1 binary rebuilds
/// every archived session from its blob, inside the ingest lock, before it
/// walks anything.
pub const DERIVED_SCHEMA: i64 = 2;

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

        let state = inspect(&path)?;

        let rebuild = match state {
            StoreState::Initialized => gate(&path)?,
            StoreState::Fresh => None,
        };

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
        if state == StoreState::Fresh {
            store.initialize()?;
        } else {
            // Every open of an existing store, and deliberately NOT gated on
            // `rebuild_required`. `reindex::open_up_to_date` is the only caller
            // that acts on that outcome, while `status`, `verify` and every
            // phase 3 reader open through here alone - so gating the column
            // bring-forward on it would leave a user who upgraded the binary
            // and ran `status` before any ingest hitting `no such column` on a
            // perfectly healthy store. Adding a missing column is safe on any
            // opener; the destructive derived-table rebuild stays exactly where
            // it is, outside this function and inside the ingest lock.
            bring_forward(&store.conn)?;
        }
        Ok(store)
    }

    /// Write the tables and the version integers a fresh store carries.
    ///
    /// One transaction, and that is load-bearing rather than tidy. The tables
    /// and the version integers are two statements, and a store that has the
    /// first without the second is one the gate reads as `NotAStore` forever.
    /// SQLite makes DDL transactional, so an interrupted initialize rolls all
    /// the way back to a database with no tables at all - which is exactly the
    /// state [`inspect`] recognizes as [`StoreState::Fresh`] and retries. The
    /// two halves are one fix: without the transaction the rollback could stop
    /// half way and leave tables behind, and without `inspect` the rolled-back
    /// file would still be non-empty and read as a store that lost its `meta`.
    fn initialize(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(crate::store::schema::CREATE_SQL)?;
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2), (?3, ?4)",
            rusqlite::params![
                META_ARCHIVE_FORMAT,
                ARCHIVE_FORMAT.to_string(),
                META_DERIVED_SCHEMA,
                DERIVED_SCHEMA.to_string(),
            ],
        )?;
        tx.commit()?;
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

/// Add whatever this build's schema has and the open store does not.
///
/// Additive only, and idempotent by construction: it creates a table this build
/// declares and the store lacks, and adds a column this build declares and the
/// table lacks. It drops nothing, renames nothing and rewrites nothing, which
/// is what makes it safe to run on a read command's open. `sessions` and
/// `session_meta` are archive tables and an added column is the only change
/// either is ever allowed (`DESIGN-BRIEF.md:94`).
///
/// Nothing is written when nothing is missing - the common case is three read
/// queries and no transaction at all - so a plain reopen of an up-to-date store
/// still touches no byte of it.
fn bring_forward(conn: &Connection) -> Result<()> {
    let present_tables = table_names(conn)?;
    let missing_tables: Vec<&&str> = crate::store::schema::TABLES
        .iter()
        .filter(|t| !present_tables.iter().any(|p| p == *t))
        .collect();

    let mut missing_columns: Vec<(&str, &str, &str)> = Vec::new();
    for (table, columns) in crate::store::schema::BRING_FORWARD_COLUMNS {
        // A table this build is about to create arrives with every column
        // already on it, so asking `PRAGMA table_info` about it would only
        // schedule columns that are not missing.
        if missing_tables.iter().any(|t| **t == *table) {
            continue;
        }
        let present = column_names(conn, table)?;
        for (name, declaration) in *columns {
            if !present.iter().any(|c| c == name) {
                missing_columns.push((table, name, declaration));
            }
        }
    }

    if missing_tables.is_empty() && missing_columns.is_empty() {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    if !missing_tables.is_empty() {
        // Every statement in it is `IF NOT EXISTS`, so this creates exactly the
        // tables and indexes that are absent and leaves the rest alone. One
        // definition of the schema rather than a second copy that can drift.
        tx.execute_batch(crate::store::schema::CREATE_SQL)?;
    }
    for (table, name, declaration) in missing_columns {
        // Identifiers come from a `const` in this crate, never from a caller.
        tx.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {name} {declaration}"
        ))?;
    }
    tx.commit()?;
    Ok(())
}

/// Every table in the open database, virtual tables included.
fn table_names(conn: &Connection) -> Result<Vec<String>> {
    let mut statement = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
    let names = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(names)
}

/// The columns of one table, in declaration order.
fn column_names(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(names)
}

/// What sits at the store path, as far as deciding whether to initialize goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreState {
    /// Nothing is there, or a database with no tables in it at all. Initialize.
    Fresh,
    /// A store with its tables. Gate it, never write over it.
    Initialized,
}

/// Decide whether the path holds a store, without writing a byte.
///
/// File length is not the test, and that is the whole point of this function.
/// `open` sets `journal_mode=wal` on the writable connection, which stamps the
/// database header and grows a brand-new file to 4096 bytes *before*
/// `initialize` runs. So a crash - or a failed commit - anywhere inside
/// `initialize` leaves a non-empty file holding no tables, and a length test
/// calls that an existing store, sends it to [`gate`], and returns `NotAStore`
/// from then on. The data directory is bricked with no repair path, for a
/// database that contains nothing.
///
/// Presence of tables is the test instead. It reads `sqlite_master` over a
/// **read-only** connection, so AC5's "a refused open leaves the store
/// byte-for-byte unchanged" survives it.
///
/// A database holding tables but no `meta` is somebody else's SQLite file, not
/// a half-built store: it is refused rather than initialized over. Initializing
/// is only ever reached with nothing to lose.
fn inspect(path: &Path) -> Result<StoreState> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() == 0 => return Ok(StoreState::Fresh),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(StoreState::Fresh),
        Err(e) => return Err(Error::io(path, e)),
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|source| Error::NotAStore {
        path: path.to_path_buf(),
        detail: format!("cannot open it for reading: {source}"),
    })?;
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))?;

    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |r| r.get(0),
        )
        .map_err(|source| Error::NotAStore {
            path: path.to_path_buf(),
            // The usual cause is a file that is not a database at all: SQLite
            // opens lazily, so the first statement is where that surfaces.
            detail: format!("reading its table list: {source}"),
        })?;

    Ok(if tables == 0 {
        StoreState::Fresh
    } else {
        StoreState::Initialized
    })
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
