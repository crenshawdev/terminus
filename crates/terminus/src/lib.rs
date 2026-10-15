//! The `terminus` binary's code, as a library.
//!
//! `main.rs` picks the allocator and calls [`main`]. The code lives here so the
//! fuzz targets in `fuzz/` can reach the parsers that read outside input, through
//! the [`fuzz`] module, which only `cargo fuzz` builds contain.
//!
//! Exit codes (phase 1 subset of the CLI contract): 0 success, 1 operational
//! failure, 2 misuse. Data goes to stdout, errors go to stderr.

// Stdout goes through `outln!` and `out!`, which survive a closed pipe.
#![cfg_attr(not(test), deny(clippy::print_stdout))]

use std::process::ExitCode;

#[macro_use]
mod out;
mod cmd;

use cmd::Failure;

const USAGE: &str = "usage: terminus [--version] <command>\n\
                     \n\
                     commands:\n  \
                       ingest                archive every transcript under the configured roots\n  \
                       ingest <path.jsonl>   archive one transcript file\n  \
                       backfill              archive every transcript already on disk\n  \
                       hook <event>          start an ingest for a Claude Code hook event\n  \
                       search <words>        find a past turn by text, path, error or identifier\n  \
                       show <id>...          print the archived record behind a turn id\n  \
                       sessions              list every archived session a read path may see\n  \
                       verify                check every block against its hash\n  \
                       reindex               rebuild the derived tables from the archive\n  \
                       status                sizes, counts, watermarks and the last ingest run\n  \
                       compact               reclaim the space retention freed, then truncate the wal\n  \
                       usage                 where the bytes went: by project, by month, by table\n  \
                       export <dir>          write the archived transcripts out, with a manifest\n  \
                       stats                 whether injection is helping: precision, misses, chars\n  \
                       observations          what happened in every session that has closed\n  \
                       observations regenerate   recompute those facts for a selected set\n  \
                       replay                re-score the logged prompts under other thresholds\n  \
                       retention --dry-run   what the next ingest pass would evict and delete\n  \
                       doctor                report what is wired into Claude Code and what is not\n  \
                       mcp                   serve the three recall tools over stdio (for Claude Code)\n  \
                       install               wire terminus into Claude Code's hooks and mcp servers\n  \
                       uninstall             remove what install added, keeping the archive\n  \
                       data move <path>      relocate the whole data directory and record where\n\
                     \n\
                     every data command accepts --json: one JSON document on stdout,\n\
                     every diagnostic on stderr, exit 0 on success including an empty result.";

pub fn main() -> ExitCode {
    // Before the parser, and before anything else: this process may be the
    // reparenting hand-off `cmd::spawn` uses to put the ingest outside every
    // set Claude Code kills a hook through (D-03). If it is, it has already
    // started the process that does the work and has nothing else to do.
    //
    // The test is `cmd::spawn::HANDOFF` as the first argument, so the only way
    // into this branch is a command line that asks for it. Nothing ambient
    // reaches it: an environment this process inherited cannot make an ordinary
    // `terminus search`, `--version` or unknown hook event return SUCCESS here
    // with an empty stdout, which is what the exit codes above are worth.
    if cmd::spawn::handed_off() {
        return ExitCode::SUCCESS;
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Operational(msg)) => {
            eprintln!("terminus: {msg}");
            ExitCode::from(1)
        }
        Err(Failure::Silent) => ExitCode::from(1),
        Err(Failure::Misuse(msg)) => {
            eprintln!("terminus: {msg}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), Failure> {
    match parse(&mut lexopt::Parser::from_env())? {
        Invocation::Version => outln!("{}", env!("CARGO_PKG_VERSION")),
        Invocation::Usage => outln!("{USAGE}"),
        Invocation::Command(command) => return command(),
    }
    Ok(())
}

/// What a command line asks for, parsed and not yet run.
enum Invocation {
    Version,
    Usage,
    Command(Command),
}

/// A subcommand whose arguments are parsed, waiting to run.
type Command = Box<dyn FnOnce() -> Result<(), Failure>>;

fn parse(parser: &mut lexopt::Parser) -> Result<Invocation, Failure> {
    use lexopt::prelude::*;

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
            Value(name) => {
                return command(&name.to_string_lossy(), parser).map(Invocation::Command)
            }
            other => return Err(Failure::Misuse(unexpected(other))),
        }
    }

    Ok(if show_version {
        Invocation::Version
    } else {
        Invocation::Usage
    })
}

