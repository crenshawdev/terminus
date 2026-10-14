//! Reading a session's archived bytes.
//!
//! Every product read of the archive goes through here, so the readers never
//! name how a session is stored. Ranges and offsets are in the uncompressed
//! session stream (v0.1.0 phase 1 D-04), and nothing here checks a hash
//! (phase 7 D-15): `verify` and append are the checkers.

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
