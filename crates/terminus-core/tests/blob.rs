//! The block codec: cutting a stream into block rows, reading a range out of
//! them, and appending.

use terminus_core::blob::block::{self, Written};
use terminus_core::blob::{Block, BlockReader, BLOCK_SIZE};

/// SplitMix64. A test that generates its own inputs has to be reproducible from
/// the seed it prints, and pulling a crate in for four lines is not worth it.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next_u64() >> 24) as u8).collect()
    }
}

/// A seed from the environment when reproducing a failure, otherwise the clock.
/// Printed either way.
fn seed(label: &str) -> u64 {
    let seed = match std::env::var("TERMINUS_TEST_SEED") {
        Ok(v) => v.parse().expect("TERMINUS_TEST_SEED must be a u64"),
        Err(_) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64,
    };
    println!("{label}: TERMINUS_TEST_SEED={seed}");
    seed
}

/// Compressible bytes: real transcripts are JSON, not noise.
fn textish(len: usize) -> Vec<u8> {
    let pattern = b"{\"type\":\"assistant\",\"uuid\":\"aaaaaaaa-0000-4000-8000-000000000001\"}\n";
    pattern.iter().copied().cycle().take(len).collect()
}

// --- reading a range -----------------------------------------------------

/// How many distinct 64 KB windows a range touches. The independent
/// calculation the reader's counter is checked against.
fn windows_touched(offset: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (offset + len - 1) / BLOCK_SIZE - offset / BLOCK_SIZE + 1
}

fn rows_of(len: usize) -> (Vec<u8>, Written, BlockReader) {
    let data = textish(len);
    let written = block::write(&data, BS).expect("write");
    let reader = BlockReader::new(BS, written.uncompressed_len).unwrap();
    (data, written, reader)
}

/// A block's stored bytes, as the archive module hands them to the reader.
fn rows<'a>(written: &'a Written) -> impl Fn(u64) -> Option<&'a [u8]> + Copy + 'a {
    move |n| written.blocks.get(n as usize).map(|b| b.bytes.as_slice())
}

#[test]
fn a_range_inside_one_block_decompresses_exactly_one_block() {
    let (data, written, reader) = rows_of(200 * 1024);
    let offset = 2 * BLOCK_SIZE + 1000;
    let got = reader
        .read_range(offset as u64, 100, rows(&written))
        .unwrap();
    assert!(got == data[offset..offset + 100]);
    assert_eq!(reader.blocks_decompressed(), 1);
}

#[test]
fn a_range_spanning_three_blocks_decompresses_exactly_three() {
    let (data, written, reader) = rows_of(200 * 1024);
    let (offset, end) = (10, 2 * BLOCK_SIZE + 10);
    let got = reader
        .read_range(offset as u64, (end - offset) as u64, rows(&written))
        .unwrap();
    assert!(got == data[offset..end]);
    assert_eq!(reader.blocks_decompressed(), 3);
}

#[test]
fn reading_the_whole_stream_decompresses_every_block() {
    let (data, written, reader) = rows_of(200 * 1024);
    assert_eq!(reader.block_count(), 4);
    let got = reader
        .read_range(0, data.len() as u64, rows(&written))
        .unwrap();
    assert!(got == data);
    assert_eq!(reader.blocks_decompressed(), 4);
}

