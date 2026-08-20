//! `verbatim install`: one command wires verbatim into Claude Code.
//!
//! It does five things and refuses to do any of the first four halfway:
//!
//! 1. puts this build at the canonical stable path ([`binary`], D-08, INST-02),
//! 2. writes the four exec-form hook entries into `settings.json`
//!    ([`targets`], D-01),
//! 3. registers the MCP server in `.claude.json` ([`targets`], D-05),
//! 4. says what a low `cleanupPeriodDays` costs and what auto-compact costs, and
//!    changes neither by itself (INST-04),
//! 5. starts the backfill ([`start_backfill`], ING-11) - which is the one step
//!    that cannot fail the install, because by then everything above it has
//!    already succeeded.
//!
//! # Nothing is written before the answer (INST-03)
//!
//! Every refusable check runs first, both diffs are rendered from files nothing
//! has touched, and then one confirmation covers the lot - not one per file, and
//! never one after a write. The binary copy counts as a write and is ordered
//! with the rest. A declined confirmation exits 0 having changed nothing and
//! says so.
//!
//! The diff is the change, not the file: `.claude.json` is 240 KB, and a user
//! asked to approve two added lines must be shown two added lines.
//!
//! # `--yes`, and what happens with nothing to read (INST-08, D-20)
//!
//! `--yes` accepts every default and prompts for nothing, which is what makes
//! install runnable from a script. Without it, install asks and reads the
//! answer; if there is no answer to read - stdin at end of file, closed, or
//! unreadable - it refuses and names `--yes`, because a scripted install that
//! silently took defaults it never showed anyone is the failure this rule
//! exists to prevent. `std::io::IsTerminal` decides only how the question is
//! presented, and needs no dependency.
//!
//! `install` stays human-only with no `--json` (D-24): an interactive
//! confirmation and a single JSON document on stdout contradict each other, and
//! no INST requirement asks for one.

pub mod binary;
pub mod json_file;
pub mod targets;

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use verbatim_core::Config;

use json_file::{Document, Json};

use crate::cmd::Failure;

/// Below this many days, install says what Claude Code's cleanup costs.
///
/// A quarter of transcripts is the window in which a session verbatim has not
/// yet seen can still be recovered from Claude Code's own files. Below it, the
/// second recovery path is short enough to be worth a sentence.
const LOW_CLEANUP_DAYS: i64 = 90;

/// What Claude Code deletes after, when `cleanupPeriodDays` is not set at all.
const CLAUDE_DEFAULT_CLEANUP_DAYS: i64 = 30;

/// What install offers to raise it to: ten years, which is "keep them".
const KEEP_CLEANUP_DAYS: i64 = 3650;

pub struct Options {
    pub yes: bool,
}

/// A new file in `dir`, created exclusively, and the path it landed at.
///
/// Every atomic write install makes - the settings files, the backups, the copy
/// of this build - goes through a temporary file in the destination directory
/// and a rename. This is the one place those temporary files are created, and
/// `create_new` is the reason: it is `O_EXCL`, so it fails on anything already
/// at that name rather than opening it.
///
/// `File::create` would not. It follows symlinks and truncates what it finds, so
/// a stale `.settings.json.verbatim-<pid>` symlink left behind by a crashed run,
/// or a pid that has come round again, is enough to truncate an unrelated file
/// through it, and the rename that follows moves the *link* into place rather
/// than the file. Neither is atomic and neither is recoverable.
///
/// A collision is answered with a different name, never with a reuse: whatever
/// is at the first one, it is not ours.
pub(super) fn create_temporary(
    dir: &Path,
    prefix: &str,
) -> Result<(std::path::PathBuf, std::fs::File), Failure> {
    /// Enough to step over a stale name or two. Past it, something is wrong
    /// with the directory rather than with the name, and guessing again is not
    /// going to find out what.
    const ATTEMPTS: u32 = 64;

    let pid = std::process::id();
    for attempt in 0..ATTEMPTS {
        let path = match attempt {
            0 => dir.join(format!("{prefix}{pid}")),
            n => dir.join(format!("{prefix}{pid}-{n}")),
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(Failure::Operational(format!(
                    "{} could not be created: {e}",
                    path.display()
                )))
            }
        }
    }
    Err(Failure::Operational(format!(
        "{} already holds {ATTEMPTS} files named '{prefix}{pid}...'; verbatim will not write \
         over any of them. remove them and run install again",
        dir.display()
    )))
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Options, Failure> {
    use lexopt::prelude::*;

    let mut yes = false;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Short('y') | Long("yes") => yes = true,
            // Including `--json`, deliberately (D-24).
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }
    Ok(Options { yes })
}

