//! The block-framed zstd session blob.
//!
//! A session is stored as one blob: a header, a block offset table, and the
//! session's bytes cut into fixed-size uncompressed blocks, each compressed
//! independently. Independent blocks are the whole point - reading one turn
//! decompresses only the blocks that turn occupies (STOR-01) instead of the
//! whole session.
//!
//! # Layout
//!
//! ```text
//! magic        4  b"VBLB"
//! format       2  u16 LE   archive format that wrote this blob
//! codec        2  u16 LE   1 = zstd
//! block_size   4  u32 LE   uncompressed bytes per block, last block short
//! stream_len   8  u64 LE   total uncompressed bytes
//! block_count  4  u32 LE
//! table       20 * block_count
//!                 u64 LE   uncompressed start offset
//!                 u64 LE   compressed offset from the start of the blob
//!                 u32 LE   compressed length
//! payload         compressed blocks, in order
//! ```
//!
//! The table is what D-04 turns on: `turns` rows address the *uncompressed*
//! stream, and the blob translates. A future block-size change re-blobs and
//! rewrites no turn row, which is what "the archive table never migrates" is
//! there to protect.
//!
//! The uncompressed start offset is redundant with `block_size` and stored
//! anyway, so parsing can check the two against each other instead of trusting
//! either.

use crate::error::{Error, Result};

mod reader;
mod writer;

pub use reader::BlobReader;
pub use writer::{append, write, Appended, Written};

/// Uncompressed bytes per block.
///
/// From `DESIGN-BRIEF.md:73`, and CONTEXT flags it as never measured against
/// the real turn-size distribution (p50 726 bytes, p90 4,860). It is a header
/// field rather than a constant baked into turn coordinates precisely so that
/// changing it later costs a re-blob and no turn-row rewrite. Do not change it
/// in phase 1.
pub const BLOCK_SIZE: usize = 65536;

/// zstd compression level.
///
/// D-08 measured level 3 at 4.21x on real block-framed transcripts against
/// level 9's 4.49x and level 19's 4.80x. No dictionary: a dictionary would
/// become part of an archive format that never migrates.
pub const ZSTD_LEVEL: i32 = 3;

/// Largest `block_size` a header may declare.
///
/// The field is header-controlled and every decompression buffer is sized from
/// it, so an unbounded value turns a corrupt blob into a header-driven
/// allocation: `u32::MAX` in a 45-byte blob asks for 4 GiB, and an allocation
/// failure aborts the process rather than returning [`Error::BlobFormat`] -
/// killing `verify` and `reindex`, the two commands whose whole job is to
/// survive corruption and name it. 1024x this build's [`BLOCK_SIZE`] leaves a
/// future block-size change all the room D-04 promises it while keeping the
/// worst case a bounded allocation that fails as an error.
const MAX_BLOCK_SIZE: u32 = 64 * 1024 * 1024;

/// Largest buffer a read reserves up front from a header-declared length.
///
/// Sized above D-05's largest observed record (1,134,645 bytes) so every real
/// turn read still reserves exactly once. Past it the buffer grows as blocks
/// actually decompress, which is what keeps a bogus `uncompressed_len` from
/// being an allocation request in its own right.
const MAX_RESERVE: u64 = 4 * 1024 * 1024;

const MAGIC: [u8; 4] = *b"VBLB";
const CODEC_ZSTD: u16 = 1;
const HEADER_LEN: usize = 24;
const TABLE_ENTRY_LEN: usize = 20;

/// Where one block's compressed bytes live, and what uncompressed range they
/// cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockEntry {
    pub uncompressed_start: u64,
    pub compressed_offset: u64,
    pub compressed_len: u32,
}

/// A parsed blob header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobHeader {
    pub format: u16,
    pub codec: u16,
    pub block_size: u32,
    pub uncompressed_len: u64,
    pub blocks: Vec<BlockEntry>,
}