fn command(name: &str, parser: &mut lexopt::Parser) -> Result<Command, Failure> {
    Ok(match name {
        "ingest" => then(cmd::ingest::parse(parser)?, cmd::ingest::run),
        // The whole history, once, without the shell waiting for it (ING-11).
        // It prints an estimate first and does the work in a detached process,
        // which is why it is a command of its own rather than a flag on
        // `ingest`: the two return at different times and mean different things.
        "backfill" => then(cmd::backfill::parse(parser)?, cmd::backfill::run),
        // Written into settings.json by `terminus install` and never typed by
        // hand. It writes nothing to stdout and exits 0 once its event name is
        // known, because a hook that fails is a hook that can block a prompt.
        "hook" => then(cmd::hook::parse(parser)?, cmd::hook::run),
        "search" => then(cmd::search::parse(parser)?, cmd::search::run),
        "show" => then(cmd::show::parse(parser)?, cmd::show::run),
        "sessions" => then(cmd::sessions::parse(parser)?, cmd::sessions::run),
        // D-24: the three commands phase 1 and 2 shipped take `--json` and
        // nothing else. `cmd::json_flag` is what replaced `no_more_arguments`
        // here - the rule that a subcommand rejects every argument it was not
        // written for still holds, and `--json` is now one it was.
        "verify" => then(cmd::json_flag(parser)?, cmd::verify::run),
        "reindex" => then(cmd::json_flag(parser)?, cmd::reindex::run),
        "status" => then(cmd::json_flag(parser)?, cmd::status::run),
        // RET-04. Reclaims what retention already freed and deletes nothing
        // itself: `VACUUM` then `PRAGMA wal_checkpoint(TRUNCATE)` (D-08), under
        // the ingest lock, because a plain `VACUUM` fails outright while a pass
        // holds a write transaction.
        "compact" => then(cmd::json_flag(parser)?, cmd::compact::run),
        // RET-05. Two tables that each reconcile to their own total (D-09):
        // archived blob bytes by project and by month, and the database file's
        // pages by table. Never one number for both - the file is 2.4x the
        // archive, and the difference is the derived tables.
        "usage" => then(cmd::json_flag(parser)?, cmd::usage::run),
        // PRIV-04. The transcripts as they arrived, one file per session, plus
        // a manifest that states in words that they are unredacted and what is
        // not in them. Redaction is keyed on destination and a directory the
        // user named is not egress (PRIV-01), so the statement is the answer.
        "export" => then(cmd::export::parse(parser)?, cmd::export::run),
        // FEED-04. A CLI data command and never an MCP tool (D-14): every
        // tool description sits in every session's context forever.
        "stats" => then(cmd::json_flag(parser)?, cmd::stats::run),
        // OBS-01. The facts come off the parser, never off a model: this reads
        // back a row the ingest pass wrote and opens no network connection on
        // any path.
        "observations" => then(cmd::observations::parse(parser)?, cmd::observations::run),
        // FEED-03. Read-only by construction (D-15) and the one command whose
        // flags move the injection thresholds - for this report and nothing
        // else: the live path compiles them in (D-08).
        "replay" => then(cmd::replay::parse(parser)?, cmd::replay::run),
        // RET-03. A report and never an action: retention acts inside the
        // ingest pass and nowhere else (D-04, D-10), so this shares that one
        // evaluation function and applies nothing it names.
        "retention" => then(cmd::retention::parse(parser)?, cmd::retention::run),
        // D-26: the MCP server is this binary, not a second artifact to place
        // and keep in version lockstep. It takes no arguments - a client spawns
        // it, nobody types it - and it writes JSON-RPC on stdout rather than
        // anything the `--json` contract above describes.
        "mcp" => then(cmd::mcp::parse(parser)?, cmd::mcp::run),
        // Read-only by construction (INST-06, D-12): it opens the store
        // through the read path, never `Store::open`, and creates no file and
        // no directory on any path it takes.
        "doctor" => then(cmd::json_flag(parser)?, cmd::doctor::run),
        // Human-only and deliberately not a `--json` data command (D-24): it
        // prints a receipt for a person to read.
        "install" => then(cmd::install::parse(parser)?, |()| cmd::install::run()),
        // Human-only and not a `--json` data command (D-24): it asks under
        // `--purge`. The only command in this binary that deletes anything: it
        // removes what install added, leaves the archive alone and prints its
        // path, and destroys it only under `--purge`, after showing the size
        // and asking (INST-07, D-13).
        "uninstall" => then(cmd::uninstall::parse(parser)?, cmd::uninstall::run),
        // STOR-07. Human-only for the third time and the same reason: it prints
        // what is about to move, warns about the one hazard nothing can detect
        // for the user (D-16) and asks once. The two-word form follows
        // `observations regenerate`, and the location it records is read by
        // `store::data_dir` and by nothing in settings.json (D-14).
        "data" => then(cmd::data::parse(parser)?, cmd::data::run),
        other => return Err(Failure::Misuse(format!("unknown command '{other}'"))),
    })
}

