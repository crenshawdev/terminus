//! Resolving the data directory and opening the store behind the version
//! gate.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};

use crate::error::{Error, Result};

/// The archive format this build writes and understands.
///
/// Bumped only when the archived bytes change meaning. The archive tables never
/// migrate and there is no conversion, so a store written in an older format is
/// refused ([`Error::ArchiveFormatTooOld`]) rather than rebuilt: a rebuild
/// would read the old bytes as the new format and fail every session.
///
/// Format 2 is numbered block rows in `blocks`, each with its compressed
/// bytes and a BLAKE3 hash of its uncompressed bytes, and `session_meta` holds
/// the block size and a checksum over those hashes.
pub const ARCHIVE_FORMAT: i64 = 2;

/// The derived-table schema this build writes.
///
/// Bumped whenever anything rebuildable from the archive changes shape: `turns`,
/// `compactions`, `session_titles`, `turns_fts`, `entities`, `paths`, or what a
/// rule writes into them, as `is_typed`'s did at 6. A separate integer from
/// [`ARCHIVE_FORMAT`], because one cannot tell "derived tables need a rebuild"
/// from "the archive itself changed". A store with an older number has its
/// derived tables rebuilt from the archive by
/// [`crate::reindex::open_up_to_date`].
pub const DERIVED_SCHEMA: i64 = 6;

/// The store file inside the data directory.
pub const DB_FILE_NAME: &str = "terminus.db";

/// `meta` key holding [`ARCHIVE_FORMAT`].
pub const META_ARCHIVE_FORMAT: &str = "archive_format";
/// `meta` key holding [`DERIVED_SCHEMA`].
pub const META_DERIVED_SCHEMA: &str = "derived_schema";

/// How long a writer waits behind another writer before giving up. Exclusivity
/// is the `LOCK` file's job, not the busy handler's; this is only there
/// so a short overlap with a reader is not an error.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// How many prepared statements the writable connection keeps. `derive_turn`
/// and the archive reads go through `prepare_cached`, and a loop compiles each
/// statement once only while all of its statements fit. Twice rusqlite's
/// default of 16, so one ingest or `reindex` loop fits with room to spare.
const STATEMENT_CACHE_CAPACITY: usize = 32;

/// The store's derived schema differs from this build's and its derived tables
/// need rebuilding.
///
/// Returned as an inspectable outcome rather than acted on here: the rebuild
/// lives in `reindex`. The archive is never part of the rebuild, and an archive
/// format other than this build's never gets this far: newer and older are both
/// refused by the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildRequired {
    pub store_archive_format: i64,
    pub store_derived_schema: i64,
    pub binary_archive_format: i64,
    pub binary_derived_schema: i64,
}

/// A column this build declares that the open store does not carry.
///
/// Only a read-only open can produce one: [`Store::open`] runs
/// [`bring_forward`] and adds it. Both strings come from
/// [`crate::store::schema::BRING_FORWARD_COLUMNS`], never from the database, so
/// a caller may compare them against that list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingColumn {
    pub table: &'static str,
    pub column: &'static str,
}