#[test]
fn ranges_straddling_every_block_boundary_read_correctly() {
    let (data, written, reader) = rows_of(200 * 1024);
    let total = data.len();

    let mut checked = 0;
    for boundary in (0..=total).step_by(BLOCK_SIZE) {
        for span in [1usize, 2, 3, 17, 1024, BLOCK_SIZE, BLOCK_SIZE + 5] {
            // A window centred on the boundary, clamped to the stream.
            let offset = boundary.saturating_sub(span / 2);
            let len = span.min(total - offset);
            if len == 0 {
                continue;
            }

            reader.reset_block_counter();
            let got = reader
                .read_range(offset as u64, len as u64, rows(&written))
                .unwrap();
            assert!(
                got == data[offset..offset + len],
                "bytes differ at offset {offset} len {len}"
            );
            assert_eq!(
                reader.blocks_decompressed(),
                windows_touched(offset, len),
                "block count for offset {offset} len {len}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} ranges checked");
}

#[test]
fn an_empty_range_decompresses_nothing() {
    let (_data, written, reader) = rows_of(200 * 1024);
    assert!(reader
        .read_range(70_000, 0, rows(&written))
        .unwrap()
        .is_empty());
    assert_eq!(reader.blocks_decompressed(), 0);
}

// --- block rows ----------------------------------------------------------

const BS: u32 = BLOCK_SIZE as u32;

fn hashes_of(blocks: &[Block]) -> Vec<[u8; 32]> {
    blocks.iter().map(|b| b.hash).collect()
}

/// BLAKE3 over the hashes, computed here rather than through the codec.
fn independent_checksum(blocks: &[Block]) -> [u8; 32] {
    let mut all = Vec::new();
    for b in blocks {
        all.extend_from_slice(&b.hash);
    }
    *blake3::hash(&all).as_bytes()
}

fn check_rows_round_trip(data: &[u8]) {
    let written = block::write(data, BS).expect("write");
    assert_eq!(written.uncompressed_len, data.len() as u64);
    assert_eq!(
        written.blocks.len(),
        data.len().div_ceil(BLOCK_SIZE),
        "block count for {} bytes",
        data.len()
    );
    assert_eq!(written.blocks_compressed, written.blocks.len());
    for (i, (b, chunk)) in written
        .blocks
        .iter()
        .zip(data.chunks(BLOCK_SIZE))
        .enumerate()
    {
        assert_eq!(b.block_no, i as u64);
        assert_eq!(b.hash, *blake3::hash(chunk).as_bytes(), "block {i} hash");
    }
    assert_eq!(written.checksum, independent_checksum(&written.blocks));

    let reader = BlockReader::new(BS, written.uncompressed_len).unwrap();
    let back = reader
        .read_range(0, data.len() as u64, |n| {
            written.blocks.get(n as usize).map(|b| b.bytes.as_slice())
        })
        .expect("read");
    assert!(back == data, "round trip differed for {} bytes", data.len());
}

#[test]
fn block_rows_round_trip_the_named_boundary_lengths() {
    for len in [0usize, 1, 65535, 65536, 65537, 200 * 1024, 1_200 * 1024] {
        check_rows_round_trip(&textish(len));
    }
}

#[test]
fn block_rows_round_trip_pseudorandom_lengths() {
    let mut rng = Rng(seed("block_rows_round_trip_pseudorandom_lengths"));
    for _ in 0..24 {
        let len = rng.below(300 * 1024) as usize;
        check_rows_round_trip(&textish(len));
    }
    for len in [1usize, 65536, 65537] {
        check_rows_round_trip(&rng.bytes(len));
    }
}

#[test]
fn the_session_checksum_is_blake3_over_the_block_hashes() {
    let written = block::write(&textish(200 * 1024), BS).unwrap();
    assert_eq!(written.checksum, independent_checksum(&written.blocks));
    // Never over the stream itself.
    assert_ne!(
        written.checksum,
        *blake3::hash(&textish(200 * 1024)).as_bytes()
    );
    // An empty session hashes an empty list.
    assert_eq!(
        block::write(&[], BS).unwrap().checksum,
        *blake3::hash(&[]).as_bytes()
    );
}

/// Replace the stored rows with what an append returned: the tail at its own
/// number, then the new blocks.
fn apply_rows(rows: &mut Vec<Block>, written: &Written) {
    for b in &written.blocks {
        let at = b.block_no as usize;
        if at < rows.len() {
            rows[at] = b.clone();
        } else {
            assert_eq!(at, rows.len(), "an append skipped a block number");
            rows.push(b.clone());
        }
    }
}

fn tail_of(rows: &[Block], len: u64) -> Option<&[u8]> {
    (!len.is_multiple_of(u64::from(BS))).then(|| rows.last().unwrap().bytes.as_slice())
}

#[test]
fn appending_a_kilobyte_at_a_time_gives_the_rows_of_one_write() {
    const STEP: usize = 1024;
    let data = textish(200 * 1024);
    let one_shot = block::write(&data, BS).unwrap();

    let mut rows: Vec<Block> = Vec::new();
    let mut checksum = block::write(&[], BS).unwrap().checksum;
    let mut len = 0u64;
    let mut total_compressions = 0;
    for chunk in data.chunks(STEP) {
        let complete_before = (len / u64::from(BS)) as usize;
        let before = rows.clone();
        let appended = block::append(
            BS,
            len,
            &hashes_of(&rows),
            &checksum,
            tail_of(&rows, len),
            chunk,
        )
        .unwrap();
        assert!(appended.blocks_recompressed <= 1);
        assert!(appended.blocks_compressed <= 2);
        assert!(
            appended
                .blocks
                .iter()
                .all(|b| b.block_no as usize >= complete_before),
            "an append rewrote a complete block"
        );
        total_compressions += appended.blocks_compressed;
        apply_rows(&mut rows, &appended);
        assert!(rows[..complete_before] == before[..complete_before]);
        checksum = appended.checksum;
        len = appended.uncompressed_len;
    }

    assert!(
        total_compressions <= 210,
        "{total_compressions} compressions"
    );
    assert_eq!(len, data.len() as u64);
    assert!(
        rows == one_shot.blocks,
        "rows differ from the one-shot write"
    );
    assert_eq!(checksum, one_shot.checksum);
}

#[test]
fn a_block_append_onto_a_boundary_recompresses_nothing() {
    let written = block::write(&textish(2 * BLOCK_SIZE), BS).unwrap();
    let appended = block::append(
        BS,
        written.uncompressed_len,
        &hashes_of(&written.blocks),
        &written.checksum,
        None,
        &textish(10),
    )
    .unwrap();
    assert_eq!(appended.blocks_recompressed, 0);
    assert_eq!(appended.blocks_compressed, 1);
    assert_eq!(appended.blocks.len(), 1);
    assert_eq!(appended.blocks[0].block_no, 2);
}

#[test]
fn a_block_append_crossing_a_boundary_redoes_only_the_tail() {
    let first = textish(BLOCK_SIZE - 100);
    let written = block::write(&first, BS).unwrap();
    let appended = block::append(
        BS,
        written.uncompressed_len,
        &hashes_of(&written.blocks),
        &written.checksum,
        Some(&written.blocks[0].bytes),
        &textish(200),
    )
    .unwrap();
    assert_eq!(appended.blocks_compressed, 2);
    assert_eq!(appended.blocks_recompressed, 1);
    let numbers: Vec<u64> = appended.blocks.iter().map(|b| b.block_no).collect();
    assert_eq!(numbers, [0, 1]);

    let mut whole = first.clone();
    whole.extend_from_slice(&textish(200));
    assert_eq!(
        appended.checksum,
        block::write(&whole, BS).unwrap().checksum
    );
}

#[test]
fn a_block_append_of_nothing_writes_nothing() {
    let written = block::write(&textish(100_000), BS).unwrap();
    let appended = block::append(
        BS,
        written.uncompressed_len,
        &hashes_of(&written.blocks),
        &written.checksum,
        Some(&written.blocks[1].bytes),
        &[],
    )
    .unwrap();
    assert!(appended.blocks.is_empty());
    assert_eq!(appended.checksum, written.checksum);
    assert_eq!(appended.blocks_compressed, 0);
}

#[test]
fn a_tampered_hash_list_is_refused() {
    let written = block::write(&textish(200_000), BS).unwrap();
    let mut hashes = hashes_of(&written.blocks);
    hashes[0][0] ^= 0x01;
    let err = block::append(
        BS,
        written.uncompressed_len,
        &hashes,
        &written.checksum,
        Some(&written.blocks[3].bytes),
        b"more",
    )
    .unwrap_err();
    assert!(
        matches!(err, terminus_core::Error::BlobChecksumMismatch { .. }),
        "{err:?}"
    );

    // A block row gone from the middle drops its hash from the list.
    hashes = hashes_of(&written.blocks);
    hashes.remove(1);
    let err = block::append(
        BS,
        written.uncompressed_len,
        &hashes,
        &written.checksum,
        Some(&written.blocks[3].bytes),
        b"more",
    )
    .unwrap_err();
    assert!(
        matches!(err, terminus_core::Error::BlobChecksumMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn a_tampered_tail_is_refused() {
    let data = textish(200_000);
    let written = block::write(&data, BS).unwrap();
    let mut tail = written.blocks[3].bytes.clone();
    let at = tail.len() / 2;
    tail[at] ^= 0x01;

    let err = block::append(
        BS,
        written.uncompressed_len,
        &hashes_of(&written.blocks),
        &written.checksum,
        Some(&tail),
        b"more",
    )
    .unwrap_err();
    assert!(
        matches!(err, terminus_core::Error::BlobChecksumMismatch { .. }),
        "{err:?}"
    );

    // A tail whose bytes were swapped for another block's decodes cleanly to
    // the wrong length or the wrong bytes, and the hash catches it either way.
    let swapped = block::write(
        &textish(data.len() % BLOCK_SIZE)
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>(),
        BS,
    )
    .unwrap();
    let err = block::append(
        BS,
        written.uncompressed_len,
        &hashes_of(&written.blocks),
        &written.checksum,
        Some(&swapped.blocks[0].bytes),
        b"more",
    )
    .unwrap_err();
    assert!(
        matches!(err, terminus_core::Error::BlobChecksumMismatch { .. }),
        "{err:?}"
    );
}

#[test]
fn a_range_decompresses_exactly_the_blocks_it_touches() {
    let mut rng = Rng(seed("a_range_decompresses_exactly_the_blocks_it_touches"));
    let data = textish(300 * 1024);
    let written = block::write(&data, BS).unwrap();
    let reader = BlockReader::new(BS, written.uncompressed_len).unwrap();
    let total = data.len();

    for _ in 0..200 {
        let offset = rng.below(total as u64) as usize;
        let len = rng.below((total - offset) as u64 + 1) as usize;
        let mut asked = Vec::new();
        reader.reset_block_counter();
        let got = reader
            .read_range(offset as u64, len as u64, |n| {
                asked.push(n);
                written.blocks.get(n as usize).map(|b| b.bytes.as_slice())
            })
            .unwrap();
        assert!(
            got == data[offset..offset + len],
            "offset {offset} len {len}"
        );
        assert_eq!(reader.blocks_decompressed(), windows_touched(offset, len));
        let want: Vec<u64> = reader.blocks(offset as u64, len as u64).unwrap().collect();
        assert_eq!(asked, want, "rows fetched for offset {offset} len {len}");
        assert_eq!(asked.len(), windows_touched(offset, len));
    }
}

#[test]
fn a_block_read_past_the_end_is_an_error_not_a_short_read() {
    let written = block::write(&textish(100_000), BS).unwrap();
    let reader = BlockReader::new(BS, 100_000).unwrap();
    let fetch = |n: u64| written.blocks.get(n as usize).map(|b| b.bytes.as_slice());

    for (offset, len) in [(99_999, 2), (100_001, 0), (0, 100_001), (u64::MAX, 1)] {
        assert!(
            matches!(
                reader.read_range(offset, len, fetch),
                Err(terminus_core::Error::RangeOutOfBounds { .. })
            ),
            "offset {offset} len {len}"
        );
    }
    assert_eq!(reader.read_range(100_000, 0, fetch).unwrap().len(), 0);
    assert_eq!(reader.read_range(99_999, 1, fetch).unwrap().len(), 1);
}

#[test]
fn a_missing_block_fails_only_the_ranges_that_need_it() {
    let data = textish(200 * 1024);
    let written = block::write(&data, BS).unwrap();
    let reader = BlockReader::new(BS, written.uncompressed_len).unwrap();
    let fetch = |n: u64| {
        (n != 1)
            .then(|| written.blocks.get(n as usize).map(|b| b.bytes.as_slice()))
            .flatten()
    };
    assert!(reader.read_range(BLOCK_SIZE as u64, 10, fetch).is_err());
    assert!(reader.read_range(10, 10, fetch).unwrap() == data[10..20]);
}

/// The block size comes from a column, and every decompression buffer is sized
/// from it: an absurd value would be an allocation request the store controls.
#[test]
fn an_absurd_block_size_is_refused() {
    for size in [0u32, u32::MAX, 64 * 1024 * 1024 + 1] {
        for err in [
            BlockReader::new(size, 1 << 31).err(),
            block::write(b"x", size).err(),
            block::append(size, 0, &[], &blake3::hash(&[]).into(), None, b"x").err(),
        ] {
            assert!(
                matches!(err, Some(terminus_core::Error::BlobFormat { .. })),
                "block size {size} returned {err:?}"
            );
        }
    }
    assert!(BlockReader::new(64 * 1024 * 1024, 1).is_ok());
}
