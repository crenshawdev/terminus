//! The block codec: a session stream cut into numbered blocks, each compressed
//! on its own and hashed over its uncompressed bytes.
//!
//! Pure functions over bytes. No SQL and no connection: the archive module
//! fetches and writes the rows, and this decides what goes in them.

use std::cell::Cell;
use std::ops::Range;

use crate::error::{Error, Result};

use super::{MAX_BLOCK_SIZE, MAX_RESERVE, ZSTD_LEVEL};

/// One block row: its number, its compressed bytes, and BLAKE3 over its
/// uncompressed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub block_no: u64,
    pub bytes: Vec<u8>,
    pub hash: [u8; 32],
}

/// What a write or an append produced, and what the caller records about it.
#[derive(Debug, Clone)]
pub struct Written {
    /// The rows to write, in `block_no` order. On an append that ended
    /// mid-block the first is the rewritten tail at its own number.
    pub blocks: Vec<Block>,
    /// The session checksum: BLAKE3 over every block hash in `block_no` order.
    pub checksum: [u8; 32],
    /// Total uncompressed bytes the session now holds.
    pub uncompressed_len: u64,
    /// zstd compression calls made.
    ///
    /// Counted where the work happens: an append that quietly recompressed the
    /// whole stream would show it here and nowhere else, because zstd is
    /// deterministic and the rows would come out identical either way.
    pub blocks_compressed: usize,
    /// How many of those calls redid bytes already stored. At most one, the
    /// partial tail.
    pub blocks_recompressed: usize,
}

/// Refuse a block size of 0 or above [`MAX_BLOCK_SIZE`].
///
/// The size comes from a column, and every decompression buffer is sized from
/// it, so an unbounded value is an allocation request the store controls.
pub fn check_block_size(block_size: u32) -> Result<()> {
    if block_size == 0 {
        return Err(Error::BlobFormat {
            detail: "block size 0".into(),
        });
    }
    if block_size > MAX_BLOCK_SIZE {
        return Err(Error::BlobFormat {
            detail: format!("block size {block_size} exceeds the {MAX_BLOCK_SIZE} byte maximum"),
        });
    }
    Ok(())
}

/// BLAKE3 over the block hashes, concatenated in `block_no` order.
pub fn checksum<'a>(hashes: impl IntoIterator<Item = &'a [u8; 32]>) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for hash in hashes {
        hasher.update(hash);
    }
    *hasher.finalize().as_bytes()
}

/// Cut `data` into blocks of `block_size` bytes, numbered from `first_block_no`,
/// each compressed on its own.
///
/// The hash is over the uncompressed bytes, never the compressed ones: a later
/// zstd that encodes the same block differently would otherwise fail every
/// stored hash, and `verify` could not tell corruption from a codec change.
pub fn cut(data: &[u8], block_size: u32, first_block_no: u64) -> Result<Vec<Block>> {
    check_block_size(block_size)?;
    data.chunks(block_size as usize)
        .enumerate()
        .map(|(i, chunk)| {
            let bytes = zstd::bulk::compress(chunk, ZSTD_LEVEL).map_err(|source| Error::Codec {
                operation: "compress",
                source,
            })?;
            Ok(Block {
                block_no: first_block_no + i as u64,
                bytes,
                hash: *blake3::hash(chunk).as_bytes(),
            })
        })
        .collect()
}

/// A whole new session stream, cut from block 0.
pub fn write(data: &[u8], block_size: u32) -> Result<Written> {
    let blocks = cut(data, block_size, 0)?;
    Ok(Written {
        checksum: checksum(blocks.iter().map(|b| &b.hash)),
        uncompressed_len: data.len() as u64,
        blocks_compressed: blocks.len(),
        blocks_recompressed: 0,
        blocks,
    })
}

