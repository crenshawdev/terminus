//! `verbatim ingest <path.jsonl>`: one explicit transcript file.
//!
//! One file and not a tree, deliberately (D-18). The no-arg walk over
//! `~/.claude/projects` is phase 2 (ING-05); keeping phase 1 on one named file
//! is what puts the crash harness (AC2) and the lock race (AC6) on a small
//! fixture instead of the real 962 MB tree.

use std::path::PathBuf;

use verbatim_core::ingest::{self, Outcome};

use super::Failure;

pub fn run(args: Args) -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    match ingest::run(&data_dir, &args.transcript)? {
        // Exit 0 with nothing on stdout. Losing the lock race is the expected
        // outcome of a second hook spawn, not a failure to report (ING-02), and
        // a transcript with no new complete record is the steady state.
        Outcome::LockHeld | Outcome::UpToDate => Ok(()),
        Outcome::Committed(_) => Ok(()),
    }
}

/// The parsed command line for `ingest`.
#[derive(Debug)]
pub struct Args {
    pub transcript: PathBuf,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut transcript: Option<PathBuf> = None;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Value(v) if transcript.is_none() => transcript = Some(PathBuf::from(v)),
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    let transcript = transcript.ok_or_else(|| {
        Failure::Misuse("ingest needs a transcript path: verbatim ingest <path.jsonl>".into())
    })?;
    Ok(Args { transcript })
}