/// An open terminus store.
pub struct Store {
    conn: Connection,
    data_dir: PathBuf,
    path: PathBuf,
    rebuild: Option<RebuildRequired>,
    missing: Vec<MissingColumn>,
    missing_tables: Vec<&'static str>,
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
    /// before a writable one is opened: a refused open must leave
    /// `terminus.db`, its WAL and the `runs` table byte-for-byte unchanged, and
    /// read-only is what makes that true at the SQLite level rather than by
    /// discipline. A writable connection would checkpoint the WAL when the last
    /// one closes, which rewrites both files without a single statement of ours.
    ///
    /// Everything this creates is owner-only from the syscall that creates it:
    /// the data directory 0700 and `terminus.db` 0600, the latter created
    /// empty by us BEFORE rusqlite ever sees the path. `-wal` and `-shm` are
    /// then SQLite's to make, and it `fchmod`s a fresh sidecar to the main
    /// database's mode after opening it, which is why 0600 on the one file
    /// covers all three and why there is no `chmod` after the fact here - that
    /// would be a window with session text already on disk at 0644.
    pub fn open(data_dir: &Path) -> Result<Self> {
        crate::owner_only::create_dir_all(data_dir).map_err(|e| Error::io(data_dir, e))?;
        let path = data_dir.join(DB_FILE_NAME);

        // A zero-length file is exactly what `inspect` reads as
        // `StoreState::Fresh`, so this changes nothing about what happens next
        // on a store that does not exist yet: `initialize` still runs, and
        // SQLite's `journal_mode` stamp lands in a file that is already 0600.
        // `create_new`, so an existing store is never truncated - its
        // `AlreadyExists` is the ordinary case, not a failure.
        match crate::owner_only::options()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io(&path, e)),
        }

        // Before any SQLite connection, read-only ones included: a `-shm` SQLite
        // is about to create takes the main database's mode, so `terminus.db`
        // has to be narrow first or the sidecar of a wide store is born wide.
        tighten(data_dir);

        let state = inspect(&path)?;

        let rebuild = match state {
            StoreState::Initialized => gate(&path)?,
            StoreState::Fresh => None,
        };

        let conn = Connection::open(&path).map_err(Error::Sqlite)?;
        conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;

        let store = Store {
            conn,
            data_dir: data_dir.to_path_buf(),
            path,
            rebuild,
            // `bring_forward` runs below on every existing store and
            // `initialize` writes the whole schema on a fresh one, so a writable
            // open never leaves a declared column absent.
            missing: Vec::new(),
            // The same sentence, one level up: `bring_forward`'s missing-table
            // arm creates a table this build declares and the store lacks, so a
            // table added in a later release reaches an existing store with no
            // `DERIVED_SCHEMA` bump.
            missing_tables: Vec::new(),
        };
        if state == StoreState::Fresh {
            store.initialize()?;
        } else {
            // Every open of an existing store, and deliberately NOT gated on
            // `rebuild_required`. `reindex` and the recovery at the top of an
            // ingest pass act on that outcome through `reindex::up_to_date`,
            // while `compact`, `observations regenerate` and the snapshot taken
            // after a pass open through here alone - so gating `bring_forward`
            // on it would leave them naming a table the store lacks. Creating a
            // missing table is safe on any writable opener; the destructive
            // derived-table rebuild stays
            // exactly where it is, outside this function and inside the ingest
            // lock.
            bring_forward(&store.conn)?;
        }
        Ok(store)
    }

    /// Open an existing store for reading, and do nothing else to it.
    ///
    /// `SQLITE_OPEN_READ_ONLY`, no `create_dir_all`, no `initialize`, no
    /// `bring_forward`, no `pragma_update`. A `readOnlyHint` server may not
    /// perform DDL on connect, `terminus search` may not create a store as a
    /// side effect of a read, and a store on read-only media must return an
    /// empty result rather than fail to open at all - which are three
    /// statements about the same connection flags.
    ///
    /// The version gate still runs, because it is already read-only and because
    /// [`Store::rebuild_required`] is what lets a read command say the store
    /// predates this build instead of silently querying an old-shape index.
    ///
    /// Skipping `bring_forward` means a table or column this build declares is
    /// not guaranteed to be there, and a read query naming one fails with
    /// `no such table` or `no such column` on a connection that cannot `CREATE`
    /// or `ALTER`. So their absence is measured here, once, and reported through
    /// [`Store::missing_tables`] and [`Store::missing_columns`] - the degraded
    /// read - rather than left to surface as a SQLite error from the middle of
    /// a search.
    ///
    /// A missing data directory or database file, or a database with no
    /// tables, is [`Error::StoreNotFound`] and any other failure to open is
    /// [`Error::StoreUnreadable`]: both are variants and neither is a panic,
    /// because a read command renders them as an empty result with a reason
    /// and a caller must not have to match on message text to tell them apart.
    pub fn open_read_only(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join(DB_FILE_NAME);

        // Nothing is created, and that includes the probe: `fs::metadata` on a
        // path that is not there is the whole of "a read against a machine that
        // has never ingested".
        match std::fs::metadata(&path) {
            // A zero-length file is what an interrupted `initialize` leaves
            // (see `inspect`), and it holds no store to read.
            Ok(m) if m.len() == 0 => return Err(Error::StoreNotFound { path }),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::StoreNotFound { path })
            }
            Err(e) => {
                return Err(Error::StoreUnreadable {
                    path,
                    detail: e.to_string(),
                })
            }
        }

        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn =
            Connection::open_with_flags(&path, flags).map_err(|source| Error::StoreUnreadable {
                path: path.clone(),
                detail: source.to_string(),
            })?;
        conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))?;

        // SQLite opens lazily, so this first statement is where a WAL database
        // whose shared-memory index cannot be created actually fails. Mapping it
        // here is what keeps that case an empty result with a reason rather than
        // a bare `unable to open database file` out of the query layer.
        let tables = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|source| Error::StoreUnreadable {
                path: path.clone(),
                detail: source.to_string(),
            })?;
        // `Store::open` stamps `journal_mode=wal` before `initialize` commits,
        // so a non-empty file with no tables is a store another process is still
        // creating: no store yet. Tables but no `meta` is somebody else's file,
        // and `read_versions` says so.
        if tables == 0 {
            return Err(Error::StoreNotFound { path });
        }

        let rebuild = read_versions(&conn, &path)?;
        let missing = missing_columns(&conn)?;
        let missing_tables = missing_tables(&conn)?;

        Ok(Store {
            conn,
            data_dir: data_dir.to_path_buf(),
            path,
            rebuild,
            missing,
            missing_tables,
        })
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
    ///
    /// The transaction is IMMEDIATE because two opens of a fresh directory can
    /// both read it as fresh. When the other open has already created the
    /// tables, every `CREATE TABLE IF NOT EXISTS` here only reads, so a deferred
    /// transaction holds a read lock by the time the `meta` insert needs to
    /// write. SQLite does not call the busy handler for a read lock upgrading to
    /// a write lock, so that insert failed at once with "database is locked"
    /// instead of waiting out `BUSY_TIMEOUT_MS`. Taking the write lock first
    /// means the wait happens, and the statements are idempotent, so running
    /// them again on a store the other open finished is harmless.
    fn initialize(&self) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
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

    /// Columns this build declares and the open store does not carry.
    ///
    /// Always empty for [`Store::open`], which adds them. A read-only open
    /// cannot, so this is the list of columns its caller must not name in a
    /// query - and, since only a store older than this build can be missing
    /// one, the evidence for [`Store::predates_this_build`].
    pub fn missing_columns(&self) -> &[MissingColumn] {
        &self.missing
    }

    /// Tables this build declares and the open store does not carry.
    ///
    /// The table-level counterpart of [`Store::missing_columns`], and it exists
    /// because the column list cannot answer for a whole table: `decisions`
    /// and `labels` sit in [`crate::store::schema::TABLES`] with no
    /// `DERIVED_SCHEMA` bump behind them, so on a store written by an older
    /// build `rebuild` is `None` and [`Store::missing_columns`] - which walks
    /// `BRING_FORWARD_COLUMNS` alone - is empty. Both terms of
    /// [`Store::predates_this_build`] are false and the store reads as current,
    /// while a query naming either table fails with `no such table`.
    ///
    /// Deliberately NOT folded into [`Store::predates_this_build`]. That flag
    /// makes every read command print the degraded-results line, and a
    /// store missing only the decision log answers `search`, `show` and
    /// `sessions` perfectly well - the absence is a fact about one report, so it
    /// is reported to the commands whose answer depends on it and to no others.
    ///
    /// Always empty for [`Store::open`], which creates them.
    pub fn missing_tables(&self) -> &[&'static str] {
        &self.missing_tables
    }

    /// Is this store older than the build reading it?
    ///
    /// Either half is enough: version integers that do not match, or a column
    /// this build's queries name and the store does not carry. A read command
    /// says so and queries what is there; it never repairs, because opening a
    /// store is a read.
    pub fn predates_this_build(&self) -> bool {
        self.rebuild.is_some() || !self.missing.is_empty()
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

/// Narrow a store something left readable by group or world.
///
/// Runs on every [`Store::open`] and on NO read-only open: `doctor`,
/// `search`, `show` and `replay` report on a store, and reporting on something
/// is not permission to change it. Its counterpart is that a writable open is
/// already a write, so repairing there costs the caller nothing it was not
/// already paying.
///
/// Compare, then chmod - never chmod unconditionally. A `chmod` to the
/// mode a file already has still moves its ctime, and "the repair ran once" is
/// exactly what a second open leaving every ctime alone is the evidence for.
/// There is no marker key recording the repair as done, deliberately: a store
/// widened again later - by a restore from backup, by a `data move`, by a hand
/// `chmod` - is tightened again, where a marker or a version gate would decide
/// it had already been handled. The cost is a handful of `stat` calls per open.
///
/// Every failure is ignored and the open proceeds. The alternative is an
/// ingest that stops dead on a store whose files another account owns, or one on
/// a read-only mount, which trades a mode this build cannot fix for a product
/// that does not run.
fn tighten(data_dir: &Path) {
    narrow_directory(data_dir);

    for name in [
        DB_FILE_NAME.to_string(),
        // SQLite's sidecars are named off the database file, and both can
        // outlive the process that made them.
        format!("{DB_FILE_NAME}-wal"),
        format!("{DB_FILE_NAME}-shm"),
        crate::ingest::lock::LOCK_FILE_NAME.to_string(),
    ] {
        narrow_file(&data_dir.join(name));
    }

    // Each holds one kind of thing terminus wrote: whole-archive copies,
    // injected brief text, per-prompt decision records. The directory and the
    // files directly inside it, one level, because none of the three nests.
    for name in [
        crate::store::snapshot::DIR_NAME,
        crate::inject::state::DIR_NAME,
        crate::inject::decision::DIR_NAME,
    ] {
        let dir = data_dir.join(name);
        narrow_directory(&dir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            narrow_file(&entry.path());
        }
    }
}

fn narrow_directory(path: &Path) {
    narrow(path, crate::owner_only::DIR_MODE, std::fs::FileType::is_dir);
}

fn narrow_file(path: &Path) {
    narrow(
        path,
        crate::owner_only::FILE_MODE,
        std::fs::FileType::is_file,
    );
}

/// One path: stat it, and chmod only if a bit is actually wrong.
///
/// `symlink_metadata`, and a file-type test that admits only the kind the
/// caller asked for. `set_permissions` follows a symlink, so a link left in
/// `snapshots/` would otherwise have its TARGET chmod'd - a file that may sit
/// anywhere on the machine and be nothing of terminus's. A path that is not
/// there at all is the ordinary case, not a failure: most of this list is
/// absent on most stores.
fn narrow(path: &Path, mode: u32, wanted: fn(&std::fs::FileType) -> bool) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !wanted(&meta.file_type()) {
        return;
    }
    if meta.permissions().mode() & 0o777 == mode {
        return;
    }
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

/// Add whatever this build's schema has and the open store does not.
///
/// Additive only, and idempotent by construction: it creates a table this build
/// declares and the store lacks, and adds a column this build declares and the
/// table lacks. It drops nothing, renames nothing and rewrites nothing, which
/// is what makes it safe to run on a read command's open. `sessions` and
/// `session_meta` are archive tables and an added column is the only change
/// either is ever allowed.
///
/// Nothing is written when nothing is missing - the common case is three read
/// queries and no transaction at all - so a plain reopen of an up-to-date store
/// still touches no byte of it.
///
/// When something is missing, the list is read again inside an IMMEDIATE
/// transaction before anything is written. Two processes opening a store that
/// lacks a table would otherwise both see it missing under a deferred
/// transaction's read lock, and once the first had created it, the second's
/// read lock could not become the write lock its own `CREATE` needs: SQLite
/// fails that upgrade at once with "database is locked" instead of calling the
/// busy handler. Taking the write lock first means the second open waits, reads
/// the list again and finds nothing left to add. A column listed in
/// [`crate::store::schema::BRING_FORWARD_COLUMNS`] is covered the same way.
fn bring_forward(conn: &Connection) -> Result<()> {
    let (missing_tables, missing_columns) = missing(conn)?;
    if missing_tables.is_empty() && missing_columns.is_empty() {
        return Ok(());
    }

    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let (missing_tables, missing_columns) = missing(&tx)?;
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

/// A column this build declares: `(table, column, declaration)`.
type Column = (&'static str, &'static str, &'static str);

/// The tables and the columns this build declares and the open store lacks.
fn missing(conn: &Connection) -> Result<(Vec<&'static str>, Vec<Column>)> {
    let present_tables = table_names(conn)?;
    let missing_tables: Vec<&'static str> = crate::store::schema::TABLES
        .iter()
        .copied()
        .filter(|t| !present_tables.iter().any(|p| p == t))
        .collect();

    let mut missing_columns = Vec::new();
    for (table, columns) in crate::store::schema::BRING_FORWARD_COLUMNS {
        // A table this build is about to create arrives with every column
        // already on it, so asking `PRAGMA table_info` about it would only
        // schedule columns that are not missing.
        if missing_tables.contains(table) {
            continue;
        }
        let present = column_names(conn, table)?;
        for (name, declaration) in *columns {
            if !present.iter().any(|c| c == name) {
                missing_columns.push((*table, *name, *declaration));
            }
        }
    }
    Ok((missing_tables, missing_columns))
}

/// Which of [`crate::store::schema::BRING_FORWARD_COLUMNS`] the open store does
/// not carry.
///
/// The read-only counterpart of the scheduling half of [`bring_forward`]: same
/// list, same `PRAGMA table_info` comparison, and no `ALTER`. A table that is
/// missing outright reports every one of its columns, which is the honest
/// answer - a query naming one of them would fail either way.
fn missing_columns(conn: &Connection) -> Result<Vec<MissingColumn>> {
    let mut missing = Vec::new();
    for (table, columns) in crate::store::schema::BRING_FORWARD_COLUMNS {
        let present = column_names(conn, table)?;
        for (name, _) in *columns {
            if !present.iter().any(|c| c == name) {
                missing.push(MissingColumn {
                    table,
                    column: name,
                });
            }
        }
    }
    Ok(missing)
}

/// Which of [`crate::store::schema::TABLES`] the open store does not carry.
///
/// The read-only counterpart of [`bring_forward`]'s missing-table arm: same
/// list, same `sqlite_master` comparison, and no `CREATE`. A read never
/// migrates, so this is what its caller must not query.
fn missing_tables(conn: &Connection) -> Result<Vec<&'static str>> {
    let present = table_names(conn)?;
    Ok(crate::store::schema::TABLES
        .iter()
        .filter(|table| !present.iter().any(|name| name == *table))
        .copied()
        .collect())
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

/// Decide whether the path holds a store, without writing a byte unless SQLite
/// needs a crashed transaction rolled back first.
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
/// **read-only** connection, so "a refused open leaves the store byte-for-byte
/// unchanged" survives it. The one exception is a hot journal,
/// which only a writable connection can roll back. Refusing it instead would
/// leave a store killed during its first open refused by every open after.
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

    let count = |conn: &Connection| {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |r| r.get::<_, i64>(0),
        )
    };
    let tables = match count(&conn) {
        // A process killed mid-transaction leaves a hot journal, and SQLite
        // will not read past it until it is rolled back, which a read-only
        // connection cannot do. The first open of a new store is where that
        // happens: setting WAL mode writes the header through a rollback
        // journal. A writable connection rolls it back on its first read, to
        // the last committed state. This is the only write `inspect` makes.
        Err(e)
            if e.sqlite_error().map(|e| e.extended_code)
                == Some(rusqlite::ffi::SQLITE_READONLY_ROLLBACK) =>
        {
            drop(conn);
            Connection::open(path).and_then(|conn| count(&conn))
        }
        counted => counted,
    }
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
/// build, `Ok(Some(..))` when only its derived schema differs, and an error
/// when its archive format is newer or older.
fn gate(path: &Path) -> Result<Option<RebuildRequired>> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(path, flags).map_err(|source| Error::NotAStore {
        path: path.to_path_buf(),
        detail: format!("cannot open it for reading: {source}"),
    })?;
    conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS.into()))?;

    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let outcome = read_versions(&tx, path)?;

    // Explicit: the read transaction and the read-only connection are both gone
    // before any writable connection is opened.
    drop(tx);
    drop(conn);
    Ok(outcome)
}

