//! How much of one record the archive stores (ING-07, phase 8 D-05).
//!
//! Two things this module is NOT, because both are easy to read into it and
//! both are wrong.
//!
//! It is not redaction. Ingest-time redaction is barred outright
//! (`.planning/PROJECT.md`: it makes the store lossy, defeats "verbatim" and
//! destroys what it strips; redaction is egress-only). What happens here is
//! ELISION: a whole named subtree is dropped and a mark is left saying how many
//! bytes stood there. Nothing is rewritten in place, nothing is scanned for
//! sensitive content, and no value is ever partially kept.
//!
//! And it is not a filter on what gets indexed. It changes the bytes the ARCHIVE
//! holds, which is the only thing in this product that is not rebuildable, so
//! choosing a reduced mode is irreversible for the records it touches -
//! `blob::append` copies completed blocks across untouched, so a store whose
//! mode changes mid-life holds a mix inside one blob and no later pass revisits
//! the earlier bytes.
//!
//! # What is elided, and why those two keys
//!
//! The top-level `toolUseResult` and the top-level `attachment` objects, and
//! nothing else. They are the two subtrees
//! [`crate::index::text`]'s module doc names as identifiable without parsing a
//! message body, `toolUseResult` is 41% of the real corpus by bytes, and both
//! are output rather than authorship: a `lean` or `minimal` store keeps every
//! prompt, every assistant turn, every tool name and every tool argument.
//!
//! D-12 is why it has to happen HERE, on the record's JSON line, rather than off
//! the derived tables: `SELECT count(*) FROM turns WHERE record_type='user' AND
//! tool_name IS NOT NULL` returns 0 on the live store while `user` records hold
//! 57.3% of the stream, so an implementation filtering on `turns.tool_name`
//! elides nothing at all.
//!
//! # What may not move
//!
//! An elided record must still be the same turn, of the same type, at the same
//! ordinal. The top-level `type`, `uuid`, `timestamp`, `sessionId`,
//! `session_id`, `cwd`, `gitBranch`, `subtype` and `compactMetadata` fields are
//! what [`crate::parse::record::Record::parse`] classifies on and what
//! `derive::derive_turn` and `ingest::write_session_meta` read. Nothing here
//! touches any of them: the only mutation is one key's VALUE being replaced.
//!
//! # The cost of choosing one
//!
//! An elided record's searchable text shrinks with it. `index::text::project`
//! reads those same two subtrees, so under `lean` and `minimal` the tokens that
//! were in them are no longer in `turns_fts`. That is the half a user notices as
//! "search stopped finding that", and it is the trade the mode is.
//!
//! # Two exactness notes about the round trip
//!
//! Only an elided line is re-serialized, and only an elided line changes shape.
//! `serde_json`'s object map is ordered by key here, so a re-serialized line
//! comes back with its keys sorted and its whitespace normalized; and a JSON
//! number that is not exactly representable as an `f64` comes back in `f64`'s
//! spelling. Neither matters, because [`CaptureMode::Full`] is the only mode
//! that promises the source bytes back and it never reaches the parser.

use std::borrow::Cow;

use serde_json::{Map, Value};

use crate::config::CaptureMode;

/// The reserved key an elided value is replaced by, whose value is the number of
/// bytes the elided subtree serialized to.
///
/// One key rather than an object of fields: it names the elision and it carries
/// the size, and a reader that finds it knows both "this was dropped on purpose"
/// and "this is what it cost". Spelled in `terminus`'s own namespace so it
/// cannot collide with a key Claude Code writes, and never emitted into
/// `turns_fts` - `index::text` emits no keys and no numbers, so an elided record
/// contributes nothing rather than contributing the word `terminusElided`.
pub const ELISION_MARK: &str = "terminusElided";

/// The top-level keys the reduced modes elide, in the order they are tested.
///
/// Fixed, and deliberately not configurable: these two are identifiable without
/// parsing a message body, and every other subtree of a record is either the
/// authorship a reduced mode is defined to keep or too small to be worth a knob.
pub const ELIDED_FIELDS: [&str; 2] = ["toolUseResult", "attachment"];

/// How large a subtree has to be, serialized, before [`CaptureMode::Lean`]
/// elides it.
///
/// 8 KB, which is where the phase CONTEXT measured the corpus splitting -
/// records over 8 KB are 52.0% of the stream bytes - and which is the reading of
/// `DESIGN-BRIEF.md:163`'s "elide LARGE tool-result and attachment bodies". It
/// is also what makes `minimal` strictly smaller than `lean` on any transcript
/// holding a small tool result: without a threshold the two modes would store
/// the same bytes.
pub const LEAN_THRESHOLD_BYTES: usize = 8_192;

