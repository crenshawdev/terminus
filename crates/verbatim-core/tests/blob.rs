//! The block-framed zstd blob: writing, reading a range, and appending.

use verbatim_core::blob::{self, BlobHeader, BLOCK_SIZE};

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
    let seed = match std::env::var("VERBATIM_TEST_SEED") {
        Ok(v) => v.parse().expect("VERBATIM_TEST_SEED must be a u64"),
        Err(_) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64,
    };
    println!("{label}: VERBATIM_TEST_SEED={seed}");
    seed
}

/// Compressible bytes: real transcripts are JSON, not noise.
fn textish(len: usize) -> Vec<u8> {
    let pattern = b"{\"type\":\"assistant\",\"uuid\":\"aaaaaaaa-0000-4000-8000-000000000001\"}\n";
    pattern.iter().copied().cycle().take(len).collect()
}

fn check_round_trip(data: &[u8]) {
    let written = blob::write(data).expect("write");
    let header = BlobHeader::parse(&written.bytes).expect("parse");

    assert_eq!(header.block_size as usize, BLOCK_SIZE);
    assert_eq!(header.uncompressed_len, data.len() as u64);
    assert_eq!(
        header.blocks.len(),
        data.len().div_ceil(BLOCK_SIZE),
        "block count for {} bytes",
        data.len()
    );
    assert_eq!(written.blocks_compressed, header.blocks.len());
    assert_eq!(written.uncompressed_len, data.len() as u64);

    let back = blob::read_all(&written.bytes).expect("read_all");
    assert!(back == data, "round trip differed for {} bytes", data.len());

    // The writer's checksum is BLAKE3 over the uncompressed bytes, computed
    // here independently rather than taken from the writer's own path.
    assert_eq!(written.checksum, *blake3::hash(data).as_bytes());
}

#[test]
fn round_trips_the_named_boundary_lengths() {
    for len in [0usize, 1, 65535, 65536, 65537, 200 * 1024, 1_200 * 1024] {
        check_round_trip(&textish(len));
    }
}

#[test]
fn round_trips_incompressible_bytes_too() {
    let mut rng = Rng(seed("round_trips_incompressible_bytes_too"));
    // zstd stores rather than shrinks noise; a block can come out larger than
    // it went in and the offset table has to survive that.
    for len in [0usize, 1, 65535, 65536, 65537, 200 * 1024] {
        check_round_trip(&rng.bytes(len));
    }
}

#[test]
fn round_trips_pseudorandom_lengths() {
    let mut rng = Rng(seed("round_trips_pseudorandom_lengths"));
    for _ in 0..24 {
        let len = rng.below(300 * 1024) as usize;
        check_round_trip(&textish(len));
    }
}

#[test]
fn an_empty_stream_is_a_blob_with_no_blocks() {
    let written = blob::write(&[]).expect("write");
    let header = BlobHeader::parse(&written.bytes).expect("parse");
    assert_eq!(header.blocks.len(), 0);
    assert_eq!(header.uncompressed_len, 0);
    assert!(blob::read_all(&written.bytes).unwrap().is_empty());
}

#[test]
fn block_extents_are_in_order_and_inside_the_blob() {
    let data = textish(200 * 1024);
    let written = blob::write(&data).expect("write");
    let header = BlobHeader::parse(&written.bytes).expect("parse");

    let mut previous_end = 0u64;
    for (i, block) in header.blocks.iter().enumerate() {
        assert_eq!(block.uncompressed_start, (i * BLOCK_SIZE) as u64);
        assert!(
            block.compressed_offset >= previous_end,
            "block {i} overlaps its predecessor"
        );
        previous_end = block.compressed_offset + u64::from(block.compressed_len);
        assert!(previous_end <= written.bytes.len() as u64);
    }
    assert_eq!(previous_end, written.bytes.len() as u64);
}

