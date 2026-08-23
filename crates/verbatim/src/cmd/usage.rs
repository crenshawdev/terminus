//! `verbatim usage`: where the bytes went (RET-05).
//!
//! **Two independent tables, and they are never mixed (D-09).**
//!
//! The ARCHIVE table is `length(sessions.blob)` by project and, separately, by
//! month. Its totals reconcile to `sum(length(sessions.blob))` and to nothing
//! else - never to the size of the file on disk, which on the live store
//! measured 2026-08-22 is 1,103,396,864 bytes against 465,985,120 bytes of
//! archived blob, 2.4x everything the projects account for. The rest is the
//! derived tables, and attributing them pro-rata would make every per-project
//! number an estimate: a user comparing two projects would be comparing a model
//! rather than a measurement.
//!
//! The FOOTPRINT table is where that other 57% actually is, per table, out of
//! SQLite's own `dbstat`. It reconciles to the database file's size and to
//! nothing else. Two tables that each reconcile to their own total is the whole
//! design; one table that reconciled to neither is what it replaces.
//!
//! **`dbstat`'s aggregate form.** `dbstat(<schema>, 1)` returns one row per
//! btree instead of one per page. Measured 2026-08-22 against the live
//! 1,114,308,608-byte store: 0.144 s wall for the whole grouped answer, which
//! is a command a person typed and not the hook path. The per-page form walks
//! 272,048 rows to compute the same numbers.
//!
//! It opens through [`super::read`] because it is a read: a question about how
//! big the store is must not create one, and every per-project number goes
//! through `config::visible` for the same reason every other read does -
//! exclusion is retroactive (ING-08), so a project excluded after its sessions
//! were archived must not appear in a report of what is stored.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use verbatim_core::config::visible;

use super::json::Document;
use super::read::{self, Opened, Reader};
use super::{human, Failure};

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "usage";

pub fn run(json: bool) -> Result<(), Failure> {
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        Opened::Nothing(reason) => return read::empty(document(&Usage::default()), &reason, json),
    };
    let usage = measure(&reader).map_err(op)?;

    if json {
        document(&usage).emit();
        return Ok(());
    }

    // Two tables, printed as two, with the total each one reconciles to at its
    // foot. The totals are the point of the report and a reader who takes only
    // the last line of each block has the answer.
    println!("archive by project");
    for bucket in &usage.by_project {
        println!(
            "  {:>14}  {:>5} session(s)  {}",
            human(bucket.bytes as u64),
            bucket.sessions,
            bucket.name()
        );
    }
    println!(
        "  {:>14}  {:>5} session(s)  total archived",
        human(usage.archive_bytes as u64),
        usage.sessions
    );
    println!();
    println!("archive by month");
    for bucket in &usage.by_month {
        println!(
            "  {:>14}  {:>5} session(s)  {}",
            human(bucket.bytes as u64),
            bucket.sessions,
            bucket.name()
        );
    }
    println!(
        "  {:>14}  {:>5} session(s)  total archived",
        human(usage.archive_bytes as u64),
        usage.sessions
    );
    println!();
    println!("store footprint by table");
    for row in &usage.footprint {
        println!("  {:>14}  {}", human(row.bytes as u64), row.name);
    }
    println!(
        "  {:>14}  the whole database file",
        human(usage.file_bytes as u64)
    );
    Ok(())
}

/// One row of either archive table.
///
/// `name` is an `Option` and not a sentinel string, because a bucket for
/// sessions with no project has to be distinguishable from a project that is
/// somehow named "(none)". It is a row either way - counted, carried and inside
/// the total - which is the property that matters: a null silently dropped is a
/// total that no longer reconciles, and the live store has one session with no
/// `project` and one with no `first_turn_at`.
#[derive(Debug, Clone, Default)]
struct Bucket {
    key: Option<String>,
    sessions: i64,
    bytes: i64,
}

impl Bucket {
    /// What the human table prints for this bucket.
    fn name(&self) -> &str {
        self.key.as_deref().unwrap_or("(none recorded)")
    }
}

/// One row of the footprint table.
#[derive(Debug, Clone)]
struct TableBytes {
    name: String,
    bytes: i64,
}

#[derive(Debug, Clone, Default)]
struct Usage {
    sessions: i64,
    archive_bytes: i64,
    by_project: Vec<Bucket>,
    by_month: Vec<Bucket>,
    file_bytes: i64,
    footprint: Vec<TableBytes>,
}

