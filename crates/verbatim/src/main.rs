//! The `verbatim` binary.
//!
//! Exit codes (phase 1 subset of the CLI contract): 0 success, 1 operational
//! failure, 2 misuse. Data goes to stdout, errors go to stderr.

use std::process::ExitCode;

const USAGE: &str = "usage: verbatim [--version]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Misuse(msg)) => {
            eprintln!("verbatim: {msg}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// A misuse of the command line. Exit code 2, message on stderr, stdout empty.
struct Misuse(String);

fn run() -> Result<(), Misuse> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_env();
    let mut show_version = false;
    while let Some(arg) = parser.next().map_err(|e| Misuse(e.to_string()))? {
        match arg {
            Short('V') | Long("version") => show_version = true,
            // Usage is what this binary prints with no subcommand, so `--help`
            // needs no arm of its own beyond being accepted.
            Short('h') | Long("help") => {}
            // Subcommands arrive one per task in PLAN-2 (`ingest`, `verify`,
            // `reindex`); until then every value is an unrecognized argument.
            other => return Err(Misuse(unexpected(other))),
        }
    }

    if show_version {
        println!("{}", env!("CARGO_PKG_VERSION"));
    } else {
        println!("{USAGE}");
    }
    Ok(())
}

fn unexpected(arg: lexopt::Arg<'_>) -> String {
    match arg {
        lexopt::Arg::Short(c) => format!("unexpected argument '-{c}'"),
        lexopt::Arg::Long(name) => format!("unexpected argument '--{name}'"),
        lexopt::Arg::Value(v) => format!("unexpected argument '{}'", v.to_string_lossy()),
    }
}