/// Append `extra` to a stored session, rewriting at most its partial tail.
///
/// `hashes` is every stored block hash in `block_no` order, and `stored` the
/// checksum recorded for them. `tail` is the last block's compressed bytes,
/// given exactly when `uncompressed_len` ends mid-block.
///
/// Two checks and no more, both before anything is cut: the hashes reproduce
/// the checksum, and the tail decodes to its stored hash. Complete blocks are
/// neither an input nor rewritten. zstd frames here carry no content checksum,
/// so a flipped bit in the tail decodes cleanly to different bytes, and an
/// append that skipped the check would mint a fresh hash over the damage - on
/// a live session, appended to every pass, that is the pass that certifies it.
pub fn append(
    block_size: u32,
    uncompressed_len: u64,
    hashes: &[[u8; 32]],
    stored: &[u8; 32],
    tail: Option<&[u8]>,
    extra: &[u8],
) -> Result<Written> {
    check_block_size(block_size)?;
    let actual = checksum(hashes);
    if actual != *stored {
        return Err(Error::BlobChecksumMismatch {
            expected: hex(stored),
            actual: hex(&actual),
        });
    }
    let size = u64::from(block_size);
    let complete = uncompressed_len / size;
    let tail_len = (uncompressed_len % size) as usize;
    let expected_blocks = uncompressed_len.div_ceil(size);
    if hashes.len() as u64 != expected_blocks {
        return Err(Error::BlobFormat {
            detail: format!(
                "{} block hashes for {uncompressed_len} bytes at block size {block_size}, \
                 expected {expected_blocks}",
                hashes.len()
            ),
        });
    }

    let mut rest = Vec::with_capacity(tail_len + extra.len());
    match (tail_len, tail) {
        (0, None) => {}
        (0, Some(_)) => {
            return Err(Error::BlobFormat {
                detail: "a tail block was given for a stream that ends on a block boundary".into(),
            })
        }
        (_, None) => {
            return Err(Error::BlobFormat {
                detail: format!("block {complete} is a partial tail and was not given"),
            })
        }
        (len, Some(compressed)) => {
            let want = hashes[complete as usize];
            let plain = decompress(compressed, len).map_err(|e| Error::BlobChecksumMismatch {
                expected: hex(&want),
                actual: format!("block {complete} that does not decode ({e})"),
            })?;
            let got = *blake3::hash(&plain).as_bytes();
            if got != want {
                return Err(Error::BlobChecksumMismatch {
                    expected: hex(&want),
                    actual: hex(&got),
                });
            }
            rest.extend_from_slice(&plain);
        }
    }

    if extra.is_empty() {
        return Ok(Written {
            blocks: Vec::new(),
            checksum: *stored,
            uncompressed_len,
            blocks_compressed: 0,
            blocks_recompressed: 0,
        });
    }

    rest.extend_from_slice(extra);
    let blocks = cut(&rest, block_size, complete)?;
    let checksum = checksum(
        hashes[..complete as usize]
            .iter()
            .chain(blocks.iter().map(|b| &b.hash)),
    );
    Ok(Written {
        checksum,
        uncompressed_len: uncompressed_len + extra.len() as u64,
        blocks_compressed: blocks.len(),
        blocks_recompressed: usize::from(tail_len > 0),
        blocks,
    })
}

/// Reads ranges of one session's stream out of its block rows, counting the
/// blocks it decompresses.
///
/// The counter is the instrument STOR-01 is stated in: "reading a turn
/// decompresses only the blocks it occupies" is a claim about a number, so it
/// stays in the normal build.
pub struct BlockReader {
    block_size: u64,
    uncompressed_len: u64,
    blocks_decompressed: Cell<usize>,
}

impl BlockReader {
    pub fn new(block_size: u32, uncompressed_len: u64) -> Result<Self> {
        check_block_size(block_size)?;
        Ok(BlockReader {
            block_size: u64::from(block_size),
            uncompressed_len,
            blocks_decompressed: Cell::new(0),
        })
    }

