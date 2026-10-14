//! Reading and writing a session's archived bytes.
//!
//! Every product read of the archive goes through here, so the readers never
//! name how a session is stored. A session is numbered rows in `blocks`, one
//! per block of its stream, cut at the block size `session_meta` records for
//! it. Ranges and offsets are in the uncompressed session stream (v0.1.0 phase
//! 1 D-04), and nothing here checks a hash (phase 7 D-15): `verify` and append
//! are the checkers.

use std::collections::BTreeMap;
use std::ops::Range;

use rusqlite::{Connection, OptionalExtension};

use crate::blob::{Block, BlockReader};
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

/// Read `(stream_offset, byte_len)` ranges out of one session, fetching only
/// the block rows those ranges cover, each row once however many ranges need
/// it (D-13, D-20).
///
/// `None` when the session has no `sessions` row. A session whose layout will
/// not read fails every range rather than the call, and a missing row or a
/// block that will not decode fails only the ranges that need it.
pub fn ranges(
    conn: &Connection,
    session_key: &str,
    wanted: &[(u64, u64)],
) -> Result<Option<Ranges>> {
    let Some((session_no, reader)) = layout(conn, session_key)? else {
        return Ok(None);
    };
    let reader = match reader {
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

    // Merged before fetching, so a row two ranges share is fetched once, and
    // asked for by span rather than one number at a time: a corrupt
    // `uncompressed_len` can make a span enormous, and a span query returns
    // only the rows that exist.
    let mut spans: Vec<Range<u64>> = wanted
        .iter()
        .filter_map(|&(offset, len)| reader.blocks(offset, len).ok())
        .filter(|span| !span.is_empty())
        .collect();
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Range<u64>> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }

    let mut rows: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    let mut statement = conn.prepare_cached(
        "SELECT block_no, bytes FROM blocks
          WHERE session_no = ?1 AND block_no >= ?2 AND block_no < ?3
          ORDER BY block_no",
    )?;
    for span in merged {
        let mut cursor = statement.query(rusqlite::params![
            session_no,
            span.start as i64,
            span.end as i64
        ])?;
        while let Some(row) = cursor.next()? {
            rows.insert(row.get::<_, i64>(0)? as u64, row.get(1)?);
        }
    }

    let bytes = wanted
        .iter()
        .map(|&(offset, len)| {
            reader.read_range(offset, len, |block_no| {
                rows.get(&block_no).map(Vec::as_slice)
            })
        })
        .collect();
    Ok(Some(Ranges {
        bytes,
        blocks_decompressed: reader.blocks_decompressed(),
    }))
}

/// One session's whole uncompressed stream, from every row in `block_no`
/// order.
///
/// The outer error is the database's, and a session with no `sessions` row is
/// one of them. The inner one is the stored bytes failing to read - a missing
/// row, a block that will not decode, no recorded layout - which callers report
/// against that session and keep going.
pub fn stream(conn: &Connection, session_key: &str) -> Result<Result<Vec<u8>>> {
    let (session_no, reader) =
        layout(conn, session_key)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let reader = match reader {
        Ok(reader) => reader,
        Err(e) => return Ok(Err(e)),
    };
    let rows = numbered_rows(conn, session_no)?;
    Ok(decode_rows(&reader, &rows))
}

/// Decode a whole stream from rows already fetched, in `block_no` order.
pub(crate) fn whole(
    block_size: Option<i64>,
    uncompressed_len: u64,
    rows: &[(i64, Vec<u8>)],
) -> Result<Vec<u8>> {
    let reader = BlockReader::new(self::block_size(block_size)?, uncompressed_len)?;
    decode_rows(&reader, rows)
}

/// A recorded `session_meta.block_size`, or why it will not serve.
pub(crate) fn block_size(stored: Option<i64>) -> Result<u32> {
    let Some(size) = stored else {
        return Err(Error::BlobFormat {
            detail: "no block size recorded".into(),
        });
    };
    let size = u32::try_from(size).map_err(|_| Error::BlobFormat {
        detail: format!("block size {size} is out of range"),
    })?;
    crate::blob::block::check_block_size(size)?;
    Ok(size)
}

