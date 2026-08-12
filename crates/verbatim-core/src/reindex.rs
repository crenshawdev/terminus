//! Rebuilding every derived table from the blobs alone (STOR-04).
//!
//! The input is the blobs and nothing else. Every table in
//! `schema::DERIVED_TABLES` - `turns`, `compaction_boundaries`, `turns_fts`,
//! `entities` and `paths` - is dropped before a single row is read, so a
//! rebuild cannot quietly carry forward a value only the old rows knew, which
//! is the property "everything else is derived" has to mean if it is to mean
//! anything.
//!
//! `sessions` and `session_meta` are never touched. The archive table never
//! migrates (`DESIGN-BRIEF.md:94`), and STOR-05's older-format path rebuilds
//! derived tables *only*.
//!
//! Turn ids come out identical because they are derived from `sessions.
//! session_no` and `turn_seq` (D-10), and `sessions` is never dropped. Without
//! that, AC4's fixed query set would return the same row count while pointing
//! at different turns.

use std::path::Path;

use crate::blob;
use crate::derive::{self, TurnRow};
use crate::error::Result;
use crate::parse;
use crate::store::{
    schema, Store, ARCHIVE_FORMAT, DERIVED_SCHEMA, META_ARCHIVE_FORMAT, META_DERIVED_SCHEMA,
};

/// What a rebuild produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rebuilt {
    pub sessions: usize,
    pub turns: usize,
    /// Sessions whose blob would not decompress, and why.
    ///
    /// Skipped rather than fatal, and that distinction is the whole of
    /// `DESIGN-BRIEF.md:98`: one damaged session means "this session is
    /// damaged", never "the store is gone". A `?` here aborted the entire
    /// rebuild on the first bad blob, named no session, and - because
    /// [`open_up_to_date`] runs on the ingest path - stopped every future
    /// ingest of every other transcript the moment `DERIVED_SCHEMA` was
    /// bumped. The archive is truth and a corrupt blob is not recoverable
    /// here, so the rebuild carries on and reports.
    pub failed: Vec<(String, String)>,
}

/// Drop every derived table and rebuild it from the session blobs.
///
/// One transaction: a rebuild that committed half way would leave the store
/// with derived tables that describe some sessions and not others, and no
/// record of which.
pub fn reindex(store: &mut Store) -> Result<Rebuilt> {
    let tx = store.conn_mut().transaction()?;

    // Children first, and that is not tidiness. The bundled SQLite is compiled
    // with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1` (libsqlite3-sys-0.38.1/build.rs:126),
    // so foreign keys are enforced on every connection whatever
    // `crates/verbatim-core/src/store/schema.rs` says about the pragma, and
    // dropping `turns` while `entities`, `paths` or `compaction_boundaries`
    // still hold rows fails with a constraint violation. `DERIVED_TABLES` is in
    // creation order, so a rebuild drops in reverse.
    for table in schema::DERIVED_TABLES.iter().rev() {
        tx.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))?;
    }
    // Every statement in the schema is `IF NOT EXISTS`, so this recreates
    // exactly the tables that were dropped and leaves the archive alone. One
    // definition of the schema, rather than a second copy that can drift.
    tx.execute_batch(schema::CREATE_SQL)?;

    // Keys first, blobs one at a time. Selecting `blob` here too would hold the
    // whole archive in memory at once - 895 MB of it today and growing with
    // every session ever archived - and this runs inside the ingest lock on the
    // first hook-spawned pass after an upgrade, where an allocation failure
    // rolls the rebuild back, leaves `derived_schema` behind, and takes the
    // pass with it before `record_pass` can say so. One blob at a time bounds
    // it to the largest single session.
    let keys: Vec<(String, i64)> = tx
        .prepare("SELECT session_key, session_no FROM sessions ORDER BY session_no")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;

    let mut rebuilt = Rebuilt::default();
    for (session_key, session_no) in keys {
        rebuilt.sessions += 1;
        let bytes: Vec<u8> = tx.query_row(
            "SELECT blob FROM sessions WHERE session_no = ?1",
            [session_no],
            |r| r.get(0),
        )?;
        let stream = match blob::read_all(&bytes) {
            Ok(stream) => stream,
            Err(e) => {
                // Named and skipped. `verify` is the command that reports this
                // in detail; a rebuild that stopped here would take every
                // undamaged session down with it.
                rebuilt.failed.push((session_key, e.to_string()));
                continue;
            }
        };
        let scan = parse::scan(&stream);
        for (record, turn) in scan.turns() {
            let from = record.offset as usize;
            derive::derive_turn(
                &tx,
                TurnRow {
                    session_key: &session_key,
                    session_no,
                    turn,
                    stream_offset: record.offset,
                    byte_len: record.len,
                    record: &stream[from..from + record.len as usize],
                    // The rebuild reads these off the blob's own bytes, exactly
                    // as ingest read them off the file's. Dropping them here
                    // would make a `reindex` silently lose every boundary row,
                    // which is the one way "derived and rebuildable" could be
                    // false while every count still matched.
                    subtype: record.subtype.as_deref(),
                    compact_metadata: record.compact_metadata.as_deref(),
                },
            )?;
            rebuilt.turns += 1;
        }
    }

    // The version integers move with the rebuild, inside the same transaction:
    // a store stamped up to date whose rebuild rolled back is the one state
    // this must not be able to produce.
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2), (?3, ?4)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![
            META_ARCHIVE_FORMAT,
            ARCHIVE_FORMAT.to_string(),
            META_DERIVED_SCHEMA,
            DERIVED_SCHEMA.to_string(),
        ],
    )?;

    tx.commit()?;
    Ok(rebuilt)
}

/// Open a store, rebuilding its derived tables first if it predates this build.
///
/// STOR-05's older-format branch. It lives here and not in [`Store::open`] on
/// purpose: opening a store is a read, and a read that silently rewrites four
/// tables is not one. `Store::open` reports the outcome
/// ([`Store::rebuild_required`]) and this is the caller that acts on it, so
/// every code path that can decide "no, refuse instead" still has that choice.
///
/// The store is reopened after the rebuild rather than reused, so the returned
/// handle reports the versions the store now actually carries.
pub fn open_up_to_date(data_dir: &Path) -> Result<Store> {
    let mut store = Store::open(data_dir)?;
    if store.rebuild_required().is_none() {
        return Ok(store);
    }

    reindex(&mut store)?;
    drop(store);

    let reopened = Store::open(data_dir)?;
    debug_assert!(reopened.rebuild_required().is_none());
    Ok(reopened)
}