    pub fn uncompressed_len(&self) -> u64 {
        self.uncompressed_len
    }

    /// How many blocks the stream is cut into.
    pub fn block_count(&self) -> u64 {
        self.uncompressed_len.div_ceil(self.block_size)
    }

    /// Uncompressed bytes in block `block_no`: the block size, or what is left
    /// of the stream.
    pub fn block_len(&self, block_no: u64) -> u64 {
        let start = block_no.saturating_mul(self.block_size);
        self.uncompressed_len
            .min(start.saturating_add(self.block_size))
            .saturating_sub(start)
    }

    /// The block numbers `len` bytes at `offset` cover; empty for an empty
    /// range. Past the end is an error, never a short read.
    pub fn blocks(&self, offset: u64, len: u64) -> Result<Range<u64>> {
        let out_of_bounds = Error::RangeOutOfBounds {
            offset,
            len,
            uncompressed_len: self.uncompressed_len,
        };
        let Some(end) = offset.checked_add(len) else {
            return Err(out_of_bounds);
        };
        if end > self.uncompressed_len {
            return Err(out_of_bounds);
        }
        if len == 0 {
            return Ok(0..0);
        }
        Ok(offset / self.block_size..(end - 1) / self.block_size + 1)
    }

    /// Decompress block `block_no`, which must come out at its expected length.
    pub fn decompress(&self, block_no: u64, compressed: &[u8]) -> Result<Vec<u8>> {
        let plain =
            decompress(compressed, self.block_len(block_no) as usize).map_err(|e| match e {
                Error::BlobFormat { detail } => Error::BlobFormat {
                    detail: format!("block {block_no}: {detail}"),
                },
                other => other,
            })?;
        self.blocks_decompressed
            .set(self.blocks_decompressed.get() + 1);
        Ok(plain)
    }

    /// Read `len` bytes at `offset`, decompressing exactly the blocks the range
    /// covers. `compressed` hands back a block's stored bytes, or `None` for a
    /// block that is not there, which fails this range alone.
    pub fn read_range<'b>(
        &self,
        offset: u64,
        len: u64,
        mut compressed: impl FnMut(u64) -> Option<&'b [u8]>,
    ) -> Result<Vec<u8>> {
        let blocks = self.blocks(offset, len)?;
        let end = offset + len;
        // Capped: `len` on a whole read comes off a column, and reserving it
        // outright would make a corrupt length an allocation that aborts.
        let mut out = Vec::with_capacity(len.min(MAX_RESERVE) as usize);
        for block_no in blocks {
            let bytes = compressed(block_no).ok_or_else(|| Error::BlobFormat {
                detail: format!("block {block_no} is missing"),
            })?;
            let block = self.decompress(block_no, bytes)?;
            let block_start = block_no * self.block_size;
            let from = offset.saturating_sub(block_start) as usize;
            let to = (end - block_start).min(block.len() as u64) as usize;
            out.extend_from_slice(&block[from..to]);
        }
        debug_assert_eq!(out.len() as u64, len);
        Ok(out)
    }

    /// Blocks decompressed since [`Self::reset_block_counter`] or creation.
    pub fn blocks_decompressed(&self) -> usize {
        self.blocks_decompressed.get()
    }

    pub fn reset_block_counter(&self) {
        self.blocks_decompressed.set(0);
    }
}

/// Decompress one block to exactly `expected` bytes.
fn decompress(compressed: &[u8], expected: usize) -> Result<Vec<u8>> {
    let plain = zstd::bulk::decompress(compressed, expected).map_err(|source| Error::Codec {
        operation: "decompress",
        source,
    })?;
    if plain.len() != expected {
        return Err(Error::BlobFormat {
            detail: format!("decompressed to {} bytes, expected {expected}", plain.len()),
        });
    }
    Ok(plain)
}

/// A hash as it appears in a message. Only ever used in error text.
fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
