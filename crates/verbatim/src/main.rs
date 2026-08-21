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
                       ingest                archive every transcript under the configured roots\n  \
                       ingest <path.jsonl>   archive one transcript file\n  \
                       backfill              archive every transcript already on disk\n  \
                       hook <event>          start an ingest for a Claude Code hook event\n  \
                       search <words>        find a past turn by text, path, error or identifier\n  \
                       show <id>...          print the archived record behind a turn id\n  \
                       sessions              list every archived session a read path may see\n  \
                       verify                check every blob against its checksum\n  \
                       reindex               rebuild the derived tables from the blobs\n  \
                       status                sizes, counts, watermarks and the last ingest run\n  \
                       replay                re-score the logged prompts under other thresholds\n  \
                       doctor                report what is wired into Claude Code and what is not\n  \
                       mcp                   serve the three recall tools over stdio (for Claude Code)\n  \
                       install               wire verbatim into Claude Code's hooks and mcp servers\n  \
                       uninstall             remove what install added, keeping the archive\n\
                     \n\
                     every data command accepts --json: one JSON document on stdout,\n\
                     every diagnostic on stderr, exit 0 on success including an empty result.";

fn main() -> ExitCode {
    // Before the parser, and before anything else: this process may be the
    // reparenting hand-off `cmd::spawn` uses to put the ingest outside every
    // set Claude Code kills a hook through (D-03). If it is, it has already
    // started the process that does the work and has nothing else to do.
    //
    // The test is `cmd::spawn::HANDOFF` as the first argument, so the only way
    // into this branch is a command line that asks for it. Nothing ambient
    // reaches it: an environment this process inherited cannot make an ordinary
    // `verbatim search`, `--version` or unknown hook event return SUCCESS here
    // with an empty stdout, which is what the exit codes above are worth.
    if cmd::spawn::handed_off() {
        return ExitCode::SUCCESS;
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Operational(msg)) => {
            eprintln!("verbatim: {msg}");
            ExitCode::from(1)
        }
        Err(Failure::Silent) => ExitCode::from(1),
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
        // The whole history, once, without the shell waiting for it (ING-11).
        // It prints an estimate first and does the work in a detached process,
        // which is why it is a command of its own rather than a flag on
        // `ingest`: the two return at different times and mean different things.
        "backfill" => cmd::backfill::run(cmd::backfill::parse(parser)?),
        // Written into settings.json by `verbatim install` and never typed by
        // hand. It writes nothing to stdout and exits 0 once its event name is
        // known, because a hook that fails is a hook that can block a prompt.
        "hook" => cmd::hook::run(cmd::hook::parse(parser)?),
        "search" => cmd::search::run(cmd::search::parse(parser)?),
        "show" => cmd::show::run(cmd::show::parse(parser)?),
        "sessions" => cmd::sessions::run(cmd::sessions::parse(parser)?),
        // D-24: the three commands phase 1 and 2 shipped take `--json` and
        // nothing else. `cmd::json_flag` is what replaced `no_more_arguments`
        // here - the rule that a subcommand rejects every argument it was not
        // written for still holds, and `--json` is now one it was.
        "verify" => cmd::verify::run(cmd::json_flag(parser)?),
        "reindex" => cmd::reindex::run(cmd::json_flag(parser)?),
        "status" => cmd::status::run(cmd::json_flag(parser)?),
        // FEED-03. Read-only by construction (D-15) and the one command whose
        // flags move the injection thresholds - for this report and nothing
        // else: the live path compiles them in (D-08).
        "replay" => cmd::replay::run(cmd::replay::parse(parser)?),
        // D-26: the MCP server is this binary, not a second artifact to place
        // and keep in version lockstep. It takes no arguments - a client spawns
        // it, nobody types it - and it writes JSON-RPC on stdout rather than
        // anything the `--json` contract above describes.
        "mcp" => cmd::mcp::run(cmd::mcp::parse(parser)?),
        // Read-only by construction (INST-06, D-12): it opens the store
        // through the read path, never `Store::open`, and creates no file and
        // no directory on any path it takes.
        "doctor" => cmd::doctor::run(cmd::json_flag(parser)?),
        // Human-only and deliberately not a `--json` data command (D-24): it
        // shows a diff and asks once, and a single JSON document on stdout
        // cannot be both of those things.
        "install" => cmd::install::run(cmd::install::parse(parser)?),
        // Human-only for the same reason, and the only command in this binary
        // that deletes anything: it removes what install added, leaves the
        // archive alone and prints its path, and destroys it only under
        // `--purge`, after showing the size and asking (INST-07, D-13).
        "uninstall" => cmd::uninstall::run(cmd::uninstall::parse(parser)?),
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