pub fn run(options: Options) -> Result<(), Failure> {
    let stable = binary::stable_path()?;

    // Everything that can refuse, before anything that can write.
    if let binary::Occupant::Foreign = binary::occupant(&stable)? {
        return Err(occupied(&stable));
    }
    let config = Config::load()?;
    let data_dir = crate::cmd::data_dir()?;
    let settings = Document::read(&targets::settings_path()?)?;
    let claude = Document::read(&targets::claude_json_path()?)?;

    let add_hooks = |root: &mut Json| targets::add_hooks(root, &stable);
    let add_server = |root: &mut Json| targets::add_mcp_server(root, &stable);
    // Against the file's own bytes, not against a re-rendered copy of it: what
    // `apply` writes is the whole document, so a settings file this renderer did
    // not format is rewritten in full and the diff has to say so (INST-03).
    let hook_diff = pending(&settings, &settings.preview(&add_hooks)?);
    let server_diff = pending(&claude, &claude.preview(&add_server)?);

    println!("verbatim install");
    println!();
    println!("  this build  {}", current_exe_display());
    println!("  copied to   {}", stable.display());
    println!();
    show(
        &settings,
        &hook_diff,
        "the four hook entries are already there",
    );
    show(
        &claude,
        &server_diff,
        "the mcp server is already registered",
    );

    if !confirm("apply these changes?", true, options.yes)? {
        println!("nothing was changed.");
        return Ok(());
    }

    // From here on, and not one line earlier, install writes.
    binary::place(&stable)?;

    let mut backups = Vec::new();
    let mut wrote_hooks = false;
    if !hook_diff.is_empty() {
        if let Some(backup) = settings.backup()? {
            backups.push(backup);
        }
        wrote_hooks = settings.apply(&add_hooks)?;
    }
    let mut wrote_server = false;
    if !server_diff.is_empty() {
        if let Some(backup) = claude.backup()? {
            backups.push(backup);
        }
        wrote_server = claude.apply(&add_server)?;
    }

    advise(&settings, options.yes)?;

    println!();
    println!("installed");
    println!("  binary            {}", stable.display());
    println!(
        "  hook entries      {} ({})",
        settings.path().display(),
        outcome(wrote_hooks, "four entries added", "already current")
    );
    println!(
        "  mcp server        {} ({})",
        claude.path().display(),
        outcome(wrote_server, "registered", "already registered")
    );
    if backups.is_empty() {
        // True whichever way it got here: a file that did not exist has nothing
        // to copy, and a file that did not change is not backed up either.
        println!(
            "  backups           none: a backup is written for a file that existed and changed"
        );
    } else {
        for backup in &backups {
            println!("  backup            {}", backup.display());
        }
    }
    println!("  data directory    {}", data_dir.display());
    let roots = config.roots();
    println!(
        "  transcript roots  {} ({})",
        roots.len(),
        roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if roots.len() > 1 {
        // Hooks live in one settings file. The other roots are still ingested,
        // but nothing in them fires a hook.
        println!("                    only the first root's settings file carries the hooks");
    }
    println!("  undo              verbatim uninstall");
    println!();
    println!(
        "observed on the Claude Code versions on this machine: hook entries written into\n\
         settings.json are picked up by a session that is already running, with no restart.\n\
         That is measured behaviour rather than a documented contract."
    );

    // Step 6, and the last thing install does.
    start_backfill(&data_dir, &config);
    Ok(())
}

/// Start archiving the history that is already on disk, and return (ING-11).
///
/// The design brief's step 6, and deliberately the last thing install does: it
/// gates nothing above it, and it returns nothing. Both halves of that matter.
///
/// **It cannot fail an install.** By the time this runs the binary is placed and
/// both settings files are written, so the install has succeeded whatever
/// happens here - and the next hook that fires would archive the same tree
/// anyway, which is why a backfill that will not start is one reported line
/// rather than a non-zero exit. Returning `()` is how that is enforced rather
/// than remembered: there is no error for a caller to propagate by accident.
///
/// **It does not wait.** The work runs detached, so install returns while a
/// gigabyte is still being read. There is no log file by design, so the summary
/// has to say where to watch it - `verbatim status` reads the `runs` row the
/// pass writes.
fn start_backfill(data_dir: &Path, config: &Config) {
    use crate::cmd::backfill;

    println!();
    let estimate = match backfill::estimate(data_dir, config) {
        Ok(estimate) => estimate,
        Err(why) => {
            println!(
                "  problem: the backfill could not be sized ({}).",
                reason(why)
            );
            println!("  nothing else is affected; the next hook archives the same tree.");
            return;
        }
    };
    backfill::print_estimate(&estimate);

    match backfill::start(data_dir, &estimate) {
        Ok(true) => println!(
            "archiving that now, in a detached process - this install is done and the\n\
             archive fills in behind it. `verbatim status` is where to watch it: there is\n\
             no log file by design, and the pass writes a `runs` row this reads."
        ),
        // Nothing on disk to archive, so nothing was started and no store was
        // created by installing. The hooks take it from the first session.
        Ok(false) => println!(
            "nothing to archive yet - the hooks above start doing that from your next\n\
             session, and `verbatim status` is where to watch it."
        ),
        Err(why) => {
            println!("  problem: {}", reason(why));
        }
    }
}

/// A [`Failure`]'s text, for a line that reports rather than returns.
fn reason(failure: Failure) -> String {
    match failure {
        Failure::Operational(why) | Failure::Misuse(why) => why,
        Failure::Silent => "no reason was recorded".to_owned(),
    }
}

/// The advisories install gives and the settings it never changes (INST-04).
fn advise(settings: &Document, yes: bool) -> Result<(), Failure> {
    // Re-read: the file may be the one install just wrote, and it may have been
    // rewritten by Claude Code in between either way.
    let current = Document::read(settings.path())?;
    let configured = current
        .value()
        .get("cleanupPeriodDays")
        .and_then(Json::as_i64);
    let effective = configured.unwrap_or(CLAUDE_DEFAULT_CLEANUP_DAYS);

    println!();
    if effective < LOW_CLEANUP_DAYS {
        let how = match configured {
            Some(days) => format!("cleanupPeriodDays is {days}"),
            None => format!(
                "cleanupPeriodDays is unset, so Claude Code's default of \
                 {CLAUDE_DEFAULT_CLEANUP_DAYS} applies"
            ),
        };
        println!(
            "  {how}. Claude Code deletes its own transcripts that\n  \
             many days after they are written, and those files are verbatim's second\n  \
             recovery path: what is already archived stays, but a session verbatim never\n  \
             got to read goes with them."
        );
        if offer(
            &format!("  raise cleanupPeriodDays to {KEEP_CLEANUP_DAYS}?"),
            yes,
        ) {
            current.apply(&|root: &mut Json| {
                *root.entry("cleanupPeriodDays", Json::number(KEEP_CLEANUP_DAYS))? =
                    Json::number(KEEP_CLEANUP_DAYS);
                Ok(())
            })?;
            println!("  cleanupPeriodDays is now {KEEP_CLEANUP_DAYS}.");
        } else {
            println!("  cleanupPeriodDays left as it is.");
        }
    }
    println!(
        "  autoCompactEnabled is left as it is, and verbatim never sets it. Turning it\n  \
         off is the recommendation: compaction spends tokens summarizing context this\n  \
         store already holds losslessly, and a fresh session plus targeted recall beats\n  \
         a self-summarization."
    );
    Ok(())
}

/// The diff install will write, or nothing at all when it will write nothing.
///
/// Taken against the file on disk. The empty string means install has nothing
/// to add to this file, which is also the answer when the file's formatting is
/// not this renderer's: install re-renders a file only when it also has
/// something to put in it, so a formatting difference alone is not a change to
/// show or a file to back up.
fn pending(document: &Document, after: &str) -> String {
    if !document.would_write(after) {
        return String::new();
    }
    json_file::diff(document.source(), after)
}

fn show(document: &Document, diff: &str, unchanged: &str) {
    println!("  {}", document.path().display());
    if diff.is_empty() {
        println!("    no change: {unchanged}");
    } else {
        if document.reformats() {
            // The diff is large and every line of it is real: this file is not
            // written the way verbatim writes one, and verbatim writes the whole
            // document. Saying so is the difference between a confusing diff and
            // an informed answer.
            println!(
                "    note: this file is not formatted the way verbatim writes one, so applying\n    \
                 this rewrites all of it - two-space indented, keys and values unchanged and in\n    \
                 the order they are in now. every line below is part of that."
            );
        }
        print!("{diff}");
    }
    println!();
}

fn outcome(wrote: bool, yes: &str, no: &str) -> String {
    if wrote {
        yes.to_owned()
    } else {
        no.to_owned()
    }
}

fn current_exe_display() -> String {
    match std::env::current_exe() {
        Ok(path) => path.display().to_string(),
        Err(_) => "this build".to_owned(),
    }
}

/// Ask once, read the answer, and refuse rather than assume one.
///
/// `pub(super)` for `cmd::uninstall`, whose `--purge` asks the same question
/// about something it cannot undo. The rule that matters is the refusal when
/// there is no answer to read, and a second copy of it beside a second copy of
/// `--yes` and the terminal test is exactly how the two come apart.
pub(super) fn confirm(question: &str, default: bool, yes: bool) -> Result<bool, Failure> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    if yes {
        println!(
            "{question} {hint} {} (--yes)",
            if default { "y" } else { "n" }
        );
        return Ok(default);
    }

    let interactive = std::io::stdin().is_terminal();
    if interactive {
        print!("{question} {hint} ");
        let _ = std::io::stdout().flush();
    } else {
        println!("{question} {hint}");
    }

    let mut answer = String::new();
    let read = std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| unanswerable(&format!("stdin could not be read: {e}")))?;
    if read == 0 {
        return Err(unanswerable("stdin is at end of file"));
    }
    Ok(match answer.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "y" | "yes" => true,
        _ => false,
    })
}