/// The bytes to store for one record's line, under one mode.
///
/// The line is the record WITHOUT its terminating newline, exactly as
/// [`crate::parse::scan_from`] hands it to `Record::parse`; the framing is
/// [`elide_stream`]'s.
///
/// Under [`CaptureMode::Full`] the input is returned untouched and is never
/// parsed. That is what keeps the default path byte-identical to the transcript
/// and free - a full pass over a two-thousand-file tree pays nothing for a
/// feature it is not using.
///
/// Under the reduced modes a line that is not a JSON object, or is one carrying
/// neither elided key, is also returned untouched and never re-serialized. So
/// only the records that actually shrink pay the round trip, and only they
/// change shape.
pub fn elide(line: &[u8], mode: CaptureMode) -> Cow<'_, [u8]> {
    if mode.is_full() {
        return Cow::Borrowed(line);
    }
    let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(line) else {
        // Not JSON, or JSON that is not an object. `journal.jsonl` files and
        // malformed lines both land here, and both keep their bytes: a scan that
        // rewrote what it could not classify would be the one way this module
        // could lose a record.
        return Cow::Borrowed(line);
    };
    if !elide_fields(&mut object, mode) {
        return Cow::Borrowed(line);
    }
    match serde_json::to_vec(&Value::Object(object)) {
        Ok(bytes) => Cow::Owned(bytes),
        // A value that parsed cannot fail to serialize, but the archive is not
        // the place to prove it: the source line is always a valid answer.
        Err(_) => Cow::Borrowed(line),
    }
}

/// Replace whichever elided fields this mode names. `true` when anything moved.
fn elide_fields(object: &mut Map<String, Value>, mode: CaptureMode) -> bool {
    let mut changed = false;
    for field in ELIDED_FIELDS {
        let Some(value) = object.get(field) else {
            continue;
        };
        if is_mark(value) {
            // Already elided - a re-elision would only overwrite the recorded
            // size with the size of the mark itself.
            continue;
        }
        let Ok(serialized) = serde_json::to_vec(value) else {
            continue;
        };
        let elide_this = match mode {
            CaptureMode::Full => false,
            // `DESIGN-BRIEF.md:163`: the LARGE bodies only. Measured on the
            // compact serialization, which is what a transcript writes.
            CaptureMode::Lean => serialized.len() > LEAN_THRESHOLD_BYTES,
            CaptureMode::Minimal => true,
        };
        if !elide_this {
            continue;
        }
        object.insert(field.to_owned(), mark(serialized.len()));
        changed = true;
    }
    changed
}

/// The marker object an elided value is replaced by.
fn mark(bytes: usize) -> Value {
    let mut marker = Map::new();
    marker.insert(ELISION_MARK.to_owned(), Value::from(bytes as u64));
    Value::Object(marker)
}

/// Is this value an elision mark, and if so how many bytes did it stand in for?
///
/// The one reader of the mark's shape, so a caller asking "was this elided"
/// never spells the key itself. An object carrying [`ELISION_MARK`] and nothing
/// else, whose value is a non-negative integer.
pub fn elided_bytes(value: &Value) -> Option<u64> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object.get(ELISION_MARK)?.as_u64()
}

fn is_mark(value: &Value) -> bool {
    elided_bytes(value).is_some()
}

/// Apply [`elide`] across a run of complete transcript lines, preserving the
/// framing exactly.
///
/// The input is a whole-line range - every line terminated by its `\n`, which is
/// what `Scan::consumed` guarantees by stopping at the last newline (D-14). The
/// output holds the same number of lines in the same order, each still
/// terminated, so the record count, the turn ordinals and the classification of
/// every line are unchanged and only the bytes inside some of them are fewer.
///
/// An empty line is passed through: `scan_from` yields no record for one, but
/// its byte is in the stream and the two coordinate systems have to agree about
/// it. Under [`CaptureMode::Full`] the whole range is borrowed and not one byte
/// is examined.
pub fn elide_stream(lines: &[u8], mode: CaptureMode) -> Cow<'_, [u8]> {
    if mode.is_full() {
        return Cow::Borrowed(lines);
    }
    // At most the input's length: elision only ever removes bytes, and a line
    // that would not shrink is copied verbatim.
    let mut out = Vec::with_capacity(lines.len());
    let mut start = 0usize;
    for (index, byte) in lines.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        out.extend_from_slice(&elide(&lines[start..index], mode));
        out.push(b'\n');
        start = index + 1;
    }
    // Whatever follows the last newline is not a complete record and is not this
    // function's to interpret. Callers hand it whole lines, so this is normally
    // empty; passing it through keeps the function total rather than making a
    // truncated range lose its tail.
    out.extend_from_slice(&lines[start..]);
    Cow::Owned(out)
}
