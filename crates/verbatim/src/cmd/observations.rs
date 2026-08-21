//! `verbatim observations`: what happened in every session that has closed
//! (OBS-01).
//!
//! **A read command, in the shape of `stats`.** It opens through
//! [`super::read::open`], so a machine that has never ingested gets an empty
//! answer with a reason and exit 0 rather than a store created as the side
//! effect of a question (D-15). Nothing here calls a model or opens a network
//! connection: the row was written by the ingest pass off the parser, and this
//! command reads it back.
//!
//! **Listed through `config::visible`, never straight off the table.**
//! Exclusion is retroactive (ING-08) and the observations table is keyed on a
//! session, so a read path that selected from `observations` alone would list a
//! session whose project was excluded after it was archived - which is exactly
//! the half claude-mem gets wrong. The visible listing is the outer loop here
//! and the table is looked up inside it, so a session the gate does not return
//! cannot appear whatever the table holds.
//!
//! **A store older than the table is an answer, not a SQLite line.** Left to
//! the query, a missing table surfaces as `no such table: observations` inside
//! an operational failure that never learns `--json` was asked for, so the
//! envelope the contract promises is never written at all.
//! [`verbatim_core::Store::missing_tables`] is asked first, the way
//! `read::decision_log` asks it.

use std::collections::BTreeMap;

use serde_json::Value;
use verbatim_core::config::visible;
use verbatim_core::recall::scope;

use super::json::Document;
use super::read::{self, Opened};
use super::Failure;

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "observations";

/// What a store written before this build says instead of naming SQLite.
///
/// It names the table, because the difference between "nothing has been
/// observed" and "this build's table is not here yet" is the difference between
/// waiting for the next ingest and filing a bug.
const NO_TABLE: &str = "this store predates the `observations` table, so nothing has been \
                        observed yet; the next `verbatim ingest` creates it";

pub fn run(args: Args) -> Result<(), Failure> {
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        Opened::Nothing(reason) => return read::empty(document(&[]), &reason, args.json),
    };
    let conn = reader.store().conn();
    let config = reader.config();

    if reader.store().missing_tables().contains(&"observations") {
        return read::empty(document(&[]), NO_TABLE, args.json);
    }

    let scoped = scope::resolve(conn, config, &read::scope(args.project.as_deref())?)?;
    if let Some(reason) = scoped.reason() {
        return read::empty(document(&[]), &reason.to_string(), args.json);
    }

    // `o.decisions` and the `decisions` TABLE are different objects with the
    // same name, so the column is qualified here and everywhere else it is
    // named.
    let mut stored: BTreeMap<String, Row> = {
        let mut statement = op(conn.prepare(
            "SELECT o.session_key, o.session_id, o.generated_at, o.mechanical,
                    o.status, o.model, o.prompt_version, o.topic, o.outcome,
                    o.decisions, o.learned, o.unresolved, o.raw, o.tokens
               FROM observations o",
        ))?;
        let rows = op(statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Row {
                    session_key: r.get(0)?,
                    session_id: r.get(1)?,
                    generated_at: r.get(2)?,
                    mechanical: r.get(3)?,
                    status: r.get(4)?,
                    model: r.get(5)?,
                    prompt_version: r.get(6)?,
                    topic: r.get(7)?,
                    outcome: r.get(8)?,
                    decisions: r.get(9)?,
                    learned: r.get(10)?,
                    unresolved: r.get(11)?,
                    raw: r.get(12)?,
                    tokens: r.get(13)?,
                },
            ))
        }))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (key, row) = op(row)?;
            out.insert(key, row);
        }
        out
    };

    // The visible listing is the outer loop: ingest order, exclusion already
    // applied, and a row whose session the gate did not return is never reached.
    let mut listed: Vec<Row> = Vec::new();
    for session in visible::sessions(conn, config)? {
        if let Some(wanted) = scoped.project() {
            if session.project.as_deref() != Some(wanted) {
                continue;
            }
        }
        if let Some(row) = stored.remove(&session.session_key) {
            listed.push(row);
        }
    }

    if args.json {
        document(&listed.iter().map(Row::to_value).collect::<Vec<_>>()).emit();
        return Ok(());
    }

    for row in &listed {
        row.print();
    }
    // Commentary on stderr the way `sessions` reports its own, so stdout stays
    // the answer.
    eprintln!("{} observation(s)", listed.len());
    Ok(())
}

/// One row of the table, every column of it.
///
/// The judgment columns are carried even though PLAN-3 is what fills them: they
/// are part of the row and a consumer reads one shape whether or not a provider
/// has ever answered, the way the envelope carries `reason` as a null rather
/// than as a missing key.
struct Row {
    session_key: String,
    session_id: Option<String>,
    generated_at: Option<String>,
    mechanical: Option<String>,
    status: Option<String>,
    model: Option<String>,
    prompt_version: Option<String>,
    topic: Option<String>,
    outcome: Option<String>,
    decisions: Option<String>,
    learned: Option<String>,
    unresolved: Option<String>,
    raw: Option<String>,
    tokens: Option<i64>,
}

