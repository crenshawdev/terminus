//! Reading a session's archived bytes.
//!
//! Every product read of the archive goes through here, so the readers never
//! name how a session is stored. Ranges and offsets are in the uncompressed
//! session stream (v0.1.0 phase 1 D-04), and nothing here checks a hash
//! (phase 7 D-15): `verify` and append are the checkers.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};

use crate::blob::BlobReader;
use crate::error::{Error, Result};

/// What came back for one session's ranges.
#[derive(Debug)]
pub struct Ranges {
    /// One entry per range asked for, in the same order. A range that would
    /// not read fails alone.
    pub bytes: Vec<Result<Vec<u8>>>,
    /// Blocks decompressed to cut every range.
    pub blocks_decompressed: usize,
}

/// Read `(stream_offset, byte_len)` ranges out of one session, fetching the
/// session once however many ranges are asked of it (D-20).
///
/// `None` when the session has no stored bytes. Stored bytes that will not
/// open fail every range rather than the call.
pub fn ranges(
    conn: &Connection,
    session_key: &str,
    wanted: &[(u64, u64)],
) -> Result<Option<Ranges>> {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT blob FROM sessions WHERE session_key = ?1",
            [session_key],
            |r| r.get(0),
        )
        .optional()?;
    let Some(blob) = blob else { return Ok(None) };

    let reader = match BlobReader::open(&blob) {
        Ok(reader) => reader,
        Err(e) => {
            let detail = e.to_string();
            let bytes = wanted
                .iter()
                .map(|_| {
                    Err(Error::BlobFormat {
                        detail: detail.clone(),
                    })
                })
                .collect();
            return Ok(Some(Ranges {
                bytes,
                blocks_decompressed: 0,
            }));
        }
    };

    let bytes = wanted
        .iter()
        .map(|&(offset, len)| reader.read_range(offset, len))
        .collect();
    Ok(Some(Ranges {
        bytes,
        blocks_decompressed: reader.blocks_decompressed(),
    }))
}

/// One session's whole uncompressed stream.
///
/// The outer error is the database's, and a session with no row is one of
/// them. The inner one is the stored bytes failing to read, which callers
/// report against that session and keep going.
pub fn stream(conn: &Connection, session_key: &str) -> Result<Result<Vec<u8>>> {
    let blob: Vec<u8> = conn.query_row(
        "SELECT blob FROM sessions WHERE session_key = ?1",
        [session_key],
        |r| r.get(0),
    )?;
    Ok(crate::blob::read_all(&blob))
}

/// Every session's stored archive bytes, by `session_key`.
pub fn stored_bytes(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut statement = conn.prepare("SELECT session_key, length(blob) FROM sessions")?;
    let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}