/// The same question, asked after everything is already written.
///
/// No answer is the default here rather than a refusal. `advise` runs after the
/// binary and both settings files have been written, and an offer that could
/// return a failure would make a completed install exit non-zero - reporting
/// that nothing worked when everything did. The default is no change, so the
/// worst an unread answer costs is a setting left as the user had it.
fn offer(question: &str, yes: bool) -> bool {
    confirm(question, false, yes).unwrap_or(false)
}

fn unanswerable(why: &str) -> Failure {
    Failure::Operational(format!(
        "install needs an answer and {why}; nothing was changed.\n  \
         rerun with --yes to accept every default without being asked"
    ))
}

/// The stable path holds a program that is not a verbatim build (D-07).
///
/// Not hypothetical: as of 2026-08-13 `/home/john/.local/bin/verbatim` is a
/// 12 MB different program whose `--version` prints `verbatim 0.1.0`, byte for
/// byte this build's output. So the refusal is the only safe answer, and the
/// command that overrides it has to be the user's to type.
pub(super) fn occupied(stable: &Path) -> Failure {
    let removal = if cfg!(windows) {
        format!("del \"{}\"", stable.display())
    } else {
        format!("rm -f '{}'", stable.display())
    };
    Failure::Operational(format!(
        "{} is not a verbatim build; install will not overwrite it, and nothing was changed.\n  \
         that path is where every hook entry and the mcp registration point, so it has to hold\n  \
         this binary. to replace it, remove it and run install again:\n\n    \
         {removal} && {} install",
        stable.display(),
        current_exe_display()
    ))
}
