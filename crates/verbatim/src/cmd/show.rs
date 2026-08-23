//! `verbatim show`: print what a turn actually said (RCL-09, RCL-08).
//!
//! This is the terminal's `recall_get` plus `recall_context`. A search hands
//! back an excerpt - a projection, windowed around the match - and this hands
//! back the record's own archived bytes, optionally with the conversation around
//! it. Following a hit into its context is what a reader does next, which is why
//! the two live in one command rather than two.
//!
//! **The bytes are the record's, not a rendering of it.** They go to stdout
//! through `write_all` rather than through `println!("{}")`, because the archive
//! is verbatim and a lossy UTF-8 conversion here would make this - the one
//! command whose whole job is fidelity - the one place a byte could change. The
//! `--json` mode has no such option, since a JSON string is text by definition,
//! and it says so where it converts.
//!
//! **The window stops at the session (D-06)** and says which end it stopped at,
//! rather than just returning a shorter list. A caller that cannot tell "there
//! is nothing earlier" from "you asked for five and got two" cannot decide
//! whether to look in the session this one continues from - which is returned,
//! unfollowed, for exactly that decision.

use std::io::Write;

use verbatim_core::recall::{context, get, ContextTurn, Window};

use super::json::Document;
use super::read::{self, Opened};
use super::Failure;

const COMMAND: &str = "show";

pub fn run(args: Args) -> Result<(), Failure> {
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        Opened::Nothing(reason) => return read::empty(document(&[], &[]), &reason, args.json),
    };
    let conn = reader.store().conn();
    let config = reader.config();
    let scope = read::scope(args.project.as_deref())?;

    let fetched = get::records(conn, config, &scope, &args.ids)?;

    // Only when asked for. A window costs a decompressed session per anchor, and
    // `show` with no `--before`/`--after` is the command that answers "what did
    // this one turn say".
    let windows: Vec<Option<Window>> = if args.before == 0 && args.after == 0 {
        fetched.records.iter().map(|_| None).collect()
    } else {
        let mut out = Vec::with_capacity(fetched.records.len());
        for record in &fetched.records {
            out.push(Some(context::window(
                conn,
                config,
                &scope,
                record.turn_id,
                args.before,
                args.after,
            )?));
        }
        out
    };

    if args.json {
        emit(&fetched, &windows);
        return Ok(());
    }

    render(&fetched, &windows)?;

    for absent in &fetched.absent {
        eprintln!("verbatim: {}", absent.reason);
    }
    Ok(())
}

/// The human rendering: a header a reader can scan, then the bytes themselves.
fn render(fetched: &get::Fetched, windows: &[Option<Window>]) -> Result<(), Failure> {
    let mut out = std::io::stdout().lock();
    for (index, record) in fetched.records.iter().enumerate() {
        let header = format!(
            "{}  {}  {}  {}{}",
            record.turn_id,
            record.ts.as_deref().unwrap_or("(no time)"),
            file_name(&record.session_key),
            record.project.as_deref().unwrap_or("(no project)"),
            if record.body_evicted {
                "  (body evicted)"
            } else {
                ""
            },
        );
        writeln!(out, "{header}").map_err(broken_pipe)?;

        match &record.body {
            // Written as bytes: this is the command that answers "what exactly
            // was said", and a lossy conversion would answer something else.
            Some(bytes) => {
                out.write_all(bytes).map_err(broken_pipe)?;
                writeln!(out).map_err(broken_pipe)?;
            }
            // Evicted is said in the header; a body that would not read at all
            // is archive damage, which `verbatim verify` is the command for.
            None if !record.body_evicted => {
                writeln!(
                    out,
                    "(the archive would not give this record up; run `verbatim verify`)"
                )
                .map_err(broken_pipe)?;
            }
            None => {}
        }

        if let Some(window) = windows.get(index).and_then(Option::as_ref) {
            render_window(&mut out, window)?;
        }
    }
    Ok(())
}

/// The conversation around one turn, in `turn_seq` order with its ends marked.
fn render_window(out: &mut impl Write, window: &Window) -> Result<(), Failure> {
    if let Some(reason) = &window.reason {
        writeln!(out, "  (no context: {reason})").map_err(broken_pipe)?;
        return Ok(());
    }
    if window.at_session_start {
        writeln!(out, "  --- start of session ---").map_err(broken_pipe)?;
    }
    for turn in &window.turns {
        writeln!(
            out,
            "  {} {}  {}  {}",
            if turn.is_anchor { '>' } else { ' ' },
            turn.turn_seq,
            label(turn),
            turn.text
        )
        .map_err(broken_pipe)?;
    }
    if window.at_session_end {
        writeln!(out, "  --- end of session ---").map_err(broken_pipe)?;
    }
    // Returned and NOT followed (D-06). A caller that wants the turns before
    // this file's first one resolves this itself and runs a second `show`.
    if let Some(previous) = &window.continues_from {
        writeln!(out, "  (this session continues from {previous})").map_err(broken_pipe)?;
    }
    Ok(())
}

