//! `verbatim ingest`: the whole configured tree, or one named transcript file.
//!
//! With no argument it walks every configured transcript root (ING-05). With a
//! path it ingests that one file, which is the shape phase 1 shipped and the
//! shape the crash harness and the lock race still use.

use std::path::PathBuf;

use verbatim_core::ingest::pass::{self, PassOutcome};
use verbatim_core::ingest::{self, Outcome};

use super::Failure;

pub fn run(args: Args) -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    match args {
        Args::One(transcript) => match ingest::run(&data_dir, &transcript)? {
            // Exit 0 with nothing on stdout. Losing the lock race is the
            // expected outcome of a second hook spawn, not a failure to report
            // (ING-02), and a transcript with no new complete record is the
            // steady state.
            Outcome::LockHeld | Outcome::UpToDate | Outcome::Committed(_) => Ok(()),
            // Named on stderr, and exit 0: nothing was read and nothing is
            // wrong. A user who names a file by hand is owed the reason it
            // produced no archive, where the hook spawning this per file is
            // owed silence on stdout.
            Outcome::Excluded(project) => {
                eprintln!(
                    "skipped {}: inside the excluded project {}",
                    transcript.display(),
                    project.display()
                );
                Ok(())
            }
        },
        Args::Tree => tree(&data_dir),
    }
}

fn tree(data_dir: &std::path::Path) -> Result<(), Failure> {
    let summary = match pass::run(data_dir)? {
        PassOutcome::LockHeld => return Ok(()),
        PassOutcome::Ran(summary) => summary,
    };

    // Named on stderr, and exit 0. The tree WAS ingested: a pass that skipped
    // one damaged transcript out of two thousand did the work it was asked to
    // do, and failing the command would make the next hook spawn look broken
    // for as long as the damaged file sits there (D-12). A pass that could not
    // run at all is a different thing and reaches `Failure::Operational`
    // through the `?` above.
    for (path, reason) in summary.failures.iter().chain(&summary.unreadable) {
        eprintln!("{}", pass::note(path, reason));
    }
    if !summary.failures.is_empty() {
        eprintln!("{} file(s) skipped", summary.failures.len());
    }

    // The judgment step's notes, on stderr and not in `runs.error`: it runs
    // after the lock drops and after that row commits (D-07), so this is the
    // only channel it has - there is no log file, by design. Already scrubbed
    // where they were built (D-16); a provider message or a credentials failure
    // must never reach a stream carrying a key.
    for note in &summary.judgment.notes {
        eprintln!("{note}");
    }
    Ok(())
}

/// The parsed command line for `ingest`.
#[derive(Debug, PartialEq, Eq)]
pub enum Args {
    /// No argument: walk every configured transcript root.
    Tree,
    /// One named transcript file.
    One(PathBuf),
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

    // No path is no longer misuse: it is the tree pass. That is an intended
    // change to a shipped contract - `ingest` exited 2 in phase 1.
    Ok(match transcript {
        Some(path) => Args::One(path),
        None => Args::Tree,
    })
}
