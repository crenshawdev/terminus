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
//! **It reads no field of the payload, and the read is bounded twice.**
//! Deserializing a shape phase 5 owns would be inventing it a phase early. The
//! line is read and dropped; reading it at all is only what keeps the writer
//! from seeing a closed pipe, and because the ingest is already running by then
//! (below) not one byte of it is on a data path. A courtesy is bounded like
//! one: at most [`MAX_PAYLOAD`] bytes, so a line that never ends cannot grow a
//! buffer without limit, and at most [`DRAIN_DEADLINE`] of waiting, because the
//! reading happens on a thread this command starts and never joins. A writer
//! that opens stdin and then neither writes a newline nor closes it gets the
//! one line on stderr the paragraph above promises, rather than holding the
//! hook until the harness's own timeout kills it - which on `UserPromptSubmit`
//! is the user's prompt waiting on us.
//!
//! **It spawns before it reads.** A harness that writes the payload and never
//! closes stdin would otherwise be able to stop the ingest from ever starting,
//! and the ordering is the only thing that rules it out. All four events spawn
//! the same argument-free tree pass (D-18): the ingest lock collapses
//! concurrent spawns into one pass and the losers exit 0 having written
//! nothing, so nothing here needs to know that another hook just fired.

use std::io::{BufRead, Read};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

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

/// The most stdin this command will ever hold: 1 MiB, against a real payload
/// measured in kilobytes even when it carries a whole compact summary.
///
/// The cap is on the read, not on the writer, so a line that never ends is a
/// megabyte read and dropped rather than a `Vec` that grows until the machine
/// is out of memory. Nothing downstream reads these bytes, so being one byte
/// short of a payload costs nothing at all.
const MAX_PAYLOAD: u64 = 1 << 20;

/// The longest the hook waits on a writer before it stops caring.
///
/// Orders of magnitude above any local pipe write - Claude Code 2.1.231 writes
/// the payload and calls `stdin.end()` in the same tick, so the ordinary drain
/// finishes in microseconds and never comes near this - and far below the
/// harness timeout that would otherwise be the thing that ends the wait, by
/// killing the hook.
const DRAIN_DEADLINE: Duration = Duration::from_millis(500);

pub fn run(event: &str) -> Result<(), Failure> {
    // First, and before stdin is touched.
    if let Err(e) = spawn::detached(&["ingest"]) {
        eprintln!("verbatim: {event} could not start an ingest: {e}");
    }

    drain(event);

    Ok(())
}

/// Read the one line the harness owes us (D-14) and drop it, but never let the
/// harness decide how long that takes or how much of it there is.
///
/// The read runs on a thread that is deliberately not joined: a blocking read
/// on a pipe nobody closes cannot be cancelled with `std` alone (D-04), so the
/// only way to stop an idle writer from holding this process is to stop waiting
/// on the reader and let process exit take the thread with it. That is safe
/// precisely because the bytes are dropped - there is no half-finished work to
/// abandon, and the ingest was started before any of this.
fn drain(event: &str) {
    let (done, drained) = mpsc::sync_channel(1);
    // Bytes rather than a `String`: a payload that is not UTF-8 is still a
    // payload that has been drained, and an encoding error here would be a
    // diagnostic about something nothing reads.
    let reader = std::thread::Builder::new()
        .name("verbatim-hook-stdin".to_string())
        .spawn(move || {
            let mut payload = Vec::new();
            let outcome = std::io::stdin()
                .lock()
                .take(MAX_PAYLOAD)
                .read_until(b'\n', &mut payload);
            // The main thread may have given up and gone; a send with no
            // receiver left is the timeout case reporting itself.
            let _ = done.send(outcome.err());
        });
    if let Err(e) = reader {
        eprintln!("verbatim: {event} could not read its payload: {e}");
        return;
    }

    match drained.recv_timeout(DRAIN_DEADLINE) {
        Ok(None) => {}
        Ok(Some(e)) => eprintln!("verbatim: {event} could not read its payload: {e}"),
        Err(RecvTimeoutError::Timeout) => eprintln!(
            "verbatim: {event} stopped waiting on a stdin that stayed open for {} ms",
            DRAIN_DEADLINE.as_millis()
        ),
        Err(RecvTimeoutError::Disconnected) => {
            eprintln!("verbatim: {event} lost the thread reading its payload")
        }
    }
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