impl Row {
    fn to_value(&self) -> Value {
        serde_json::json!({
            "session_key": self.session_key,
            "session_id": self.session_id,
            "generated_at": self.generated_at,
            "mechanical": nested(self.mechanical.as_deref()),
            "status": self.status,
            "model": self.model,
            "prompt_version": self.prompt_version,
            "topic": self.topic,
            "outcome": self.outcome,
            "decisions": nested(self.decisions.as_deref()),
            "learned": nested(self.learned.as_deref()),
            "unresolved": nested(self.unresolved.as_deref()),
            "raw": self.raw,
            "tokens": self.tokens,
        })
    }

    /// The facts as a person reads them: a header line, then one line per fact
    /// list that has anything in it.
    ///
    /// Empty lists are skipped rather than printed as a dash, because a session
    /// that ran no command and hit no error would otherwise be six lines of
    /// nothing between the two sessions worth reading.
    fn print(&self) {
        let facts = self
            .mechanical
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or(Value::Null);

        println!(
            "{}  {}  {} turn(s)  {}  {} compaction(s)",
            file_name(&self.session_key),
            text(&facts["branch"]).unwrap_or("(no branch)"),
            number(&facts["turns"]),
            duration(facts["duration_seconds"].as_i64()),
            number(&facts["compactions"]),
        );
        for (label, field) in [
            ("files modified", "files_modified"),
            ("files read", "files_read"),
            ("tools", "tools"),
            ("commands", "commands"),
            ("errors", "errors"),
            ("commits", "commits"),
        ] {
            let values = strings(&facts[field]);
            if !values.is_empty() {
                println!("  {label:<15} {}", values.join(", "));
            }
        }
        // What the row itself says was cut, so a list a cap shortened is never
        // read as the whole of what the session did.
        let cut = strings(&facts["truncated"]);
        if !cut.is_empty() {
            println!("  {:<15} {}", "truncated", cut.join(", "));
        }
    }
}

/// A stored JSON column as a value, not as a string.
///
/// `mechanical` and the three claim lists are documents in a TEXT column, and a
/// consumer that got them as strings would have to parse the document out of
/// the document. A column that will not parse comes back as the raw string
/// rather than as a null: a reader can tell an object from a string, and the
/// bytes that are actually there are more use than a silent absence.
fn nested(stored: Option<&str>) -> Value {
    match stored {
        None => Value::Null,
        Some(text) => serde_json::from_str(text).unwrap_or_else(|_| Value::from(text)),
    }
}

/// A store error is operational (exit 1), never misuse.
fn op<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> Result<T, Failure> {
    result.map_err(|e| Failure::Operational(e.to_string()))
}

fn text(value: &Value) -> Option<&str> {
    value.as_str()
}

fn number(value: &Value) -> i64 {
    value.as_i64().unwrap_or(0)
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Wall-clock seconds as a person reads a duration.
fn duration(seconds: Option<i64>) -> String {
    match seconds {
        None => "(no duration)".to_owned(),
        Some(total) if total < 60 => format!("{total}s"),
        Some(total) if total < 3600 => format!("{}m {}s", total / 60, total % 60),
        Some(total) => format!("{}h {}m", total / 3600, (total % 3600) / 60),
    }
}

fn file_name(session_key: &str) -> &str {
    session_key
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(session_key)
}

fn document(observations: &[Value]) -> Document {
    Document::new(COMMAND).field("observations", observations.to_vec())
}

/// The parsed command line for `observations`.
#[derive(Debug)]
pub struct Args {
    pub project: Option<String>,
    pub json: bool,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut project = None;
    let mut json = false;

    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long("project") => project = Some(super::value(parser, "project")?),
            Long(super::JSON_FLAG) => json = true,
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    Ok(Args { project, json })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column holding a document comes back as a document, and one holding
    /// bytes that are not JSON comes back as those bytes rather than as a null.
    #[test]
    fn a_stored_document_column_is_a_value_and_never_a_string_of_json() {
        assert_eq!(nested(None), Value::Null);
        assert_eq!(nested(Some(r#"{"turns":3}"#))["turns"], Value::from(3));
        assert_eq!(nested(Some("[1,2]")), serde_json::json!([1, 2]));
        assert_eq!(
            nested(Some("not json at all")),
            Value::from("not json at all")
        );
    }

    #[test]
    fn a_duration_reads_as_a_duration() {
        assert_eq!(duration(None), "(no duration)");
        assert_eq!(duration(Some(2)), "2s");
        assert_eq!(duration(Some(201)), "3m 21s");
        assert_eq!(duration(Some(7_400)), "2h 3m");
    }
}