/// A command's `run`, holding the arguments its `parse` returned.
fn then<A: 'static>(args: A, run: fn(A) -> Result<(), Failure>) -> Command {
    Box::new(move || run(args))
}

pub(crate) fn unexpected(arg: lexopt::Arg<'_>) -> String {
    match arg {
        lexopt::Arg::Short(c) => format!("unexpected argument '-{c}'"),
        lexopt::Arg::Long(name) => format!("unexpected argument '--{name}'"),
        lexopt::Arg::Value(v) => format!("unexpected argument '{}'", v.to_string_lossy()),
    }
}

/// The parsers the fuzz targets in `fuzz/` call. `cargo fuzz` sets
/// `cfg(fuzzing)`; no other build contains this module.
#[cfg(fuzzing)]
pub mod fuzz {
    use std::ffi::OsString;
    use std::path::Path;

    use terminus_core::Config;

    use crate::cmd::{hook, install::json_file, mcp};

    /// Parse a command line as `terminus` would, without running the command.
    pub fn command_line(args: Vec<OsString>) {
        let _ = super::parse(&mut lexopt::Parser::from_args(args));
    }

    /// Parse one hook stdin line, then run `event`'s injection against the
    /// store in `data_dir`.
    pub fn hook(event: &str, line: &[u8], data_dir: &Path, config: &Config) -> Option<String> {
        let arm = hook::arm(event)?;
        arm(data_dir, config, &hook::Payload::parse(line).borrowed())
    }

    /// Parse one MCP message.
    pub fn mcp_message(line: &str) {
        let _ = mcp::rpc::parse(line);
    }

    /// Run the MCP server loop with `input` as its stdin.
    pub fn mcp_serve(input: &[u8]) {
        let _ = mcp::serve(input, std::io::sink());
    }

    /// Parse `text` as `settings.json` or `.claude.json`. Whatever parses must
    /// render to text that parses back to the same value, because install
    /// writes the rendered form over the user's file.
    pub fn settings(text: &str) {
        let Ok(value) = json_file::parse(text) else {
            return;
        };
        let rendered = json_file::render(&value, true);
        let again = json_file::parse(&rendered)
            .unwrap_or_else(|e| panic!("rendered settings do not parse: {e}\n{rendered}"));
        assert_eq!(again, value, "rendering changed the value");
    }
}
