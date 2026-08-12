//! The `verbatim` binary.
//!
//! Exit codes (phase 1 subset of the CLI contract): 0 success, 1 operational
//! failure, 2 misuse. Data goes to stdout, errors go to stderr.

use std::process::ExitCode;

mod cmd;

use cmd::Failure;

const USAGE: &str = "usage: verbatim [--version] <command>\n\
                     \n\
                     commands:\n  \
                       ingest <path.jsonl>   archive one transcript file";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Operational(msg)) => {
            eprintln!("verbatim: {msg}");
            ExitCode::from(1)
        }
        Err(Failure::Misuse(msg)) => {
            eprintln!("verbatim: {msg}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), Failure> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_env();
    let mut show_version = false;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Short('V') | Long("version") => show_version = true,
            // Usage is what this binary prints with no subcommand, so `--help`
            // needs no arm of its own beyond being accepted.
            Short('h') | Long("help") => {}
            // A subcommand takes over the rest of the command line: the parser
            // is handed on rather than re-created, so `ingest -- --odd-name`
            // reaches the subcommand's own rules.
            Value(name) => return dispatch(&name.to_string_lossy(), &mut parser),
            other => return Err(Failure::Misuse(unexpected(other))),
        }
    }

    if show_version {
        println!("{}", env!("CARGO_PKG_VERSION"));
    } else {
        println!("{USAGE}");
    }
    Ok(())
}

fn dispatch(name: &str, parser: &mut lexopt::Parser) -> Result<(), Failure> {
    match name {
        "ingest" => cmd::ingest::run(cmd::ingest::parse(parser)?),
        other => Err(Failure::Misuse(format!("unknown command '{other}'"))),
    }
}

pub(crate) fn unexpected(arg: lexopt::Arg<'_>) -> String {
    match arg {
        lexopt::Arg::Short(c) => format!("unexpected argument '-{c}'"),
        lexopt::Arg::Long(name) => format!("unexpected argument '--{name}'"),
        lexopt::Arg::Value(v) => format!("unexpected argument '{}'", v.to_string_lossy()),
    }
}
