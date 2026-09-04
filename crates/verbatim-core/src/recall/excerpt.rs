//! The text under each hit, cut from the archive.
//!
//! **Never from FTS5 (D-05).** Measured on sqlite 3.53.4 against the
//! contentless table `crates/verbatim-core/src/store/schema.rs` declares,
//! `snippet(f,0,'[',']','...',8)` returns an empty string with exit 0 while
//! `bm25(f)` still ranks - because `content=''` means the table holds no text
//! to snippet. An excerpt built from `snippet()` would validate against the
//! documented shape, pass every schema test, and tell the reader nothing.
//!
//! So the bytes come out of the session blob at the `stream_offset` and
//! `byte_len` already on the turn row, and are projected through the same
//! [`crate::index::text::project`] ingest indexed them with. That is what makes
//! the excerpt prose rather than a JSON line: the projection is the rule that
//! knows a `tool_use` block's text from its scaffolding, and writing a second
//! one here would be a second definition of what a turn said.
//!
//! **One blob per session per search (D-20).** rusqlite's incremental blob I/O
//! is behind a `blob` feature this workspace does not enable, so reading one
//! turn materializes the whole compressed session - up to 10.1 MB uncompressed
//! (phase 1 D-05). Hits are therefore grouped by session and each session's
//! blob is selected once, which bounds the cost per request without eliminating
//! it.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};

use crate::blob::BlobReader;
use crate::error::Result;
use crate::index::text;
use crate::observe::egress::Redaction;
use crate::recall::search::Hit;
use crate::recall::Query;

/// How many characters of context an excerpt carries.
///
/// A terminal line and an MCP result are the two consumers and both want a
/// sentence, not a paragraph: `recall_get` is the tool for the whole record,
/// and an excerpt long enough to replace it would put the archive back in every
/// search result.
pub const EXCERPT_CHARS: usize = 240;

/// What stands where text was cut away.
///
/// ASCII, because this is printed to a terminal whose code page is not ours to
/// choose - the Windows console is first-class (`.planning/PROJECT.md`) and a
/// horizontal ellipsis is exactly the character that arrives there as mojibake.
pub const ELISION: &str = "...";

/// How many session blobs an excerpt pass materialized.
///
/// An instrument, not an answer, and in the normal build for the same reason
/// [`BlobReader::blocks_decompressed`] is: "each session's blob is read at most
/// once per search" is a claim about a number, so the number is measured rather
/// than argued from the shape of the code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reads {
    pub blobs: usize,
}

/// Give every hit an excerpt, reading each session's blob once.
///
/// A blob that will not open, a range that will not read, a record that is not
/// JSON: each leaves that one hit with an empty excerpt and every other field
/// intact. A search is not the command that reports archive damage - `verbatim
/// verify` is - and failing the whole search because one session of many is
/// corrupt would hide the results that are fine.
///
/// `redaction` is the CALLER's (phase 4 D-01). Nothing here reads a `Config`,
/// so whether a surface filters stays a property of its own call site and is
/// reviewable there: both of this function's callers - `search::run` and
/// `inject::prompt` - resolve `Redaction::of` from the config they already
/// hold, and each could be changed without the other moving. That is the point
/// of the parameter, not an accident of them currently agreeing.
pub fn attach(
    conn: &Connection,
    query: &Query,
    hits: &mut [Hit],
    redaction: &Redaction<'_>,
) -> Result<Reads> {
    let mut reads = Reads::default();
    if hits.is_empty() {
        return Ok(reads);
    }

    let coordinates = coordinates(conn, hits)?;

    // Grouped by session, which is the whole of D-20: the blob is selected once
    // and every turn of that session is cut out of the one copy.
    let mut by_session: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, hit) in hits.iter().enumerate() {
        by_session
            .entry(hit.session_key.clone())
            .or_default()
            .push(index);
    }

    for (session_key, indexes) in by_session {
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT blob FROM sessions WHERE session_key = ?1",
                [&session_key],
                |r| r.get(0),
            )
            .optional()?;
        reads.blobs += 1;
        let Some(blob) = blob else { continue };
        let Ok(reader) = BlobReader::open(&blob) else {
            continue;
        };

        for index in indexes {
            let Some((offset, len)) = coordinates.get(&hits[index].turn_id) else {
                continue;
            };
            let Ok(bytes) = reader.read_range(*offset as u64, *len as u64) else {
                continue;
            };
            hits[index].excerpt = of_record_with(query, &bytes, redaction);
        }
    }

    Ok(reads)
}

