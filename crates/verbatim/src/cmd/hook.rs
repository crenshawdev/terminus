//! `verbatim hook <event>`: the entry point Claude Code calls.
//!
//! Four events, one behaviour: start the ingest, read the payload, say at most
//! one thing, exit 0. Most of what makes this command interesting is still a
//! negative.
//!
//! **It writes one JSON object to stdout, or nothing, and never anything else
//! (D-01, D-15).** Claude Code validates any hook stdout that parses as JSON
//! against the event name, so a stray line is not ignored, it is a protocol
//! error - and a document that starts with `{` and fails schema validation is a
//! red `hook_non_blocking_error` banner shown to the user rather than silence.
//! The object carries `hookSpecificOutput.hookEventName` equal to the event
//! that fired and the injected text in `hookSpecificOutput.additionalContext`,
//! and it is built with `serde_json` and never with `format!` (the D-25 rule
//! `cmd::json` states), because injected text is arbitrary transcript bytes.
//! Only `SessionStart` and `UserPromptSubmit` have that variant in the
//! harness's union: `SessionEnd` and `PostCompact` write nothing on every path,
//! against every store. Text that is empty or whitespace is nothing too, not an
//! empty `additionalContext`.
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
//! **It reads five fields of the payload, and the read is still bounded
//! twice.** Phase 5 needs `session_id`, `transcript_path`, `cwd`, `prompt` and
//! `source` to inject anything, and D-02 says they arrive here or nowhere: the
//! argv stays `["hook", "<event>"]` and gains no flag, because
//! `cmd::install::targets` writes those entries once and never rewrites them
//! (INST-05), so a new argument would mean editing every user's `settings.json`
//! on upgrade. Every field is optional at the type level - a payload that omits
//! one is a payload that gets less injection, never an error - and the bounds
//! the read had when the bytes were dropped are the bounds it still has: at
//! most [`MAX_PAYLOAD`] bytes, so a line that never ends cannot grow a buffer
//! without limit, and at most [`DRAIN_DEADLINE`] of waiting, because the
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

use verbatim_core::inject;

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
/// megabyte read and discarded rather than a `Vec` that grows until the machine
/// is out of memory. A payload cut off at the cap stops being JSON, so it parses
/// to a [`Payload`] with every field absent - which is one uninjected event and
/// not a failure.
const MAX_PAYLOAD: u64 = 1 << 20;

/// The longest the hook waits on a writer before it stops caring.
///
/// Orders of magnitude above any local pipe write - Claude Code 2.1.231 writes
/// the payload and calls `stdin.end()` in the same tick, so the ordinary drain
/// finishes in microseconds and never comes near this - and far below the
/// harness timeout that would otherwise be the thing that ends the wait, by
/// killing the hook.
const DRAIN_DEADLINE: Duration = Duration::from_millis(500);

/// The longest the hook waits on its own injection before abandoning it and
/// writing nothing (INJ-06, D-03).
///
/// 50 ms is five times the 10 ms p99 wall budget
/// `crates/verbatim/tests/hook.rs` asserts over 100 runs of every event - so an
/// ordinary injection is never near it - and a hundredth of the five-second
/// wait `store::open`'s default busy handler would impose on a prompt submitted
/// while a backfill holds the store, which was measured at 49 s over the real
/// corpus. The number that matters is the one it is far below: the user is
/// waiting on this, and a memory system that costs a visible pause on every
/// prompt has already lost the argument for existing.
const INJECT_DEADLINE: Duration = Duration::from_millis(50);

/// The five fields of the hook payload injection reads.
///
/// Owned `String`s rather than borrows of the drained buffer, because the
/// buffer belongs to a thread this command may have stopped waiting on.
///
/// Read off a [`serde_json::Value`] rather than deserialized into a derived
/// struct: this crate depends on `serde_json` and deliberately not on `serde`'s
/// derive (`crates/verbatim/Cargo.toml`), and the hook path is the startup floor
/// the whole architecture is shaped around. Every field is an `Option` on
/// purpose - the harness sends a different set per event, and an absent field is
/// an event that gets less injection rather than an error.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Payload {
    /// The Claude Code session this event belongs to. Present on all four.
    pub session_id: Option<String>,
    /// The transcript file Claude Code is appending to, which is what ingest
    /// keys a session on once it is canonicalized.
    pub transcript_path: Option<String>,
    /// The working directory of the session - never this process's own, which
    /// on a hook is whatever directory the harness happened to choose (D-12).
    pub cwd: Option<String>,
    /// The text the user just submitted. `UserPromptSubmit` only.
    pub prompt: Option<String>,
    /// Why the session started. `SessionStart` only, one of
    /// `startup`, `resume`, `clear`, `compact`, `fork`.
    pub source: Option<String>,
}

