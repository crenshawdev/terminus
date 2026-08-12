//! Reading a byte range out of a session blob.

use std::cell::Cell;

use crate::error::{Error, Result};

use super::BlobHeader;

/// Reads ranges of a session's uncompressed stream out of its blob.
///
/// The reader counts the blocks it decompresses. That counter is the instrument
/// STOR-01 and AC1 are stated in terms of - "reading a single turn decompresses
/// only the blocks that turn occupies" is a claim about a number, so the number
/// is measured rather than argued from the code. It stays in the normal build
/// for that reason, not behind a test feature.
pub struct BlobReader<'a> {
    blob: &'a [u8],
    header: BlobHeader,
    blocks_decompressed: Cell<usize>,
}

impl<'a> BlobReader<'a> {
    /// Parse a blob's header, validating its block table.
    pub fn open(blob: &'a [u8]) -> Result<Self> {
        Ok(BlobReader {
            header: BlobHeader::parse(blob)?,
            blob,
            blocks_decompressed: Cell::new(0),
        })
    }

    pub fn header(&self) -> &BlobHeader {
        &self.header
    }

    pub fn uncompressed_len(&self) -> u64 {
        self.header.uncompressed_len
    }

    pub fn block_size(&self) -> u32 {
        self.header.block_size
    }

    /// Blocks decompressed since [`Self::reset_block_counter`], or since the
    /// reader was opened.
    pub fn blocks_decompressed(&self) -> usize {
        self.blocks_decompressed.get()
    }

    /// Zero the counter, so the next read is measured on its own.
    pub fn reset_block_counter(&self) {
        self.blocks_decompressed.set(0);
    }

    /// Read `len` bytes at `offset`, both in uncompressed-stream coordinates
    /// (D-04), decompressing only the blocks the range overlaps.
    ///
    /// A range may span several blocks. STOR-01's "one block" holds for turns
    /// that fit in one, and 28 of 6,731 sampled records exceed 65,536 bytes with
    /// a maximum of 1,134,645 (D-05), so N blocks is the general case and one
    /// block is the common one.
    ///
    /// Reading past the end of the stream is an error, never a short read: a
    /// truncated turn would be indistinguishable from a turn that is short.
    pub fn read_range(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let end = offset.checked_add(len).ok_or(Error::RangeOutOfBounds {
            offset,
            len,
            uncompressed_len: self.header.uncompressed_len,
        })?;
        if end > self.header.uncompressed_len {
            return Err(Error::RangeOutOfBounds {
                offset,
                len,
                uncompressed_len: self.header.uncompressed_len,
            });
        }
        if len == 0 {
            return Ok(Vec::new());
        }

        let block_size = u64::from(self.header.block_size);
        let first = (offset / block_size) as usize;
        let last = ((end - 1) / block_size) as usize;

        let mut out = Vec::with_capacity(len as usize);
        for index in first..=last {
            let block = super::decompress_block(self.blob, &self.header, index)?;
            self.blocks_decompressed
                .set(self.blocks_decompressed.get() + 1);

            let block_start = (index as u64) * block_size;
            let from = offset.saturating_sub(block_start) as usize;
            let to = (end - block_start).min(block.len() as u64) as usize;
            out.extend_from_slice(&block[from..to]);
        }

        debug_assert_eq!(out.len() as u64, len);
        Ok(out)
    }

    /// Read the whole session stream. Every block, by definition.
    pub fn read_all(&self) -> Result<Vec<u8>> {
        self.read_range(0, self.header.uncompressed_len)
    }
}