/// `(stream_offset, byte_len)` for every hit, in one statement.
///
/// D-04: both address the UNCOMPRESSED session stream, never the blob.
fn coordinates(conn: &Connection, hits: &[Hit]) -> Result<BTreeMap<i64, (i64, i64)>> {
    let ids: Vec<i64> = hits.iter().map(|hit| hit.turn_id).collect();
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = conn.prepare(&format!(
        "SELECT id, stream_offset, byte_len FROM turns WHERE id IN ({placeholders})"
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
        ))
    })?;

    let mut out = BTreeMap::new();
    for row in rows {
        let (id, coordinates) = row?;
        out.insert(id, coordinates);
    }
    Ok(out)
}

/// One record's bytes, as the sentence a reader should see - unfiltered.
///
/// Public because a context window projects its turns the same way (RCL-08):
/// one rule for what a turn said, whatever asked for it.
///
/// This is [`of_record_with`] under [`Redaction::none`], and it is the
/// signature every caller had before the egress knob existed. A model-facing
/// caller passes its own redaction through [`of_record_with`] instead.
pub fn of_record(query: &Query, record: &[u8]) -> String {
    of_record_with(query, record, &Redaction::none())
}

/// One record's projection, with the caller's redaction applied to it
/// (PRIV-03).
///
/// **The filter runs over the FULL projection, before the window is cut**
/// (phase 4 D-02). Every value-shape rule in `observe::egress` needs a
/// secret's NAME and its VALUE in the same string - `Authorization: Bearer
/// <v>`, `--token <v>`, `"password": "<v>"` - and [`EXCERPT_CHARS`] is a
/// 240-character window centred on the first query token, so a name that falls
/// outside the cut is a value no rule can see. Filtering the window afterwards
/// would leave exactly the secrets whose labels were trimmed off, and would
/// pass a test whose planted secret happened to land whole inside the window.
///
/// Before [`flatten`] as well as before [`window`], and that ordering is also
/// load-bearing: the projection is newline-joined by construction and the
/// header rule is written per line, so collapsing it to one line first would
/// hand every rule a single line and change what "anywhere on a line" means.
///
/// What it costs is measured rather than argued. Over the live store on
/// 2026-09-04, scrubbing the real projection of every turn of the 60 most
/// recent sessions (5,412 records, 12.9 MB) changed 0.18% of projections.
pub fn of_record_with(query: &Query, record: &[u8], redaction: &Redaction<'_>) -> String {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(record) else {
        // A line that is not JSON is archived verbatim all the same (D-13), and
        // it has no projection. Nothing to excerpt is an empty excerpt.
        return String::new();
    };
    let projected = text::project(&value);
    window(query, &flatten(&redaction.apply(&projected)))
}

/// The projection as one line: runs of whitespace become single spaces.
///
/// The projected text is newline-joined by construction - one line per text
/// block, per tool input leaf, per stderr line - and an excerpt is read on one
/// terminal row or inside one JSON string.
fn flatten(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending = false;
    for c in text.chars() {
        if c.is_whitespace() {
            pending = !out.is_empty();
            continue;
        }
        if pending {
            out.push(' ');
            pending = false;
        }
        out.push(c);
    }
    out
}

/// A bounded window around the first query token in `text`.
///
/// The fallback when no token occurs is the head of the projection rather than
/// an empty string. That case is real and is not damage: an entity-only hit
/// matched on a normalized `error` value or on a tool name that the projected
/// text spells differently, and the opening of the turn is still what the turn
/// was about.
fn window(query: &Query, text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= EXCERPT_CHARS {
        return text.to_owned();
    }

    let at = first_token(query, text).unwrap_or(0);
    // A third of the window ahead of the match, so the reader sees what led to
    // it as well as what followed.
    let lead = EXCERPT_CHARS / 3;
    let end = (at.saturating_sub(lead) + EXCERPT_CHARS).min(chars.len());
    let start = end - EXCERPT_CHARS;

    let mut out = String::new();
    if start > 0 {
        out.push_str(ELISION);
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push_str(ELISION);
    }
    out
}

/// The character index of the first query token in `text`, case-insensitively.
fn first_token(query: &Query, text: &str) -> Option<usize> {
    let folded: Vec<char> = text.to_lowercase().chars().collect();
    // Folded on both sides, and the index counted in characters, because the
    // window is cut in characters: a byte index from `str::find` would be the
    // wrong number here for any turn carrying one non-ASCII character.
    let mut best: Option<usize> = None;
    for token in query.tokens() {
        let needle: Vec<char> = token.to_lowercase().chars().collect();
        if needle.is_empty() || needle.len() > folded.len() {
            continue;
        }
        if let Some(at) = folded
            .windows(needle.len())
            .position(|window| window == needle.as_slice())
        {
            best = Some(best.map_or(at, |current: usize| current.min(at)));
        }
    }
    best
}