impl Payload {
    /// The same five fields as the borrowed shape the library takes.
    fn borrowed(&self) -> inject::Payload<'_> {
        inject::Payload {
            session_id: self.session_id.as_deref(),
            transcript_path: self.transcript_path.as_deref(),
            cwd: self.cwd.as_deref(),
            prompt: self.prompt.as_deref(),
            source: self.source.as_deref(),
        }
    }

    /// The fields of one drained line, or an empty payload.
    ///
    /// Bytes that are not JSON, or JSON that is not an object, or an object
    /// whose fields are numbers instead of strings, all read as absent. This is
    /// the only parse on the hook path and it must not have a failing arm: a
    /// malformed payload is upstream's, and a hook that reported it would be
    /// spending the user's prompt on a diagnostic about a field nothing here
    /// requires.
    fn parse(line: &[u8]) -> Payload {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            return Payload::default();
        };
        let field = |name: &str| {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        Payload {
            session_id: field("session_id"),
            transcript_path: field("transcript_path"),
            cwd: field("cwd"),
            prompt: field("prompt"),
            source: field("source"),
        }
    }
}

pub fn run(event: &str) -> Result<(), Failure> {
    // First, and before stdin is touched.
    if let Err(e) = spawn::detached(&["ingest"]) {
        eprintln!("verbatim: {event} could not start an ingest: {e}");
    }

    let payload = drain(event);
    if let Some(text) = inject(event, payload) {
        emit(event, &text);
    }

    Ok(())
}

/// One event's injection arm, as the library declares it.
type Arm = fn(&std::path::Path, &verbatim_core::Config, &inject::Payload) -> Option<String>;

/// The text this event injects, or nothing, and never later than
/// [`INJECT_DEADLINE`].
///
/// Two of the four events have an `additionalContext` variant and the other two
/// return here without opening anything - a `SessionEnd` that read the store
/// would be paying for an answer with nowhere to go.
///
/// **The work runs on a thread this command starts and never joins**, the same
/// shape and the same reason as [`drain`]: a blocking SQLite call cannot be
/// cancelled with `std` alone, so the only way to stop waiting is to stop
/// waiting and let process exit take the thread with it. Abandoning it is safe
/// because nothing on that thread writes, creates a directory or takes the
/// ingest lock (INJ-06). This is the second of D-03's two mechanisms: the busy
/// timeout `verbatim_core::inject` sets turns lock contention into an immediate
/// `SQLITE_BUSY`, and this catches what a busy timeout cannot see - a slow
/// query, a large blob, a `verbatim.toml` on a filesystem that has stopped
/// answering.
///
/// **A panic is one silent event, not a wedged hook.** The unwind is caught and
/// becomes no injection; the default panic hook has already said what happened
/// on stderr, so nothing is added to it. `crates/verbatim-core`'s `977d0b4` is
/// the precedent - the same failure, one layer down.
///
/// Resolving the data directory and loading the config happen inside the thread
/// rather than ahead of it, so that they are inside the deadline too: a
/// `verbatim.toml` that does not parse is a reported error on `verbatim search`
/// and one uninjected event here.
fn inject(event: &str, payload: Payload) -> Option<String> {
    let arm: Arm = match event {
        "SessionStart" => inject::brief::session_start,
        "UserPromptSubmit" => inject::prompt::user_prompt_submit,
        _ => return None,
    };

    let (done, injected) = mpsc::sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("verbatim-hook-inject".to_string())
        .spawn(move || {
            let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let data_dir = super::data_dir().ok()?;
                let config = verbatim_core::Config::load().ok()?;
                arm(&data_dir, &config, &payload.borrowed())
            }))
            .ok()
            .flatten();
            // The main thread may have given up and gone; a send with no
            // receiver left is the timeout case reporting itself.
            let _ = done.send(text);
        });
    if let Err(e) = worker {
        eprintln!("verbatim: {event} could not start its injection: {e}");
        return None;
    }

    match injected.recv_timeout(INJECT_DEADLINE) {
        Ok(text) => text,
        Err(RecvTimeoutError::Timeout) => {
            eprintln!(
                "verbatim: {event} abandoned its injection after {} ms",
                INJECT_DEADLINE.as_millis()
            );
            None
        }
        // The worker panicked and the default hook has already printed it.
        Err(RecvTimeoutError::Disconnected) => None,
    }
}

/// Write the one object the harness accepts, on one line.
fn emit(event: &str, text: &str) {
    // Whitespace is not context. An `additionalContext` of `"  "` is a
    // validated document whose only effect is to make the next one less
    // trusted.
    if text.trim().is_empty() {
        return;
    }
    let document = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "additionalContext": text,
        }
    });
    println!("{document}");
}

