//! The session archive's bytes, as block rows.
//!
//! A session's stream is cut into fixed-size uncompressed blocks, each
//! compressed on its own and stored as one row, keyed by `session_no` and
//! `block_no`, with BLAKE3 over its uncompressed bytes. The layout lives in the
//! schema; [`block`] is the codec that fills the rows. Independent blocks are
//! the point: reading one turn fetches and decompresses only the blocks that
//! turn covers (STOR-01), and an append rewrites at most the partial tail.
//!
//! `turns` rows address the uncompressed stream (D-04), never a block, so a
//! later block-size change rewrites no turn row.

pub mod block;

pub use block::{append, write, Block, BlockReader, Written};

/// Uncompressed bytes per block for a new session.
///
/// From `DESIGN-BRIEF.md:73`, and still unmeasured against the real turn-size
/// distribution (p50 726 bytes, p90 4,860). Each session records the size it
/// was cut at in `session_meta.block_size` (D-05), so changing this applies to
/// new sessions only and needs no rewrite.
pub const BLOCK_SIZE: usize = 65536;

/// zstd compression level.
///
/// D-08 measured level 3 at 4.21x on real transcripts cut into 64 KB blocks,
/// against level 9's 4.49x and level 19's 4.80x. No dictionary: a dictionary
/// would become part of the archive format.
pub const ZSTD_LEVEL: i32 = 3;

/// Largest block size a session may record.
///
/// The size comes from a column and every decompression buffer is sized from
/// it, so an unbounded value turns a corrupt row into an allocation request,
/// and an allocation failure aborts the process rather than returning
/// [`Error::BlobFormat`](crate::Error::BlobFormat) - killing `verify` and
/// `reindex`, the two commands whose job is to survive corruption and name it.
/// 1024x [`BLOCK_SIZE`] leaves a later size change plenty of room and keeps the
/// worst case a bounded allocation that fails as an error.
const MAX_BLOCK_SIZE: u32 = 64 * 1024 * 1024;

/// Largest buffer a read reserves up front from a stored length.
///
/// Sized above D-05's largest observed record (1,134,645 bytes) so every real
/// turn read still reserves exactly once. Past it the buffer grows as blocks
/// actually decompress, which is what keeps a bogus `uncompressed_len` from
/// being an allocation request in its own right.
const MAX_RESERVE: u64 = 4 * 1024 * 1024;