/// One context turn's kind, with its tool when it has one.
fn label(turn: &ContextTurn) -> String {
    match &turn.tool_name {
        Some(tool) => format!("{}/{tool}", turn.record_type),
        None => turn.record_type.clone(),
    }
}

/// A closed stdout is not a failure of the archive.
///
/// `verbatim show <id> | head` closes the pipe on the first page, and a command
/// that reported that as an operational failure would make the ordinary way of
/// reading a long record look like a broken store.
fn broken_pipe(error: std::io::Error) -> Failure {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        Failure::Silent
    } else {
        Failure::Operational(error.to_string())
    }
}

fn emit(fetched: &get::Fetched, windows: &[Option<Window>]) {
    let records: Vec<serde_json::Value> = fetched
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            serde_json::json!({
                "turn_id": record.turn_id,
                "session_key": record.session_key,
                "turn_seq": record.turn_seq,
                "record_type": record.record_type,
                "tool_name": record.tool_name,
                "ts": record.ts,
                "project": record.project,
                // Lossy, and only here: a JSON string is text by definition, so
                // this is the one rendering that cannot carry a byte that is not
                // UTF-8. The human mode writes the bytes themselves.
                "body": record.body.as_ref().map(|b| String::from_utf8_lossy(b).into_owned()),
                "body_evicted": record.body_evicted,
                "context": windows.get(index).and_then(Option::as_ref).map(window_value),
            })
        })
        .collect();

    let absent: Vec<serde_json::Value> = fetched
        .absent
        .iter()
        .map(|absent| {
            serde_json::json!({"turn_id": absent.turn_id, "reason": absent.reason.to_string()})
        })
        .collect();

    let mut document = document(&records, &absent);
    // The request-level reason first, then the one that explains an answer with
    // no records in it at all.
    if let Some(reason) = &fetched.reason {
        document = document.because(reason);
    } else if fetched.records.is_empty() {
        if let Some(first) = fetched.absent.first() {
            document = document.because(&first.reason);
        }
    }
    document.emit();
}

fn window_value(window: &Window) -> serde_json::Value {
    serde_json::json!({
        "turns": window.turns.iter().map(|turn| serde_json::json!({
            "turn_id": turn.turn_id,
            "turn_seq": turn.turn_seq,
            "record_type": turn.record_type,
            "tool_name": turn.tool_name,
            "ts": turn.ts,
            "text": turn.text,
            "is_anchor": turn.is_anchor,
        })).collect::<Vec<_>>(),
        // Said out loud rather than left to be inferred from a short list
        // (D-06): "there is nothing earlier" and "you asked for five and got
        // two" are different answers.
        "at_session_start": window.at_session_start,
        "at_session_end": window.at_session_end,
        "continues_from": window.continues_from,
        "reason": window.reason.as_ref().map(ToString::to_string),
    })
}

fn document(records: &[serde_json::Value], absent: &[serde_json::Value]) -> Document {
    Document::new(COMMAND)
        .field("records", records.to_vec())
        .field("absent", absent.to_vec())
}

/// The transcript's own file name, which is what a session key ends with.
fn file_name(session_key: &str) -> &str {
    session_key
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(session_key)
}

/// The parsed command line for `show`.
#[derive(Debug)]
pub struct Args {
    pub ids: Vec<i64>,
    pub project: Option<String>,
    pub before: usize,
    pub after: usize,
    pub json: bool,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut ids: Vec<i64> = Vec::new();
    let mut project = None;
    let mut before = 0;
    let mut after = 0;
    let mut json = false;

    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long("project") => project = Some(super::value(parser, "project")?),
            Long("before") => before = count(parser, "before")?,
            Long("after") => after = count(parser, "after")?,
            Long(super::JSON_FLAG) => json = true,
            Value(word) => {
                let raw = word.to_string_lossy().into_owned();
                // Misuse, not an empty result: a word where an id belongs is a
                // mistake in the command line, and answering "no such turn"
                // would suggest the archive had been consulted about it.
                let id = raw.parse::<i64>().map_err(|_| {
                    Failure::Misuse(format!(
                        "{raw:?} is not a turn id; ids are the numbers `verbatim search` prints"
                    ))
                })?;
                ids.push(id);
            }
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    if ids.is_empty() {
        return Err(Failure::Misuse(
            "show needs at least one turn id; `verbatim search` prints them".into(),
        ));
    }

    Ok(Args {
        ids,
        project,
        before,
        after,
        json,
    })
}

/// A count flag, clamped to the window the query layer will apply anyway.
fn count(parser: &mut lexopt::Parser, flag: &str) -> Result<usize, Failure> {
    let raw = super::value(parser, flag)?;
    let value = raw
        .parse::<usize>()
        .map_err(|_| Failure::Misuse(format!("--{flag} {raw:?} is not a number")))?;
    Ok(value.min(verbatim_core::recall::MAX_CONTEXT_SIDE))
}
