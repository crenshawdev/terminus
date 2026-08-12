//! `verbatim status`: what the store holds and what the last run did (ING-09).
//!
//! Every number comes out of the store. There is no log file, by design, so
//! `runs` is where a pass's failures live and this is what surfaces them.
//!
//! It takes no ingest lock. Reading while a pass runs must work, which is what
//! WAL is on for, and a status command that blocked behind an ingest would be
//! useless exactly when it is wanted.
//!
//! `rusqlite` is deliberately not named here: the binary crate does not depend
//! on it, and a read command is not a reason to put a second SQL dependency in
//! the hook path's build. The connection comes from `Store::conn` and every
//! query's error is mapped by [`read`].

use std::fmt::Display;
use std::path::Path;

use verbatim_core::store::{Store, DB_FILE_NAME};

use super::Failure;

/// A store or filesystem error is operational (exit 1), never misuse.
fn read<T, E: Display>(result: std::result::Result<T, E>) -> Result<T, Failure> {
    result.map_err(|e| Failure::Operational(e.to_string()))
}

pub fn run() -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    let store = Store::open(&data_dir)?;
    let conn = store.conn();

    let sessions: i64 = read(conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0)))?;
    let turns: i64 = read(conn.query_row("SELECT count(*) FROM turns", [], |r| r.get(0)))?;
    let (watermarks, covered): (i64, i64) = read(conn.query_row(
        "SELECT count(*), coalesce(sum(byte_offset), 0) FROM watermarks",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ))?;

    println!("store          {}", store.path().display());
    println!("size           {}", size(&data_dir));
    println!("sessions       {sessions}");
    println!("turns          {turns}");
    println!("watermarks     {watermarks} covering {covered} byte(s)");

    // Counted first rather than reached for with an optional row, so that "no
    // run yet" and "the query failed" stay two different answers.
    let runs: i64 = read(conn.query_row("SELECT count(*) FROM runs", [], |r| r.get(0)))?;
    if runs == 0 {
        // An empty store is not a failure: it is what a machine looks like
        // before the first hook has ever fired.
        println!("last run       none");
        return Ok(());
    }

    let run: Run = read(conn.query_row(
        "SELECT started_at, coalesce(duration_ms, 0), files_seen, files_committed,
                files_failed, bytes_read, turns_added, error
         FROM runs ORDER BY id DESC LIMIT 1",
        [],
        |r| {
            Ok(Run {
                started_at: r.get(0)?,
                duration_ms: r.get(1)?,
                files_seen: r.get(2)?,
                files_committed: r.get(3)?,
                files_failed: r.get(4)?,
                bytes_read: r.get(5)?,
                turns_added: r.get(6)?,
                error: r.get(7)?,
            })
        },
    ))?;

    println!("last run       {}", run.started_at);
    println!("  duration     {} ms", run.duration_ms);
    println!(
        "  files        {} walked, {} committed, {} failed",
        run.files_seen, run.files_committed, run.files_failed
    );
    println!("  bytes read   {}", run.bytes_read);
    println!("  turns added  {}", run.turns_added);
    if let Some(error) = &run.error {
        // In full, every line of it. This is the only account of what that pass
        // skipped and why.
        println!("  error");
        for line in error.lines() {
            println!("    {line}");
        }
    }

    Ok(())
}

struct Run {
    started_at: String,
    duration_ms: i64,
    files_seen: i64,
    files_committed: i64,
    files_failed: i64,
    bytes_read: i64,
    turns_added: i64,
    error: Option<String>,
}

/// The store's footprint: the database and its WAL sidecars together, because
/// a hot WAL can hold a large share of what has been written.
fn size(data_dir: &Path) -> String {
    let mut total = 0u64;
    for name in [
        DB_FILE_NAME.to_owned(),
        format!("{DB_FILE_NAME}-wal"),
        format!("{DB_FILE_NAME}-shm"),
    ] {
        if let Ok(meta) = std::fs::metadata(data_dir.join(name)) {
            total += meta.len();
        }
    }
    format!("{total} byte(s) ({})", human(total))
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