fn decode_rows(reader: &BlockReader, rows: &[(i64, Vec<u8>)]) -> Result<Vec<u8>> {
    // Positional, and checked: a row out of place is a gap, never a stand-in.
    reader.read_range(0, reader.uncompressed_len(), |block_no| {
        rows.get(block_no as usize)
            .filter(|(no, _)| *no == block_no as i64)
            .map(|(_, bytes)| bytes.as_slice())
    })
}

/// One block row as stored, unchecked.
#[derive(Debug)]
pub struct Row {
    pub block_no: i64,
    pub bytes: Vec<u8>,
    pub hash: Vec<u8>,
}

/// Every block row of one session as stored, in `block_no` order, with nothing
/// checked. `verify` reads through this, because checking the rows is its job.
pub fn rows(conn: &Connection, session_key: &str) -> Result<Vec<Row>> {
    let mut statement = conn.prepare_cached(
        "SELECT b.block_no, b.bytes, b.hash
           FROM blocks b JOIN sessions s USING (session_no)
          WHERE s.session_key = ?1
          ORDER BY b.block_no",
    )?;
    let rows = statement.query_map([session_key], |r| {
        Ok(Row {
            block_no: r.get(0)?,
            bytes: r.get(1)?,
            hash: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Every session's stored archive bytes, by `session_key`: the sum of its
/// block rows' lengths (D-17).
pub fn stored_bytes(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut statement = conn.prepare(
        "SELECT s.session_key, coalesce(sum(length(b.bytes)), 0)
           FROM sessions s LEFT JOIN blocks b USING (session_no)
          GROUP BY s.session_key",
    )?;
    let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// What an append needs from one session's rows (D-10, D-11).
pub(crate) struct AppendState {
    /// Every stored hash in `block_no` order, or why the list will not serve:
    /// a gap in the numbering or a hash that is not 32 bytes. Kept rather than
    /// raised, so a session with nothing new to append is not failed for it.
    pub hashes: std::result::Result<Vec<[u8; 32]>, String>,
    /// The last row's bytes, when the stream ends mid-block.
    pub tail: Option<Vec<u8>>,
    /// Every row's bytes in `block_no` order, only when asked for.
    pub every_row: Option<Vec<(i64, Vec<u8>)>>,
    /// Rows whose bytes were fetched.
    pub rows_read: usize,
}

impl AppendState {
    /// Nothing read: what an evicted session gets, since it has no rows.
    pub(crate) fn empty() -> Self {
        AppendState {
            hashes: Ok(Vec::new()),
            tail: None,
            every_row: None,
            rows_read: 0,
        }
    }
}

/// Read what an append to `session_no` needs: every hash, and the bytes of at
/// most the tail row. With `every_row` it fetches every row's bytes instead,
/// which only a session flagged diverged asks for (D-13).
pub(crate) fn append_state(
    conn: &Connection,
    session_no: i64,
    block_size: Option<i64>,
    uncompressed_len: u64,
    every_row: bool,
) -> Result<AppendState> {
    // The tail is the block a partial stream ends in. No usable block size
    // means no tail is fetched, and the append refuses the size itself.
    let tail_no = block_size
        .filter(|size| *size > 0)
        .map(|size| size as u64)
        .filter(|size| !uncompressed_len.is_multiple_of(*size))
        .map(|size| (uncompressed_len / size) as i64);

    if every_row {
        let mut statement = conn.prepare_cached(
            "SELECT block_no, bytes, hash FROM blocks WHERE session_no = ?1 ORDER BY block_no",
        )?;
        let rows: Vec<(i64, Vec<u8>, Vec<u8>)> = statement
            .query_map([session_no], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let hashes = hash_list(rows.iter().map(|(no, _, hash)| (*no, hash.as_slice())));
        let tail = tail_no.and_then(|n| {
            rows.iter()
                .find(|(no, _, _)| *no == n)
                .map(|(_, bytes, _)| bytes.clone())
        });
        let rows_read = rows.len();
        return Ok(AppendState {
            hashes,
            tail,
            every_row: Some(rows.into_iter().map(|(no, bytes, _)| (no, bytes)).collect()),
            rows_read,
        });
    }

    let mut statement = conn.prepare_cached(
        "SELECT block_no, hash FROM blocks WHERE session_no = ?1 ORDER BY block_no",
    )?;
    let rows: Vec<(i64, Vec<u8>)> = statement
        .query_map([session_no], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let hashes = hash_list(rows.iter().map(|(no, hash)| (*no, hash.as_slice())));
    let tail: Option<Vec<u8>> = match tail_no {
        Some(n) => conn
            .prepare_cached("SELECT bytes FROM blocks WHERE session_no = ?1 AND block_no = ?2")?
            .query_row(rusqlite::params![session_no, n], |r| r.get(0))
            .optional()?,
        None => None,
    };
    Ok(AppendState {
        hashes,
        rows_read: usize::from(tail.is_some()),
        tail,
        every_row: None,
    })
}

/// Write block rows for `session_no`. A row already at a number is replaced,
/// which an append does to the tail and to nothing else.
pub(crate) fn write(conn: &Connection, session_no: i64, blocks: &[Block]) -> Result<()> {
    let mut statement = conn.prepare_cached(
        "INSERT INTO blocks (session_no, block_no, bytes, hash) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(session_no, block_no) DO UPDATE SET
            bytes = excluded.bytes, hash = excluded.hash",
    )?;
    for block in blocks {
        statement.execute(rusqlite::params![
            session_no,
            block.block_no as i64,
            block.bytes,
            block.hash.as_slice()
        ])?;
    }
    Ok(())
}

/// Delete every block row of one session, answering how many went.
pub(crate) fn remove(conn: &Connection, session_key: &str) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM blocks
          WHERE session_no = (SELECT session_no FROM sessions WHERE session_key = ?1)",
        [session_key],
    )?)
}

/// A session's number and a reader over its recorded layout, or `None` when
/// there is no `sessions` row. The inner error is a layout that will not read.
fn layout(conn: &Connection, session_key: &str) -> Result<Option<(i64, Result<BlockReader>)>> {
    let row: Option<(i64, Option<i64>, Option<i64>)> = conn
        .prepare_cached(
            "SELECT s.session_no, m.block_size, m.uncompressed_len
               FROM sessions s LEFT JOIN session_meta m USING (session_key)
              WHERE s.session_key = ?1",
        )?
        .query_row([session_key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(row.map(|(session_no, block_size, len)| (session_no, reader(block_size, len))))
}

fn reader(block_size: Option<i64>, uncompressed_len: Option<i64>) -> Result<BlockReader> {
    let Some(len) = uncompressed_len else {
        return Err(Error::BlobFormat {
            detail: "no session_meta row, so no stream length".into(),
        });
    };
    let block_size = self::block_size(block_size)?;
    let len = u64::try_from(len).map_err(|_| Error::BlobFormat {
        detail: format!("stream length {len} is negative"),
    })?;
    BlockReader::new(block_size, len)
}

/// Every row's bytes for one session, in `block_no` order.
fn numbered_rows(conn: &Connection, session_no: i64) -> Result<Vec<(i64, Vec<u8>)>> {
    let mut statement = conn.prepare_cached(
        "SELECT block_no, bytes FROM blocks WHERE session_no = ?1 ORDER BY block_no",
    )?;
    let rows = statement.query_map([session_no], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Hashes numbered `0..n` with no gap, each 32 bytes.
fn hash_list<'a>(
    rows: impl Iterator<Item = (i64, &'a [u8])>,
) -> std::result::Result<Vec<[u8; 32]>, String> {
    let mut hashes = Vec::new();
    for (i, (block_no, hash)) in rows.enumerate() {
        if block_no != i as i64 {
            return Err(format!("block {i} is missing"));
        }
        hashes.push(
            hash.try_into()
                .map_err(|_| format!("block {i} has a {}-byte hash", hash.len()))?,
        );
    }
    Ok(hashes)
}