impl BlobHeader {
    /// Parse and validate a blob's header and block table.
    ///
    /// Every consistency check lives here so no caller has to repeat one: a
    /// header that parses is a header whose table can be trusted to address
    /// bytes inside this blob.
    pub fn parse(blob: &[u8]) -> Result<Self> {
        if blob.len() < HEADER_LEN {
            return Err(Error::BlobFormat {
                detail: format!("{} bytes is shorter than a header", blob.len()),
            });
        }
        if blob[0..4] != MAGIC {
            return Err(Error::BlobFormat {
                detail: "bad magic".into(),
            });
        }
        let format = u16::from_le_bytes([blob[4], blob[5]]);
        let codec = u16::from_le_bytes([blob[6], blob[7]]);
        if codec != CODEC_ZSTD {
            return Err(Error::BlobFormat {
                detail: format!("unknown codec id {codec}"),
            });
        }
        let block_size = u32::from_le_bytes([blob[8], blob[9], blob[10], blob[11]]);
        if block_size == 0 {
            return Err(Error::BlobFormat {
                detail: "block size 0".into(),
            });
        }
        if block_size > MAX_BLOCK_SIZE {
            return Err(Error::BlobFormat {
                detail: format!(
                    "block size {block_size} exceeds the {MAX_BLOCK_SIZE} byte maximum"
                ),
            });
        }
        let uncompressed_len = u64::from_le_bytes(blob[12..20].try_into().unwrap());
        let block_count = u32::from_le_bytes(blob[20..24].try_into().unwrap()) as usize;

        let expected_blocks = uncompressed_len.div_ceil(u64::from(block_size));
        if expected_blocks != block_count as u64 {
            return Err(Error::BlobFormat {
                detail: format!(
                    "{block_count} blocks for {uncompressed_len} bytes at block size {block_size}, \
                     expected {expected_blocks}"
                ),
            });
        }

        let table_end = HEADER_LEN
            .checked_add(block_count.checked_mul(TABLE_ENTRY_LEN).ok_or_else(|| {
                Error::BlobFormat {
                    detail: format!("{block_count} blocks overflows the block table"),
                }
            })?)
            .ok_or_else(|| Error::BlobFormat {
                detail: "block table overflows the blob".into(),
            })?;
        if blob.len() < table_end {
            return Err(Error::BlobFormat {
                detail: format!(
                    "{} bytes cannot hold a {block_count}-entry block table",
                    blob.len()
                ),
            });
        }

        // Blocks are written back to back and never overlap, so their compressed
        // lengths have to fit in the payload region. Checking the sum, not just
        // each extent, is what bounds `uncompressed_len` against bytes that are
        // actually present: without it a small blob can declare a block table
        // whose blocks all point at the same few bytes and claim gigabytes of
        // stream behind them.
        let payload_capacity = (blob.len() - table_end) as u64;
        let mut payload_used: u64 = 0;

        let mut blocks = Vec::with_capacity(block_count);
        for i in 0..block_count {
            let at = HEADER_LEN + i * TABLE_ENTRY_LEN;
            let uncompressed_start = u64::from_le_bytes(blob[at..at + 8].try_into().unwrap());
            let compressed_offset = u64::from_le_bytes(blob[at + 8..at + 16].try_into().unwrap());
            let compressed_len = u32::from_le_bytes(blob[at + 16..at + 20].try_into().unwrap());

            let want_start = (i as u64) * u64::from(block_size);
            if uncompressed_start != want_start {
                return Err(Error::BlobFormat {
                    detail: format!(
                        "block {i} starts at {uncompressed_start}, expected {want_start}"
                    ),
                });
            }
            let end = compressed_offset
                .checked_add(u64::from(compressed_len))
                .ok_or_else(|| Error::BlobFormat {
                    detail: format!("block {i} extent overflows"),
                })?;
            if compressed_offset < table_end as u64 || end > blob.len() as u64 {
                return Err(Error::BlobFormat {
                    detail: format!(
                        "block {i} extent {compressed_offset}..{end} is outside the blob payload"
                    ),
                });
            }
            payload_used += u64::from(compressed_len);
            if payload_used > payload_capacity {
                return Err(Error::BlobFormat {
                    detail: format!(
                        "blocks 0..={i} claim {payload_used} compressed bytes, but the blob holds \
                         {payload_capacity} bytes of payload"
                    ),
                });
            }

            blocks.push(BlockEntry {
                uncompressed_start,
                compressed_offset,
                compressed_len,
            });
        }

        Ok(BlobHeader {
            format,
            codec,
            block_size,
            uncompressed_len,
            blocks,
        })
    }

    /// Serialize a header and its block table.
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.blocks.len() * TABLE_ENTRY_LEN);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&self.format.to_le_bytes());
        out.extend_from_slice(&self.codec.to_le_bytes());
        out.extend_from_slice(&self.block_size.to_le_bytes());
        out.extend_from_slice(&self.uncompressed_len.to_le_bytes());
        out.extend_from_slice(&(self.blocks.len() as u32).to_le_bytes());
        for block in &self.blocks {
            out.extend_from_slice(&block.uncompressed_start.to_le_bytes());
            out.extend_from_slice(&block.compressed_offset.to_le_bytes());
            out.extend_from_slice(&block.compressed_len.to_le_bytes());
        }
        out
    }

    /// Bytes the header and its table occupy at the front of the blob.
    fn prefix_len(&self) -> usize {
        HEADER_LEN + self.blocks.len() * TABLE_ENTRY_LEN
    }

    /// How many uncompressed bytes block `index` covers.
    pub fn block_len(&self, index: usize) -> u64 {
        let start = (index as u64) * u64::from(self.block_size);
        let end = (start + u64::from(self.block_size)).min(self.uncompressed_len);
        end.saturating_sub(start)
    }
}

/// Decompress one block's payload.
fn decompress_block(blob: &[u8], header: &BlobHeader, index: usize) -> Result<Vec<u8>> {
    let entry = header.blocks[index];
    let from = entry.compressed_offset as usize;
    let to = from + entry.compressed_len as usize;
    let expected = header.block_len(index) as usize;

    let plain =
        zstd::bulk::decompress(&blob[from..to], expected).map_err(|source| Error::Codec {
            operation: "decompress",
            source,
        })?;
    if plain.len() != expected {
        return Err(Error::BlobFormat {
            detail: format!(
                "block {index} decompressed to {} bytes, expected {expected}",
                plain.len()
            ),
        });
    }
    Ok(plain)
}

/// Decompress an entire session stream.
///
/// Every block, by definition, so it is the one read that ignores the block
/// table's whole purpose. `verify` and `reindex` want exactly this; a turn read
/// wants [`BlobReader::read_range`] instead.
pub fn read_all(blob: &[u8]) -> Result<Vec<u8>> {
    BlobReader::open(blob)?.read_all()
}
