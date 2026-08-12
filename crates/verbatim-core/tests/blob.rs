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
