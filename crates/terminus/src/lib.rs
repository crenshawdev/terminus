//! The `terminus` binary's code, as a library.
//!
//! `main.rs` picks the allocator and calls [`main`]. The code lives here so the
//! fuzz targets in `fuzz/` can reach the parsers that read outside input, through
//! the [`fuzz`] module, which only `cargo fuzz` builds contain.
//!
//! Exit codes: 0 success, 1 operational failure, 2 misuse. Data goes to stdout,
//! errors go to stderr.

// Stdout goes through `outln!` and `out!` and stderr through `errln!`. All three
// escape archived controls and survive a closed pipe, and the print macros do
// neither, so both streams are denied them.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::process::ExitCode;

#[macro_use]
mod out;
mod cmd;
mod escape;

use cmd::json::Document;
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
                       export --stdout       stream the sessions named on stdin to stdout\n  \
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
    // set Claude Code kills a hook through. If it is, it has already
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
    match parse(&mut lexopt::Parser::from_env()) {
        Ok(Invocation::Version) => outln!("{}", env!("CARGO_PKG_VERSION")),
        Ok(Invocation::Usage) => outln!("{USAGE}"),
        Ok(Invocation::Command(Command { empty, run })) => {
            if let Err(failure) = run() {
                return ended(failure, empty);
            }
        }
        Err(failure) => return ended(failure, None),
    }
    ExitCode::SUCCESS
}

