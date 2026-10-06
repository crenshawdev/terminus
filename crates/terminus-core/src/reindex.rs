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
//!
//! # The one case where the blob cannot answer (phase 8 D-03)
//!
//! "Every table is dropped before a single row is read" is exactly false for a
//! store holding an EVICTED session, and the exception is deliberate. Retention
//! empties such a session's blob on purpose (RET-02) while leaving it listed
//! and searchable, and its derived rows are the only thing left that can answer
//! either: `turns_fts` is declared `content=''`, so the projected body text is
//! not readable back out of it, and the record bytes [`crate::index::project`]
//! built it from are gone. Dropping the table would destroy them with nothing
//! anywhere able to reproduce them.
//!
//! So a store with at least one evicted session takes a second path: the
//! derived rows of the sessions that are ABOUT to be rebuilt are deleted by
//! turn id, those sessions are rebuilt, and the evicted sessions' rows are left
//! standing and counted on [`Rebuilt::preserved`]. Every store with no evicted
//! session - which is every store until a user writes a `[retention]` table -
//! takes the drop-and-recreate path unchanged.
//!
//! The two paths produce the same rows for every session that has a blob,
//! because [`crate::derive::derive_turn`] is idempotent by delete-then-insert
//! at a known rowid (D-10) rather than by anything the empty tables provided.
//!
//! **What the preserved count is honestly saying.** Those rows were written by
//! whatever build ingested that session, so a future change to a derived
//! table's SHAPE reaches the rebuilt sessions and not the preserved ones. That
//! is a real limit and this is where it is recorded; the alternative is losing
//! them inside the ingest lock - `open_up_to_date` runs at the top of every
//! pass - with a `Rebuilt::failed` note to show for it, which is silent data
//! loss reporting as retention working correctly.

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
    /// Evicted sessions whose derived rows were carried across rather than
    /// rebuilt (phase 8 D-03).
    ///
    /// Distinct from [`Rebuilt::failed`], and the distinction is the whole
    /// point: a failure is a session whose blob is damaged and whose index is
    /// now gone, a preserved session is one whose blob was emptied on purpose
    /// and whose index is intact and still the only copy. Counted rather than
    /// silent because rows carried across are rows this build did not write -
    /// see the module docs.
    pub preserved: usize,
}

/// Drop every derived table and rebuild it from the session blobs.
///
/// One transaction: a rebuild that committed half way would leave the store
/// with derived tables that describe some sessions and not others, and no
/// record of which.
pub fn reindex(store: &mut Store) -> Result<Rebuilt> {
    let tx = store.conn_mut().transaction()?;

    // Keys first, blobs one at a time. Selecting `blob` here too would hold the
    // whole archive in memory at once - 895 MB of it today and growing with
    // every session ever archived - and this runs inside the ingest lock on the
    // first hook-spawned pass after an upgrade, where an allocation failure
    // rolls the rebuild back, leaves `derived_schema` behind, and takes the
    // pass with it before `record_pass` can say so. One blob at a time bounds
    // it to the largest single session.
    //
    // Read BEFORE anything is dropped, which the drop path did not need and
    // costs it nothing: `sessions` and `session_meta` are never touched here,
    // so the same rows are there either way, and the eviction flag has to be
    // known before the choice of path can be made.
    let keys: Vec<(String, i64, bool)> = tx
        .prepare(
            "SELECT s.session_key, s.session_no, coalesce(m.is_evicted, 0) <> 0
               FROM sessions s LEFT JOIN session_meta m USING (session_key)
              ORDER BY s.session_no",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;

    let preserved = keys.iter().filter(|(_, _, evicted)| *evicted).count();
    let mut rebuilt = Rebuilt {
        preserved,
        ..Rebuilt::default()
    };

    if preserved == 0 {
        // Every store until a user writes a `[retention]` table, and the
        // stronger guarantee: nothing survives that the blobs did not produce.
        //
        // Children first, and that is not tidiness. The bundled SQLite is
        // compiled with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`
        // (libsqlite3-sys-0.38.1/build.rs:126), so foreign keys are enforced on
        // every connection whatever `crates/terminus-core/src/store/schema.rs`
        // says about the pragma, and dropping `turns` while `entities`, `paths`
        // or `compaction_boundaries` still hold rows fails with a constraint
        // violation. `DERIVED_TABLES` is in creation order, so a rebuild drops
        // in reverse.
        for table in schema::DERIVED_TABLES.iter().rev() {
            tx.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))?;
        }
    }
    // Every statement in the schema is `IF NOT EXISTS`, so this recreates
    // exactly the tables that were dropped and leaves the archive alone. One
    // definition of the schema, rather than a second copy that can drift. Run
    // on the preserving path too, where it creates nothing and is the one thing
    // that keeps a store missing a derived table from failing the rebuild
    // outright instead of rebuilding what it still can.
    tx.execute_batch(schema::CREATE_SQL)?;

    for (session_key, session_no, evicted) in keys {
        // Left exactly as it is, and not counted as rebuilt: `sessions` here
        // would claim this build wrote rows it did not write.
        if evicted {
            continue;
        }
        // The preserving path's equivalent of the drop, and only there: the
        // rows of the session about to be rebuilt, and no others. It is what
        // makes the two paths agree about a session whose stored stream now
        // yields FEWER turns than the tables hold - `derive_turn` overwrites at
        // a known rowid but cannot know about a row nothing re-derives. On the
        // drop path the tables were just recreated empty, so issuing it there
        // as well would be five statements per session for nothing, on the
        // upgrade path, inside the ingest lock, times every session ever
        // archived.
        if preserved > 0 {
            clear_derived(&tx, &session_key)?;
        }
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

/// Every derived row one session owns, deleted, children first.
///
/// The preserving path's stand-in for `DROP TABLE`, scoped to one session
/// (phase 8 D-03). Children first for the reason the drop order documents: the
/// bundled SQLite is compiled with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`, so
/// `paths`, `entities` and `compaction_boundaries` really do reference
/// `turns(id)` on every connection, and deleting the parent first fails with a
/// constraint violation. `turns_fts` is contentless with `rowid` IS `turns.id`,
/// so it is cleared by the same subquery rather than by a join it cannot serve.
///
/// The subquery is re-evaluated per statement rather than materialized into a
/// list: the ids are `turns.id` values this transaction is about to overwrite
/// anyway, and a `WHERE turn_id IN (SELECT ...)` keeps the whole thing one
/// statement SQLite plans against the `UNIQUE (session_key, turn_seq)` index.
fn clear_derived(tx: &rusqlite::Connection, session_key: &str) -> Result<()> {
    const TURN_IDS: &str = "SELECT id FROM turns WHERE session_key = ?1";
    for statement in [
        format!("DELETE FROM paths WHERE turn_id IN ({TURN_IDS})"),
        format!("DELETE FROM entities WHERE turn_id IN ({TURN_IDS})"),
        format!("DELETE FROM compaction_boundaries WHERE turn_id IN ({TURN_IDS})"),
        format!("DELETE FROM turns_fts WHERE rowid IN ({TURN_IDS})"),
        "DELETE FROM turns WHERE session_key = ?1".to_owned(),
    ] {
        tx.execute(&statement, [session_key])?;
    }
    Ok(())
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