/// `rusqlite` is deliberately not named in this crate (see `cmd::status`), so
/// the connection travels as the [`Reader`] it came out of rather than as a
/// `&Connection` this signature would have to spell.
fn measure(reader: &Reader) -> verbatim_core::Result<Usage> {
    let conn = reader.store().conn();
    let config = reader.config();
    // The visibility gate first, and it is the only place exclusion is decided.
    // A second `WHERE project NOT IN (...)` written here would be a second
    // definition of what a read may see, which is the drift `config::visible`
    // exists to prevent.
    let visible = visible::sessions(conn, config)?;

    // The bytes and the month for every session in the store, in one pass.
    // Keyed on `session_key` so the visible set decides which of them count.
    let mut rows: BTreeMap<String, (Option<String>, i64)> = BTreeMap::new();
    let mut statement = conn.prepare(
        "SELECT s.session_key, substr(m.first_turn_at, 1, 7), length(s.blob)
           FROM sessions s LEFT JOIN session_meta m USING (session_key)",
    )?;
    let mut cursor = statement.query([])?;
    while let Some(row) = cursor.next()? {
        rows.insert(row.get(0)?, (row.get(1)?, row.get(2)?));
    }

    let mut usage = Usage::default();
    let mut by_project: BTreeMap<Option<String>, Bucket> = BTreeMap::new();
    let mut by_month: BTreeMap<Option<String>, Bucket> = BTreeMap::new();
    for session in &visible {
        // A session with a row in `sessions` always has a length; the default
        // covers only the impossible case of a key that vanished between the
        // two statements, and counts it as a session with no bytes rather than
        // dropping it out of the session count.
        let (month, bytes) = rows.get(&session.session_key).cloned().unwrap_or((None, 0));
        usage.sessions += 1;
        usage.archive_bytes += bytes;
        for (buckets, key) in [
            (&mut by_project, session.project.clone()),
            (&mut by_month, month),
        ] {
            let bucket = buckets.entry(key.clone()).or_insert_with(|| Bucket {
                key,
                ..Bucket::default()
            });
            bucket.sessions += 1;
            bucket.bytes += bytes;
        }
    }
    // Biggest first, which is the order the question is asked in. The unnamed
    // bucket sorts last whatever its size: it is the residue, not an answer.
    usage.by_project = ordered(by_project);
    usage.by_month = ordered(by_month);

    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let page_count: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let freelist: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    usage.file_bytes = page_count * page_size;

    let mut statement =
        conn.prepare("SELECT name, pgsize FROM dbstat('main', 1) ORDER BY pgsize DESC")?;
    let mut cursor = statement.query([])?;
    while let Some(row) = cursor.next()? {
        usage.footprint.push(TableBytes {
            name: row.get(0)?,
            bytes: row.get(1)?,
        });
    }
    // The freelist is a row and not a remainder: it is the space `VACUUM` will
    // reclaim, so it is the number that makes `verbatim compact` legible before
    // it is run.
    usage.footprint.push(TableBytes {
        name: "(free pages)".to_owned(),
        bytes: freelist * page_size,
    });
    // And the one page that belongs to no btree and no freelist. SQLite never
    // uses the page containing byte 0x40000000, so a database that has grown
    // past 1 GiB has exactly one page `dbstat` cannot see. Measured 2026-08-22
    // against the live 1,114,308,608-byte store: `sum(pgsize)` plus the
    // freelist came to 1,114,304,512, short by 4,096 - one page. Without this
    // row the reconciliation this report documents holds on every small store
    // and silently stops holding on the ones big enough to care.
    let lock_page = 0x4000_0000 / page_size + 1;
    if page_count >= lock_page {
        usage.footprint.push(TableBytes {
            name: "(lock page)".to_owned(),
            bytes: page_size,
        });
    }
    Ok(usage)
}

/// Buckets biggest first, with the unnamed one last.
fn ordered(buckets: BTreeMap<Option<String>, Bucket>) -> Vec<Bucket> {
    let mut out: Vec<Bucket> = buckets.into_values().collect();
    out.sort_by(|a, b| {
        a.key
            .is_none()
            .cmp(&b.key.is_none())
            .then(b.bytes.cmp(&a.bytes))
            .then(a.key.cmp(&b.key))
    });
    out
}

fn document(usage: &Usage) -> Document {
    // Built as a map rather than through `json!` with a variable key, so the
    // one field whose NAME differs between the two tables - `project` against
    // `month` - is the only difference between them.
    let bucket = |field: &str, buckets: &[Bucket]| {
        buckets
            .iter()
            .map(|b| {
                let mut row = Map::new();
                row.insert(
                    field.to_owned(),
                    match &b.key {
                        Some(key) => Value::from(key.as_str()),
                        None => Value::Null,
                    },
                );
                row.insert("sessions".to_owned(), Value::from(b.sessions));
                row.insert("bytes".to_owned(), Value::from(b.bytes));
                Value::Object(row)
            })
            .collect::<Vec<_>>()
    };
    Document::new(COMMAND)
        .field("sessions", usage.sessions)
        .field("archive_bytes", usage.archive_bytes)
        .field("by_project", bucket("project", &usage.by_project))
        .field("by_month", bucket("month", &usage.by_month))
        .field("file_bytes", usage.file_bytes)
        .field(
            "footprint",
            usage
                .footprint
                .iter()
                .map(|row| json!({"name": row.name, "bytes": row.bytes}))
                .collect::<Vec<_>>(),
        )
}

/// A store error is operational (exit 1), never misuse.
fn op(error: verbatim_core::Error) -> Failure {
    Failure::Operational(error.to_string())
}