#[test]
fn a_damaged_header_is_refused_rather_than_guessed_at() {
    let mut written = blob::write(&textish(100_000)).unwrap().bytes;
    written[0] ^= 0xff;
    assert!(BlobHeader::parse(&written).is_err(), "bad magic accepted");

    let mut short = blob::write(&textish(100_000)).unwrap().bytes;
    short.truncate(10);
    assert!(BlobHeader::parse(&short).is_err(), "stub accepted");

    // A block count that disagrees with the stream length is a corrupted table,
    // not a blob to read optimistically.
    let mut wrong = blob::write(&textish(100_000)).unwrap().bytes;
    wrong[20..24].copy_from_slice(&7u32.to_le_bytes());
    assert!(
        BlobHeader::parse(&wrong).is_err(),
        "bad block count accepted"
    );
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

fn blob_of(len: usize) -> (Vec<u8>, Vec<u8>) {
    let data = textish(len);
    let written = blob::write(&data).expect("write");
    (data, written.bytes)
}

#[test]
fn a_range_inside_one_block_decompresses_exactly_one_block() {
    let (data, bytes) = blob_of(200 * 1024);
    let reader = blob::BlobReader::open(&bytes).unwrap();

    let offset = 2 * BLOCK_SIZE + 1000;
    reader.reset_block_counter();
    let got = reader.read_range(offset as u64, 100).unwrap();

    assert!(got == data[offset..offset + 100]);
    assert_eq!(reader.blocks_decompressed(), 1);
}

#[test]
fn a_range_spanning_three_blocks_decompresses_exactly_three() {
    let (data, bytes) = blob_of(200 * 1024);
    let reader = blob::BlobReader::open(&bytes).unwrap();

    let offset = 10;
    let end = 2 * BLOCK_SIZE + 10;
    reader.reset_block_counter();
    let got = reader
        .read_range(offset as u64, (end - offset) as u64)
        .unwrap();

    assert!(got == data[offset..end]);
    assert_eq!(reader.blocks_decompressed(), 3);
}

#[test]
fn reading_the_whole_stream_decompresses_every_block() {
    let (data, bytes) = blob_of(200 * 1024);
    let reader = blob::BlobReader::open(&bytes).unwrap();
    assert_eq!(reader.header().blocks.len(), 4);

    reader.reset_block_counter();
    let got = reader.read_all().unwrap();

    assert!(got == data);
    assert_eq!(reader.blocks_decompressed(), 4);
}

#[test]
fn ranges_straddling_every_block_boundary_read_correctly() {
    let (data, bytes) = blob_of(200 * 1024);
    let reader = blob::BlobReader::open(&bytes).unwrap();
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
            let got = reader.read_range(offset as u64, len as u64).unwrap();
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
fn the_block_count_matches_the_windows_touched_for_random_ranges() {
    let mut rng = Rng(seed(
        "the_block_count_matches_the_windows_touched_for_random_ranges",
    ));
    let (data, bytes) = blob_of(300 * 1024);
    let reader = blob::BlobReader::open(&bytes).unwrap();
    let total = data.len();

    for _ in 0..200 {
        let offset = rng.below(total as u64) as usize;
        let len = rng.below((total - offset) as u64 + 1) as usize;
        reader.reset_block_counter();
        let got = reader.read_range(offset as u64, len as u64).unwrap();
        assert!(
            got == data[offset..offset + len],
            "bytes differ at offset {offset} len {len}"
        );
        assert_eq!(
            reader.blocks_decompressed(),
            windows_touched(offset, len),
            "block count for offset {offset} len {len}"
        );
    }
}

#[test]
fn reading_past_the_end_is_an_error_not_a_short_read() {
    let (_data, bytes) = blob_of(100_000);
    let reader = blob::BlobReader::open(&bytes).unwrap();

    assert!(reader.read_range(99_999, 2).is_err());
    assert!(reader.read_range(100_001, 0).is_err());
    assert!(reader.read_range(0, 100_001).is_err());
    assert!(
        reader.read_range(u64::MAX, 1).is_err(),
        "overflow must not wrap"
    );

    // The exact end is not past it.
    assert_eq!(reader.read_range(100_000, 0).unwrap().len(), 0);
    assert_eq!(reader.read_range(99_999, 1).unwrap().len(), 1);
}

#[test]
fn an_empty_range_decompresses_nothing() {
    let (_data, bytes) = blob_of(200 * 1024);
    let reader = blob::BlobReader::open(&bytes).unwrap();
    reader.reset_block_counter();
    assert!(reader.read_range(70_000, 0).unwrap().is_empty());
    assert_eq!(reader.blocks_decompressed(), 0);
}

// --- appending -----------------------------------------------------------

/// The compressed bytes of each block, sliced out of a blob by its own table.
fn block_extents(bytes: &[u8]) -> Vec<Vec<u8>> {
    let header = BlobHeader::parse(bytes).unwrap();
    header
        .blocks
        .iter()
        .map(|b| {
            let from = b.compressed_offset as usize;
            bytes[from..from + b.compressed_len as usize].to_vec()
        })
        .collect()
}

/// Build a 200 KB blob 1 KB at a time and hold it to the one-shot blob.
#[test]
fn appending_a_kilobyte_at_a_time_matches_a_one_shot_write() {
    const STEP: usize = 1024;
    const TOTAL: usize = 200 * 1024;

    let data = textish(TOTAL);
    let one_shot = blob::write(&data).unwrap();

    let mut incremental = blob::write(&[]).unwrap().bytes;
    let mut total_compressions = 0usize;

    for chunk in data.chunks(STEP) {
        let before = BlobHeader::parse(&incremental).unwrap();
        let completed_before = (before.uncompressed_len / u64::from(before.block_size)) as usize;
        let extents_before = block_extents(&incremental);

        let appended = blob::append(&incremental, chunk).unwrap();

        assert!(
            appended.blocks_recompressed <= 1,
            "an append recompressed {} blocks; D-07 allows the trailing one",
            appended.blocks_recompressed
        );
        assert!(
            appended.blocks_compressed <= 2,
            "an append made {} compression calls",
            appended.blocks_compressed
        );
        total_compressions += appended.blocks_compressed;

        // Completed blocks are immutable: same compressed bytes, same
        // uncompressed start, before and after.
        let extents_after = block_extents(&appended.bytes);
        let after = BlobHeader::parse(&appended.bytes).unwrap();
        for i in 0..completed_before {
            assert!(
                extents_after[i] == extents_before[i],
                "append rewrote completed block {i}"
            );
            assert_eq!(
                after.blocks[i].uncompressed_start, before.blocks[i].uncompressed_start,
                "append moved block {i}"
            );
        }

        incremental = appended.bytes;
    }

    // 200 appends, at most one extra compression when a block boundary falls
    // inside a chunk. Recompressing the whole stream each time would be ~416.
    println!("total zstd compressions across the incremental build: {total_compressions}");
    assert!(
        total_compressions <= 210,
        "{total_compressions} compressions is whole-stream work, not tail work"
    );

    // The stream itself is identical to the one-shot write, byte for byte.
    assert!(blob::read_all(&incremental).unwrap() == data);
    assert!(block_extents(&incremental) == block_extents(&one_shot.bytes));
    assert!(incremental == one_shot.bytes, "blobs differ byte for byte");
}

#[test]
fn an_appended_blob_reads_ranges_exactly_like_a_one_shot_one() {
    let mut rng = Rng(seed(
        "an_appended_blob_reads_ranges_exactly_like_a_one_shot_one",
    ));
    let data = textish(200 * 1024);
    let one_shot = blob::write(&data).unwrap().bytes;

    let mut incremental = blob::write(&[]).unwrap().bytes;
    for chunk in data.chunks(7000) {
        incremental = blob::append(&incremental, chunk).unwrap().bytes;
    }

    let a = blob::BlobReader::open(&incremental).unwrap();
    let b = blob::BlobReader::open(&one_shot).unwrap();
    for _ in 0..100 {
        let offset = rng.below(data.len() as u64);
        let len = rng.below(data.len() as u64 - offset + 1);
        a.reset_block_counter();
        b.reset_block_counter();
        assert!(
            a.read_range(offset, len).unwrap() == b.read_range(offset, len).unwrap(),
            "bytes differ at offset {offset} len {len}"
        );
        assert_eq!(
            a.blocks_decompressed(),
            b.blocks_decompressed(),
            "block count differs at offset {offset} len {len}"
        );
    }
}

#[test]
fn the_appended_checksum_covers_the_whole_stream() {
    let first = textish(100_000);
    let second = textish(50_000);

    let blob_one = blob::write(&first).unwrap();
    let appended = blob::append(&blob_one.bytes, &second).unwrap();

    let mut whole = first.clone();
    whole.extend_from_slice(&second);

    assert_eq!(appended.uncompressed_len, whole.len() as u64);
    assert_eq!(appended.checksum, *blake3::hash(&whole).as_bytes());
    assert!(blob::read_all(&appended.bytes).unwrap() == whole);
}

#[test]
fn appending_nothing_changes_nothing() {
    let data = textish(100_000);
    let written = blob::write(&data).unwrap();
    let appended = blob::append(&written.bytes, &[]).unwrap();

    assert!(appended.bytes == written.bytes);
    assert_eq!(appended.checksum, written.checksum);
    assert_eq!(appended.uncompressed_len, written.uncompressed_len);
}

#[test]
fn appending_onto_an_exact_block_boundary_recompresses_nothing() {
    let data = textish(2 * BLOCK_SIZE);
    let written = blob::write(&data).unwrap();
    let appended = blob::append(&written.bytes, &textish(10)).unwrap();

    assert_eq!(
        appended.blocks_recompressed, 0,
        "a stream ending on a block boundary has no partial tail to redo"
    );
    assert_eq!(appended.blocks_compressed, 1);
}

#[test]
fn an_append_crossing_a_block_boundary_redoes_only_the_tail() {
    // A partial tail plus enough bytes to finish that block and start the next.
    let written = blob::write(&textish(BLOCK_SIZE - 100)).unwrap();
    let appended = blob::append(&written.bytes, &textish(200)).unwrap();

    assert_eq!(
        appended.blocks_compressed, 2,
        "finishing one block and starting the next is two compressions"
    );
    assert_eq!(
        appended.blocks_recompressed, 1,
        "exactly one of them redid bytes the blob already held"
    );
    assert_eq!(BlobHeader::parse(&appended.bytes).unwrap().blocks.len(), 2);
}
