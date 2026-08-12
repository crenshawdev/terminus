//! Writing a session blob, and extending one that already exists.

use crate::error::{Error, Result};
use crate::store::ARCHIVE_FORMAT;

use super::{BlobHeader, BlockEntry, BLOCK_SIZE, CODEC_ZSTD, ZSTD_LEVEL};

/// A freshly written blob and what the caller has to record about it.
#[derive(Debug, Clone)]
pub struct Written {
    /// The blob, ready for `sessions.blob`.
    pub bytes: Vec<u8>,
    /// BLAKE3 over the **uncompressed** session bytes, for `session_meta`.
    ///
    /// Over the uncompressed bytes and not the blob, deliberately (D-06): a
    /// codec change or a format bump must not invalidate every stored checksum,
    /// or `verify` cannot tell corruption from a decoder change.
    pub checksum: [u8; 32],
    /// Total uncompressed bytes the blob now carries.
    pub uncompressed_len: u64,
    /// Blocks compressed to produce this blob.
    pub blocks_compressed: usize,
}

/// Compress a whole byte stream into a new blob.
///
/// Takes bytes and imposes no record structure: the blob holds the transcript
/// verbatim, never a parsed projection (D-13), so a phase-3 change to
/// extraction can be replayed against the original bytes.
pub fn write(data: &[u8]) -> Result<Written> {
    let mut blocks = Vec::new();
    let mut payload = Vec::new();
    let compressed = compress_blocks(data, 0, &mut blocks, &mut payload)?;
    Ok(finish(data, blocks, payload, compressed))
}

/// Cut `data` into blocks starting at uncompressed offset `start_at`, appending
/// entries and compressed bytes to `blocks` and `payload`. Returns the number of
/// blocks compressed.
fn compress_blocks(
    data: &[u8],
    start_at: u64,
    blocks: &mut Vec<BlockEntry>,
    payload: &mut Vec<u8>,
) -> Result<usize> {
    let mut compressed = 0;
    for (i, chunk) in data.chunks(BLOCK_SIZE).enumerate() {
        let bytes = zstd::bulk::compress(chunk, ZSTD_LEVEL).map_err(|source| Error::Codec {
            operation: "compress",
            source,
        })?;
        blocks.push(BlockEntry {
            uncompressed_start: start_at + (i * BLOCK_SIZE) as u64,
            compressed_offset: payload.len() as u64,
            compressed_len: bytes.len() as u32,
        });
        payload.extend_from_slice(&bytes);
        compressed += 1;
    }
    Ok(compressed)
}

fn finish(
    data: &[u8],
    blocks: Vec<BlockEntry>,
    payload: Vec<u8>,
    blocks_compressed: usize,
) -> Written {
    let uncompressed_len = data.len() as u64;
    Written {
        bytes: finish_parts(uncompressed_len, blocks, payload),
        checksum: *blake3::hash(data).as_bytes(),
        uncompressed_len,
        blocks_compressed,
    }
}

/// Assemble header, table and payload.
///
/// `compressed_offset` in `blocks` is payload-relative on the way in and
/// blob-absolute on the way out: the header's size depends on the number of
/// blocks, so absolute offsets cannot be known until the block count is final.
fn finish_parts(uncompressed_len: u64, mut blocks: Vec<BlockEntry>, payload: Vec<u8>) -> Vec<u8> {
    let mut header = BlobHeader {
        format: ARCHIVE_FORMAT as u16,
        codec: CODEC_ZSTD,
        block_size: BLOCK_SIZE as u32,
        uncompressed_len,
        blocks: Vec::new(),
    };
    let prefix = super::HEADER_LEN + blocks.len() * super::TABLE_ENTRY_LEN;
    for block in &mut blocks {
        block.compressed_offset += prefix as u64;
    }
    header.blocks = blocks;

    let mut out = header.to_bytes();
    debug_assert_eq!(out.len(), prefix);
    debug_assert_eq!(out.len(), header.prefix_len());
    out.extend_from_slice(&payload);
    out
}