/// The exit code for a command that did not succeed, and what it writes first.
///
/// `empty` is the command's own empty document, held only when `--json` was
/// asked for. An operational failure under `--json` is that document with
/// `ok: false` and the reason, and nothing on stderr, so a caller reading
/// stdout gets the same shape whether or not the command got far enough to
/// answer (#107). Misuse writes no document: it is raised in a `parse` fn,
/// before the command's output mode is known.
fn ended(failure: Failure, empty: Option<Document>) -> ExitCode {
    match failure {
        Failure::Operational(msg) => {
            match empty {
                // A stdout that cannot be written still exits 1: the failure is
                // the answer, and a second write to a dead stdout fails the same
                // way, so the error has nowhere better to go.
                Some(document) => {
                    let _ = document.failed().because(msg).try_emit();
                }
                None => errln!("terminus: {msg}"),
            }
            ExitCode::from(1)
        }
        Failure::Silent => ExitCode::from(1),
        Failure::Misuse(msg) => {
            errln!("terminus: {msg}");
            errln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// What a command line asks for, parsed and not yet run.
enum Invocation {
    Version,
    Usage,
    Command(Command),
}

/// A subcommand whose arguments are parsed, waiting to run.
struct Command {
    /// The command's own empty document, built from its parsed arguments
    /// before `run` takes them. Held only when `--json` was asked for, and
    /// never for a command that has no `--json`.
    empty: Option<Document>,
    run: Box<dyn FnOnce() -> Result<(), Failure>>,
}

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
        // The whole history, once, without the shell waiting for it.
        // It prints an estimate first and does the work in a detached process,
        // which is why it is a command of its own rather than a flag on
        // `ingest`: the two return at different times and mean different things.
        "backfill" => then(cmd::backfill::parse(parser)?, cmd::backfill::run),
        // Written into settings.json by `terminus install` and never typed by
        // hand. It writes nothing to stdout and exits 0 once its event name is
        // known, because a hook that fails is a hook that can block a prompt.
        "hook" => then(cmd::hook::parse(parser)?, cmd::hook::run),
        "search" => then_json(
            cmd::search::parse(parser)?,
            |a| a.json,
            cmd::search::empty,
            cmd::search::run,
        ),
        "show" => then_json(
            cmd::show::parse(parser)?,
            |a| a.json,
            |_| cmd::show::empty(),
            cmd::show::run,
        ),
        "sessions" => then_json(
            cmd::sessions::parse(parser)?,
            |a| a.json,
            |_| cmd::sessions::empty(),
            cmd::sessions::run,
        ),
        // These three commands take `--json` and nothing else. `cmd::json_flag`
        // is what replaced `no_more_arguments` here - the rule that a subcommand
        // rejects every argument it was not written for still holds, and
        // `--json` is now one it was.
        "verify" => flagged(parser, cmd::verify::empty, cmd::verify::run)?,
        "reindex" => flagged(parser, cmd::reindex::empty, cmd::reindex::run)?,
        "status" => flagged(parser, cmd::status::empty, cmd::status::run)?,
        // Reclaims what retention already freed and deletes nothing itself:
        // `VACUUM` then `PRAGMA wal_checkpoint(TRUNCATE)`, under the ingest lock,
        // because a plain `VACUUM` fails outright while a pass holds a write
        // transaction.
        "compact" => flagged(parser, cmd::compact::empty, cmd::compact::run)?,
        // Two tables that each reconcile to their own total:
        // archived blob bytes by project and by month, and the database file's
        // pages by table. Never one number for both - the file is 2.4x the
        // archive, and the difference is the derived tables.
        "usage" => flagged(parser, cmd::usage::empty, cmd::usage::run)?,
        // The transcripts as they arrived, one file per session, plus
        // a manifest that states in words that they are unredacted and what is
        // not in them. Redaction is keyed on destination and a directory the
        // user named is not egress, so the statement is the answer.
        "export" => then_json(
            cmd::export::parse(parser)?,
            |a| a.json,
            cmd::export::empty,
            cmd::export::run,
        ),
        // A CLI data command and never an MCP tool: every
        // tool description sits in every session's context forever.
        "stats" => flagged(parser, cmd::stats::empty, cmd::stats::run)?,
        // The facts come off the parser, never off a model: this reads
        // back a row the ingest pass wrote and opens no network connection on
        // any path.
        "observations" => then_json(
            cmd::observations::parse(parser)?,
            cmd::observations::json,
            cmd::observations::empty,
            cmd::observations::run,
        ),
        // Read-only by construction and the one command whose flags move the
        // injection thresholds - for this report and nothing else: the live
        // path compiles them in.
        "replay" => then_json(
            cmd::replay::parse(parser)?,
            |a| a.json,
            cmd::replay::empty,
            cmd::replay::run,
        ),
        // A report and never an action: retention acts inside the ingest pass
        // and nowhere else, so this shares that one
        // evaluation function and applies nothing it names.
        "retention" => then_json(
            cmd::retention::parse(parser)?,
            |a| a.json,
            |_| cmd::retention::empty(),
            cmd::retention::run,
        ),
        // The MCP server is this binary, not a second artifact to place
        // and keep in version lockstep. It takes no arguments - a client spawns
        // it, nobody types it - and it writes JSON-RPC on stdout rather than
        // anything the `--json` contract above describes.
        "mcp" => then(cmd::mcp::parse(parser)?, cmd::mcp::run),
        // Read-only by construction: it opens the store
        // through the read path, never `Store::open`, and creates no file and
        // no directory on any path it takes.
        "doctor" => flagged(parser, cmd::doctor::empty, cmd::doctor::run)?,
        // Human-only and deliberately not a `--json` data command: it
        // prints a receipt for a person to read.
        "install" => then(cmd::install::parse(parser)?, |()| cmd::install::run()),
        // Human-only and not a `--json` data command: it asks under
        // `--purge`. The only command in this binary that deletes anything: it
        // removes what install added, leaves the archive alone and prints its
        // path, and destroys it only under `--purge`, after showing the size
        // and asking.
        "uninstall" => then(cmd::uninstall::parse(parser)?, cmd::uninstall::run),
        // Human-only for the third time and the same reason: it prints
        // what is about to move, warns about the one hazard nothing can detect
        // for the user and asks once. The two-word form follows
        // `observations regenerate`, and the location it records is read by
        // `store::data_dir` and by nothing in settings.json.
        "data" => then(cmd::data::parse(parser)?, cmd::data::run),
        other => return Err(Failure::Misuse(format!("unknown command '{other}'"))),
    })
}

/// A command's `run`, holding the arguments its `parse` returned. For a command
/// with no `--json`.
fn then<A: 'static>(args: A, run: fn(A) -> Result<(), Failure>) -> Command {
    Command {
        empty: None,
        run: Box::new(move || run(args)),
    }
}

/// The same for a command that takes `--json`. `json` reads the flag off the
/// parsed arguments, and `empty` builds the command's empty document from them.
/// `empty` is called only when the flag is set.
fn then_json<A: 'static>(
    args: A,
    json: fn(&A) -> bool,
    empty: impl FnOnce(&A) -> Document,
    run: fn(A) -> Result<(), Failure>,
) -> Command {
    Command {
        empty: json(&args).then(|| empty(&args)),
        run: Box::new(move || run(args)),
    }
}

/// [`then_json`] for the commands `cmd::json_flag` parses, whose only argument
/// is the flag itself.
fn flagged(
    parser: &mut lexopt::Parser,
    empty: fn() -> Document,
    run: fn(bool) -> Result<(), Failure>,
) -> Result<Command, Failure> {
    let json = cmd::json_flag(parser)?;
    Ok(then_json(json, |json| *json, |_| empty(), run))
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
