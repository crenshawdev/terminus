//! `terminus status`: what the store holds and what the last run did (ING-09).
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
//!
//! The session, turn and watermark numbers come from `config::visible`, not
//! from `count(*)`: an excluded project must be invisible on read as well as
//! never read on ingest (ING-08, D-23), including for sessions archived before
//! the exclusion was configured. This is the phase's only read command, so it
//! is also the proof that the boundary phase 3's search will reuse is real.

use std::fmt::Display;
use std::path::Path;

use serde_json::json;
use terminus_core::config::visible;
use terminus_core::store::{Store, DB_FILE_NAME};
use terminus_core::Config;

use super::json::Document;
use super::Failure;

/// A store or filesystem error is operational (exit 1), never misuse.
fn read<T, E: Display>(result: std::result::Result<T, E>) -> Result<T, Failure> {
    result.map_err(|e| Failure::Operational(e.to_string()))
}

pub fn run(json: bool) -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    let config = Config::load()?;
    let store = Store::open(&data_dir)?;
    let conn = store.conn();

    let counts = read(visible::counts(conn, &config))?;
    // Counted first rather than reached for with an optional row, so that "no
    // run yet" and "the query failed" stay two different answers.
    let runs: i64 = read(conn.query_row("SELECT count(*) FROM runs", [], |r| r.get(0)))?;
    let last: Option<Run> = if runs == 0 {
        None
    } else {
        Some(read(conn.query_row(
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
        ))?)
    };

    if json {
        // The same numbers the human output prints, and no others: two accounts
        // of one store that could disagree is the failure a shared shape exists
        // to prevent.
        Document::new("status")
            .field("store", store.path().display().to_string())
            .field("size_bytes", size_bytes(&data_dir))
            .field("sessions", counts.sessions)
            .field("turns", counts.turns)
            .field("watermarks", counts.watermarks)
            .field("watermark_bytes", counts.watermark_bytes)
            .field("excluded", config.exclusions())
            .field(
                "last_run",
                match &last {
                    None => serde_json::Value::Null,
                    Some(run) => json!({
                        "started_at": run.started_at,
                        "duration_ms": run.duration_ms,
                        "files_seen": run.files_seen,
                        "files_committed": run.files_committed,
                        "files_failed": run.files_failed,
                        "bytes_read": run.bytes_read,
                        "turns_added": run.turns_added,
                        "error": run.error,
                    }),
                },
            )
            .emit();
        return Ok(());
    }

    println!("store          {}", store.path().display());
    println!("size           {}", size(&data_dir));
    println!("sessions       {}", counts.sessions);
    println!("turns          {}", counts.turns);
    println!(
        "watermarks     {} covering {} byte(s)",
        counts.watermarks, counts.watermark_bytes
    );
    if !config.exclusions().is_empty() {
        // Named, because a count that dropped without explanation is a bug
        // report. The excluded sessions are still archived; they are not listed.
        println!(
            "excluded       {} project(s): {}",
            config.exclusions().len(),
            config.exclusions().join(", ")
        );
    }

    let Some(run) = last else {
        // An empty store is not a failure: it is what a machine looks like
        // before the first hook has ever fired.
        println!("last run       none");
        return Ok(());
    };

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
    let total = size_bytes(data_dir);
    format!("{total} byte(s) ({})", human(total))
}

/// The same footprint as a number, which is what `--json` carries: a consumer
/// that wants "3.2 MiB" can render it, and one that wants to compare two runs
/// cannot parse it back out of prose.
fn size_bytes(data_dir: &Path) -> u64 {
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
    total
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