/// Read the one line the harness owes us (D-14), but never let the harness
/// decide how long that takes or how much of it there is.
///
/// The read runs on a thread that is deliberately not joined: a blocking read
/// on a pipe nobody closes cannot be cancelled with `std` alone (D-04), so the
/// only way to stop an idle writer from holding this process is to stop waiting
/// on the reader and let process exit take the thread with it. Abandoning it
/// costs nothing beyond the injection it would have fed: there is no
/// half-finished work to leave behind, nothing here writes, and the ingest was
/// started before any of it.
fn drain(event: &str) -> Payload {
    let (done, drained) = mpsc::sync_channel(1);
    // Bytes rather than a `String`: a payload that is not UTF-8 is still a
    // payload that has been drained, and `serde_json` reads a slice directly.
    let reader = std::thread::Builder::new()
        .name("verbatim-hook-stdin".to_string())
        .spawn(move || {
            let mut line = Vec::new();
            let outcome = std::io::stdin()
                .lock()
                .take(MAX_PAYLOAD)
                .read_until(b'\n', &mut line);
            // The main thread may have given up and gone; a send with no
            // receiver left is the timeout case reporting itself.
            //
            // Parsed on this thread rather than the main one so a payload that
            // arrives after the deadline costs the caller nothing at all.
            let _ = done.send((Payload::parse(&line), outcome.err()));
        });
    if let Err(e) = reader {
        eprintln!("verbatim: {event} could not read its payload: {e}");
        return Payload::default();
    }

    match drained.recv_timeout(DRAIN_DEADLINE) {
        Ok((payload, None)) => payload,
        // A read that failed part way still parses what arrived, which is
        // almost always nothing. The line on stderr is the same one it was.
        Ok((payload, Some(e))) => {
            eprintln!("verbatim: {event} could not read its payload: {e}");
            payload
        }
        Err(RecvTimeoutError::Timeout) => {
            eprintln!(
                "verbatim: {event} stopped waiting on a stdin that stayed open for {} ms",
                DRAIN_DEADLINE.as_millis()
            );
            Payload::default()
        }
        Err(RecvTimeoutError::Disconnected) => {
            eprintln!("verbatim: {event} lost the thread reading its payload");
            Payload::default()
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

#[cfg(test)]
mod tests {
    use super::*;

    use verbatim_core::testkit;

    /// Every fixture payload, and the field values it spells for itself.
    ///
    /// `tests/fixtures/hooks/*.json` are the bytes Claude Code 2.1.231 was
    /// observed writing (`tests/fixtures/README.md`), so this asserts the parse
    /// against a recording of the harness rather than against a shape invented
    /// here. The `None`s are as load-bearing as the values: a `prompt` on a
    /// `SessionStart` or a `source` on a `SessionEnd` would mean the parse is
    /// reading a field the event does not carry.
    #[test]
    fn each_recorded_payload_yields_the_fields_it_spells() {
        let session = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";

        let start = Payload::parse(&testkit::fixture_bytes("hooks/session-start.json"));
        assert_eq!(start.session_id.as_deref(), Some(session));
        assert_eq!(start.cwd.as_deref(), Some("/data/code/verbatim"));
        assert!(start
            .transcript_path
            .as_deref()
            .is_some_and(|p| p.ends_with(".jsonl")));
        assert_eq!(start.source.as_deref(), Some("resume"));
        assert_eq!(start.prompt, None);

        let prompt = Payload::parse(&testkit::fixture_bytes("hooks/user-prompt-submit.json"));
        assert_eq!(prompt.session_id.as_deref(), Some(session));
        assert_eq!(prompt.cwd.as_deref(), Some("/data/code/verbatim"));
        assert_eq!(
            prompt.prompt.as_deref(),
            Some("where did we settle the detached spawn's kill behaviour")
        );
        assert_eq!(prompt.source, None);

        let end = Payload::parse(&testkit::fixture_bytes("hooks/session-end.json"));
        assert_eq!(end.session_id.as_deref(), Some(session));
        assert_eq!(end.cwd.as_deref(), Some("/data/code/verbatim"));
        assert_eq!(end.prompt, None);
        assert_eq!(end.source, None);

        let compact = Payload::parse(&testkit::fixture_bytes("hooks/post-compact.json"));
        assert_eq!(compact.session_id.as_deref(), Some(session));
        assert_eq!(compact.cwd.as_deref(), Some("/data/code/verbatim"));
        assert_eq!(compact.prompt, None);
        // `trigger`, not `source`: PostCompact carries a different key, and
        // reading `source` off it must not invent one.
        assert_eq!(compact.source, None);
    }

    /// Nothing about a malformed payload is an error. Each of these is a real
    /// way stdin arrives: a writer that sent nothing, a writer that sent prose,
    /// a truncated line (what [`MAX_PAYLOAD`] leaves), a JSON value that is not
    /// an object, and an object whose fields are the wrong type.
    #[test]
    fn a_payload_that_is_not_an_object_of_strings_has_every_field_absent() {
        for line in [
            &b""[..],
            &b"not json at all"[..],
            &br#"{"session_id":"0e5e6a1e-9f2b-4c7a-8d31-6b4f2a"#[..],
            &b"[1, 2, 3]"[..],
            &b"\"a bare string\""[..],
            &br#"{"session_id":7,"cwd":null,"prompt":{"text":"hi"},"source":[]}"#[..],
        ] {
            assert_eq!(
                Payload::parse(line),
                Payload::default(),
                "parsing {:?} found a field in it",
                String::from_utf8_lossy(line)
            );
        }
    }
}
