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

/// The result of extending an existing blob.
#[derive(Debug, Clone)]
pub struct Appended {
    pub bytes: Vec<u8>,
    pub checksum: [u8; 32],
    pub uncompressed_len: u64,
    /// zstd compression calls this append actually made.
    ///
    /// The real measurement, counted where the work happens: an implementation
    /// that quietly recompressed the whole stream would show it here and
    /// nowhere else, because zstd is deterministic and the resulting bytes
    /// would be identical either way.
    pub blocks_compressed: usize,
    /// How many of those calls redid bytes the blob already held.
    ///
    /// D-07's bound: at most one, the trailing partial block. Completed blocks
    /// are immutable and are copied across.
    pub blocks_recompressed: usize,
}

/// Append `extra` to the stream `blob` already holds.
///
/// Completed blocks are immutable once committed: their compressed bytes are
/// copied across untouched and only the trailing partial block is recompressed
/// (D-07). `SessionEnd` does not fire on a crash (`DESIGN-BRIEF.md:116`), so a
/// live session is re-ingested repeatedly while it grows; recompressing the
/// whole blob each pass would recompress a 10 MB session dozens of times and
/// push the full blob size into the WAL every time.
///
/// An existing block's uncompressed start offset is never moved, because `turns`
/// rows already point at those coordinates.
///
/// `expected` is the checksum already recorded for what `blob` holds
/// (`session_meta.checksum`), and it is checked before anything is written. It
/// is a parameter rather than an internal detail because there is no way to
/// re-derive it: the checksum this call returns covers the appended stream, so
/// an append that did not verify first would hash the corruption and hand back
/// a blob that is self-consistent and wrong. zstd frames here carry no content
/// checksum, so a flipped bit inside a completed block decodes cleanly to
/// different bytes rather than failing, and completed blocks are copied across
/// untouched - which means the append is the only place that damage can still
/// be caught. Since a live session is appended to on every pass while it grows,
/// the alternative is that the next pass certifies it forever and `verify` then
/// compares a corrupt stream against a checksum minted from that same corrupt
/// stream.
pub fn append(blob: &[u8], expected: &[u8; 32], extra: &[u8]) -> Result<Appended> {
    let header = BlobHeader::parse(blob)?;
    if header.block_size as usize != BLOCK_SIZE {
        return Err(Error::BlobFormat {
            detail: format!(
                "cannot append to a blob with block size {}; this build writes {BLOCK_SIZE}",
                header.block_size
            ),
        });
    }

    // Decompressing the whole stream is not new cost: the checksum is defined
    // over the whole uncompressed stream (D-06) and BLAKE3 exposes no resumable
    // state, so this read was already being paid to recompute it. Paying it
    // here instead, on the stream going in, buys the verification as well - and
    // hashing `existing || extra` afterwards means the new blob is never read
    // back at all.
    let existing = super::read_all(blob)?;
    let actual = *blake3::hash(&existing).as_bytes();
    if actual != *expected {
        return Err(Error::BlobChecksumMismatch {
            expected: hex(expected),
            actual: hex(&actual),
        });
    }

    // The last block is partial exactly when the stream does not end on a block
    // boundary. Everything before it is complete and immutable.
    let complete = (header.uncompressed_len / u64::from(header.block_size)) as usize;
    let tail_len = (header.uncompressed_len % u64::from(header.block_size)) as usize;

    let mut blocks = Vec::with_capacity(complete + 1);
    let mut payload = Vec::new();
    for index in 0..complete {
        let entry = header.blocks[index];
        let from = entry.compressed_offset as usize;
        let to = from + entry.compressed_len as usize;
        blocks.push(BlockEntry {
            uncompressed_start: entry.uncompressed_start,
            // Rewritten by `finish`; the header grows as blocks are added, so a
            // block's position in the payload is stable but its offset in the
            // blob is not.
            compressed_offset: payload.len() as u64,
            compressed_len: entry.compressed_len,
        });
        payload.extend_from_slice(&blob[from..to]);
    }

    // Rebuild the stream from the partial tail onwards and compress that. The
    // tail comes out of the stream just verified, not a second decompression of
    // the same block.
    let mut rest = Vec::with_capacity(tail_len + extra.len());
    if tail_len > 0 {
        rest.extend_from_slice(&existing[existing.len() - tail_len..]);
    }
    rest.extend_from_slice(extra);

    let compressed = compress_blocks(
        &rest,
        complete as u64 * u64::from(header.block_size),
        &mut blocks,
        &mut payload,
    )?;

    let uncompressed_len = header.uncompressed_len + extra.len() as u64;
    let written = finish_parts(uncompressed_len, blocks, payload);

    // Over the whole uncompressed stream (D-06). The appended stream is exactly
    // the verified bytes followed by `extra`, so it is hashed from those rather
    // than by decompressing the blob that was just built.
    let mut hasher = blake3::Hasher::new();
    hasher.update(&existing);
    hasher.update(extra);
    let checksum = *hasher.finalize().as_bytes();

    Ok(Appended {
        bytes: written,
        checksum,
        uncompressed_len,
        blocks_compressed: compressed,
        blocks_recompressed: usize::from(tail_len > 0),
    })
}

/// A checksum as it appears in a message. Only ever used in error text.
fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