/// The gate's decision, over a connection the caller already holds.
///
/// Split out so [`Store::open_read_only`] runs the same rule on its own
/// read-only connection instead of opening a second one: the read path is the
/// cold-start path, and two connects to answer one question is a cost the hook
/// budget notices. [`gate`] keeps its own connection because a writable one is
/// opened right after it and must not exist while the versions are read.
fn read_versions(conn: &Connection, path: &Path) -> Result<Option<RebuildRequired>> {
    let read = |key: &str| -> Result<Option<i64>> {
        let text: Option<String> = conn
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

    // Refused, never rebuilt. A rebuild here would create the new
    // archive tables empty inside the old store, fail every session in
    // `reindex` and stamp the new format over bytes only the old one reads.
    // Both opens reach this before any writable connection, so `bring_forward`
    // never runs on such a store.
    if store_archive < ARCHIVE_FORMAT {
        return Err(Error::ArchiveFormatTooOld {
            path: path.to_path_buf(),
            store_format: store_archive,
            binary_format: ARCHIVE_FORMAT,
        });
    }

    let outcome = if store_derived != DERIVED_SCHEMA {
        // A *newer* derived_schema lands here too, and deliberately: derived
        // tables are rebuildable from the archive by definition, so rebuilding
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

    Ok(outcome)
}

/// The file in terminus's config directory that names where the data directory
/// lives.
///
/// Plain text holding one absolute path, and deliberately NOT TOML: the `toml`
/// dependency in the root `Cargo.toml` is `default-features = false` with the
/// serializer half absent, so a TOML pointer would cost a new cargo feature on
/// the hook-path binary to WRITE a single line. `terminus data move` writes it;
/// nothing else does.
pub const LOCATION_FILE_NAME: &str = "data-location";

/// Where [`LOCATION_FILE_NAME`] sits, for the one command that writes it.
///
/// An `Err` here is a config directory that does not resolve at all, which the
/// READ side treats as "no pointer" ([`data_dir`]) and the write side must
/// treat as a failure - there is nowhere to put the pointer, so the move cannot
/// be made to stick.
pub fn location_pointer_path() -> Result<PathBuf> {
    Ok(crate::config::config_dir()?.join(LOCATION_FILE_NAME))
}

/// Record that the data directory now lives at `destination`, for `terminus
/// data move` and nothing else.
///
/// The text is the destination and one newline, written through a temporary
/// and a rename ([`owner_only::replace`](crate::owner_only::replace)). Atomic
/// because the alternative is a truncated pointer: a process killed mid-write
/// would leave a file naming half a path, and the next command would resolve a
/// store that does not exist rather than either the old one or the new one.
///
/// The pointer is 0600 from the create. The config directory is made 0700 only
/// when this call makes it: one that already exists keeps its mode. A move
/// killed mid-write can leave `.data-location-{pid}-{n}.tmp` behind in the
/// config directory, and nothing deletes it.
pub fn write_location_pointer(destination: &Path) -> Result<()> {
    let pointer = location_pointer_path()?;
    if let Some(dir) = pointer.parent() {
        crate::owner_only::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let text = format!("{}\n", destination.display());
    crate::owner_only::replace(&pointer, text.as_bytes(), ".data-location-", ".tmp")
        .map_err(|e| Error::io(&pointer, e))
}

/// The data directory, resolved once at startup and passed down explicitly
/// afterwards, never re-derived.
///
/// Three sources, in this order and no other:
///
/// 1. `TERMINUS_DATA_DIR`, which outranks everything. A test bench and a
///    spawned child that set it behave exactly as they did before the pointer
///    existed, which is what makes the pointer a change nothing else has to
///    know about.
/// 2. The [`LOCATION_FILE_NAME`] pointer `terminus data move` writes.
/// 3. The platform location, which is where every user starts.
///
/// **This runs on the hook path**, where `cmd::hook` resolves the data
/// directory on every event against a p99 asserted at 10 ms with a measured
/// 0.408 ms floor. So the pointer costs one `fs::read_to_string` of a small
/// file that usually is not there, reached only when the environment variable
/// is unset, and nothing else.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_var("TERMINUS_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = pointed_at()? {
        return Ok(dir);
    }
    platform_data_dir()
}

/// The store location the pointer file names, if it names one.
///
/// `Ok(None)` is the ordinary answer and covers three states that are all
/// "nobody has moved the store": no config directory resolves - because a store
/// path that stopped resolving when `HOME` moved would take every command down
/// with it, and the platform location is a better answer than an error; no
/// pointer file, which is where every user starts; and a pointer that is empty
/// or whitespace-only, treated as unset the way [`non_empty_var`] treats an
/// empty environment variable.
///
/// A pointer file that EXISTS and cannot be read is an `Err`, and the
/// distinction is the point: falling through to the platform location there
/// would silently start a second store beside the one the user moved, and both
/// would then be written to. Not-there is normal; there-and-unreadable is not.
fn pointed_at() -> Result<Option<PathBuf>> {
    let Ok(path) = crate::config::config_dir() else {
        return Ok(None);
    };
    let path = path.join(LOCATION_FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error::DataDirUnresolved {
                detail: format!("{} exists and could not be read: {e}", path.display()),
            })
        }
    };

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    // A relative pointer would resolve against the process's working directory,
    // which on the hook path is whatever project Claude Code was invoked in - so
    // it would scatter a store per repository rather than name one. Refused
    // loudly rather than ignored: the file was hand-edited to say something, and
    // silently using a different store than the one it names is the failure this
    // whole resolver exists to avoid.
    let pointed = PathBuf::from(trimmed);
    if !pointed.is_absolute() {
        return Err(Error::DataDirUnresolved {
            detail: format!(
                "{} names a relative path ({trimmed}); it must hold one absolute path",
                path.display()
            ),
        });
    }
    Ok(Some(pointed))
}

