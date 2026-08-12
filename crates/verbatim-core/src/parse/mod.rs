//! Scanning a transcript byte range into records and a resume offset.
//!
//! Two rules carry this module. Lines are split on `\n` and never trimmed, so
//! what the ingest hands to the blob stays byte-identical to the source
//! (D-13/AC7). And the resume offset is the byte just past the **last** `\n`,
//! never end-of-file (D-14): all 1,221 top-level and 812 sidecar transcripts
//! measured end with a newline, so a trailing partial line is a record still
//! being written and must not be archived or indexed until it is complete.

pub mod record;

pub use record::{Record, Turn, TURN_TYPES};

/// The result of scanning one byte range of a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// One entry per complete line, in file order. Empty lines yield nothing.
    pub records: Vec<Record>,
    /// Absolute offset just past the last `\n` in the scanned range: where the
    /// next pass resumes, and the only offset a watermark may take (D-14).
    ///
    /// Equal to the range's start offset when the range holds no newline at
    /// all, which is the "one long record still being written" case: no
    /// progress, nothing archived.
    pub resume_offset: u64,
}

impl Scan {
    /// Records that D-03 classifies as turns, in byte order.
    pub fn turns(&self) -> impl Iterator<Item = (&Record, &Turn)> {
        self.records
            .iter()
            .filter_map(|r| r.turn.as_ref().map(|t| (r, t)))
    }

    /// How many records became turns.
    pub fn turn_count(&self) -> usize {
        self.records.iter().filter(|r| r.is_turn()).count()
    }

    /// Bytes of `buffer` this scan consumed: everything up to the resume point.
    pub fn consumed(&self, start_offset: u64) -> usize {
        (self.resume_offset - start_offset) as usize
    }
}

/// Scan a whole transcript from its beginning.
pub fn scan(bytes: &[u8]) -> Scan {
    scan_from(bytes, 0, 0)
}

/// Scan `bytes`, which begin at `start_offset` in the transcript, numbering
/// turns from `first_turn_seq`.
///
/// `first_turn_seq` is what makes an incremental pass agree with a rebuild: a
/// tail read numbers its turns on from the ones already stored, and a rebuild
/// over the whole stream from 0 lands on the same numbers, because both count
/// in byte order (D-02).
pub fn scan_from(bytes: &[u8], start_offset: u64, first_turn_seq: i64) -> Scan {
    let mut records = Vec::new();
    let mut turn_seq = first_turn_seq;
    let mut line_start = 0usize;
    let mut resume = 0usize;

    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line = &bytes[line_start..index];
        // A blank line is not a record. Its bytes still reach the blob, because
        // the blob takes the raw range and never a rebuilt projection of it.
        if !line.is_empty() {
            let record = Record::parse(line, start_offset + line_start as u64, turn_seq);
            if record.is_turn() {
                turn_seq += 1;
            }
            records.push(record);
        }
        line_start = index + 1;
        resume = index + 1;
    }

    Scan {
        records,
        resume_offset: start_offset + resume as u64,
    }
}
