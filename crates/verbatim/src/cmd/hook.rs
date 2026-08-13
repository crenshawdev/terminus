//! `verbatim hook <event>`: the entry point Claude Code calls.
//!
//! Four events, one behaviour: start the ingest, read the payload, say nothing,
//! exit 0. Everything that makes this command interesting is a negative.
//!
//! **It writes nothing to stdout, on any path (D-15).** Claude Code 2.1.231
//! validates any hook stdout that parses as JSON against the event name, so a
//! stray line is not ignored, it is a protocol error. The resume brief and the
//! prompt injection that will one day be written here are phase 5's
//! (INJ-01..INJ-06) and are deliberately absent.
//!
//! **It exits 0 once the event name is known, whatever happens next.** A
//! `UserPromptSubmit` hook that exits non-zero can block the prompt the user
//! just typed, and no memory system is worth that. A stdin that never closes, a
//! payload that is not JSON, a spawn that failed - each is one line on stderr
//! and still exit 0. The one exception is upstream of all of it: an event name
//! this binary does not answer to is [`Failure::Misuse`] (exit 2), because
//! `verbatim install` writes these entries and an unknown one means a broken
//! settings file rather than a routine event.
//!
//! **It reads no field of the payload.** Deserializing a shape phase 5 owns
//! would be inventing it a phase early. The line is read and dropped; reading
//! it at all is what keeps the writer from seeing a closed pipe.
//!
//! **It spawns before it reads.** A harness that writes the payload and never
//! closes stdin would otherwise be able to stop the ingest from ever starting,
//! and the ordering is the only thing that rules it out. All four events spawn
//! the same argument-free tree pass (D-18): the ingest lock collapses
//! concurrent spawns into one pass and the losers exit 0 having written
//! nothing, so nothing here needs to know that another hook just fired.

use std::io::BufRead;

use super::{spawn, Failure};

/// The events this binary answers to, and therefore the events
/// `verbatim install` may write into `settings.json`.
///
/// One list, named here and read there, so the settings file cannot come to
/// hold an event the binary rejects.
///
/// `PostCompact`, never `PreCompact` (D-02). Compaction is append-only and the
/// `system`/`compact_boundary` record is appended *after* it, so an ingest
/// spawned before compaction can never see the boundary it exists to record.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "SessionEnd",
    "PostCompact",
];

pub fn run(event: &str) -> Result<(), Failure> {
    // First, and before stdin is touched.
    if let Err(e) = spawn::detached(&["ingest"]) {
        eprintln!("verbatim: {event} could not start an ingest: {e}");
    }

    // One line, then EOF (D-14). Read as bytes rather than as a `String`: a
    // payload that is not UTF-8 is still a payload that has been drained, and
    // an encoding error here would be a diagnostic about something nothing
    // reads.
    let mut payload = Vec::new();
    if let Err(e) = std::io::stdin().lock().read_until(b'\n', &mut payload) {
        eprintln!("verbatim: {event} could not read its payload: {e}");
    }

    Ok(())
}

/// The one positional event name, matched against [`EVENTS`].
pub fn parse(parser: &mut lexopt::Parser) -> Result<&'static str, Failure> {
    use lexopt::prelude::*;

    let mut event: Option<&'static str> = None;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Value(v) if event.is_none() => {
                let name = v.to_string_lossy();
                event = Some(known(&name)?);
            }
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    event.ok_or_else(|| Failure::Misuse(format!("hook needs an event: {}", EVENTS.join(", "))))
}

fn known(name: &str) -> Result<&'static str, Failure> {
    EVENTS
        .iter()
        .copied()
        .find(|event| *event == name)
        .ok_or_else(|| {
            Failure::Misuse(format!(
                "unknown hook event '{name}'; expected one of {}",
                EVENTS.join(", ")
            ))
        })
}