#[cfg(target_os = "linux")]
fn platform_data_dir() -> Result<PathBuf> {
    if let Some(xdg) = non_empty_var("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg).join("terminus"));
    }
    let home = non_empty_var("HOME").ok_or_else(|| Error::DataDirUnresolved {
        detail: "neither TERMINUS_DATA_DIR, XDG_DATA_HOME nor HOME is set".into(),
    })?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("terminus"))
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Result<PathBuf> {
    let home = non_empty_var("HOME").ok_or_else(|| Error::DataDirUnresolved {
        detail: "neither TERMINUS_DATA_DIR nor HOME is set".into(),
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("terminus"))
}

/// An environment variable set to the empty string is treated as unset: an
/// empty path would resolve to the process's current directory.
fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The losing open of a fresh directory, made deterministic. Both opens
    /// read the directory as fresh, the winner has already written the
    /// tables, and its backfill holds the write lock when the loser's
    /// `initialize` starts. The lock is released on a timer well inside the
    /// busy timeout, so a deferred transaction fails before it is released.
    #[test]
    fn a_losing_initialize_waits_for_the_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let winner = Store::open(dir.path()).unwrap();
        let loser = Store::open(dir.path()).unwrap();

        let path = winner.path().to_path_buf();
        let (held, lock_held) = mpsc::channel();
        let backfill = std::thread::spawn(move || {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "BEGIN IMMEDIATE; \
                 INSERT OR REPLACE INTO meta (key, value) VALUES ('backfill', '1');",
            )
            .unwrap();
            held.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(1));
            conn.execute_batch("COMMIT").unwrap();
        });
        lock_held.recv().unwrap();

        loser.initialize().unwrap();
        backfill.join().unwrap();

        let read = |key: &str| -> String {
            loser
                .conn()
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(read(META_ARCHIVE_FORMAT), ARCHIVE_FORMAT.to_string());
        assert_eq!(read(META_DERIVED_SCHEMA), DERIVED_SCHEMA.to_string());
    }
}
