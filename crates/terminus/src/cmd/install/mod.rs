//! `terminus install`: wire terminus into Claude Code, asking nothing.
//!
//! It stores this build in the versions folder and points the stable path at it
//! with a link ([`binary`]), merges the four hook entries into `settings.json`
//! with a one-time backup ([`targets`]), and places the MCP entry by running
//! `claude mcp` ([`claude`]). The hook entries and the MCP entry name the
//! stable path, never a version file. It reads `.claude.json` and never writes
//! it, so Claude Code stays that file's only writer.
//!
//! Every check that can refuse runs before the first write. After that only the
//! registration can fall short. When it does, the rest still happens, the
//! receipt says the install is incomplete and prints the command that finishes
//! it, and install exits 1.
//!
//! It ends with a receipt of every file it touched, the stored version file
//! among them, and the two Claude Code
//! settings it reports and never changes, then starts the backfill last. It
//! never reads stdin and takes no flags, so `--yes` and `--json` are misuse.

pub mod binary;
pub mod claude;
pub mod json_file;
pub mod targets;

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use terminus_core::Config;

use json_file::{Document, Json};

use crate::cmd::Failure;

/// What Claude Code deletes after, when `cleanupPeriodDays` is not set at all.
const CLAUDE_DEFAULT_CLEANUP_DAYS: i64 = 30;

/// What the printed line raises it to: ten years, which is "keep them". Doctor
/// prints the same line.
const KEEP_CLEANUP_DAYS: i64 = 3650;

/// A new file in `dir`, created exclusively, and the path it landed at.
///
/// Every atomic write install makes - the settings files, the backups, the
/// stored version - goes through a temporary file in the destination directory
/// and a rename. This is the one place those temporary files are created, and
/// `create_new` is the reason: it is `O_EXCL`, so it fails on anything already
/// at that name rather than opening it.
///
/// `File::create` would not. It follows symlinks and truncates what it finds, so
/// a stale `.settings.json.terminus-<pid>` symlink left behind by a crashed run,
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
    temporary(dir, prefix, |path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    })
}

/// [`create_temporary`] for anything `create` makes at a name that must not
/// exist yet. The link install puts over the stable path is made this way,
/// because `symlink` also fails on a name that is taken.
pub(super) fn temporary<T>(
    dir: &Path,
    prefix: &str,
    create: impl Fn(&Path) -> std::io::Result<T>,
) -> Result<(std::path::PathBuf, T), Failure> {
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
        match create(&path) {
            Ok(made) => return Ok((path, made)),
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
        "{} already holds {ATTEMPTS} files named '{prefix}{pid}...'; terminus will not write \
         over any of them. remove them and run install again",
        dir.display()
    )))
}

/// Install takes no arguments. Anything given is misuse, `--json` (D-24) and
/// `--yes` (D-08) included.
pub fn parse(parser: &mut lexopt::Parser) -> Result<(), Failure> {
    match parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        Some(other) => Err(Failure::Misuse(crate::unexpected(other))),
        None => Ok(()),
    }
}

pub fn run() -> Result<(), Failure> {
    let stable = binary::stable_path()?;

    // Everything that can refuse, before anything that can write.
    if let binary::Occupant::Foreign = binary::occupant(&stable)? {
        return Err(occupied(&stable));
    }
    let versions = binary::versions_dir()?;
    let config = Config::load()?;
    let data_dir = crate::cmd::data_dir()?;
    let settings = Document::read(&targets::settings_path()?)?;
    // Only where it is. What is in it is read after the writes above it, and a
    // file that cannot be read leaves the registration incomplete rather than
    // refusing the install (phase 3 D-05).
    let claude_json = targets::claude_json_path()?;

    let add_hooks = |root: &mut Json| targets::add_hooks(root, &stable);
    // A formatting difference alone is not a reason to write or back up: the
    // file is re-rendered only when install also has something to add to it.
    let hooks_pending = settings.would_write(&settings.preview(&add_hooks)?);

    // From here on, and not one line earlier, install writes.
    let version_file = binary::place(&stable, &versions)?;

    let mut backups = Vec::new();
    let mut wrote_hooks = false;
    if hooks_pending {
        if let Some(backup) = settings.backup()? {
            backups.push(backup);
        }
        wrote_hooks = settings.apply(&add_hooks)?;
    }
    let registered = register(&claude_json, &stable);

    advise(&settings)?;

    println!();
    match &registered {
        Ok(_) => println!("installed"),
        Err(_) => println!("install incomplete: the mcp server is not registered"),
    }
    println!(
        "  binary            {} (a link to {})",
        stable.display(),
        version_file.display()
    );
    println!(
        "  hook entries      {} ({})",
        settings.path().display(),
        outcome(wrote_hooks, "four entries added", "already current")
    );
    if wrote_hooks && settings.reformats() {
        // The file was not in the two-space form this renderer writes, so the
        // write re-rendered all of it (D-12).
        println!(
            "                    rewritten in full in two-space form, keys and values unchanged"
        );
        match backups.first() {
            Some(backup) => println!(
                "                    the original bytes are in {}",
                backup.display()
            ),
            None => println!("                    no backup was written"),
        }
    }
    match &registered {
        Ok(done) => println!(
            "  mcp server        {} ({})",
            claude_json.display(),
            done.describe()
        ),
        Err(incomplete) => {
            println!(
                "  mcp server        {} (not registered: {})",
                claude_json.display(),
                incomplete.cause
            );
            println!("  to finish, run    {}", incomplete.command);
        }
    }
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
    println!("  undo              terminus uninstall");
    println!();
    println!(
        "observed on the Claude Code versions on this machine: hook entries written into\n\
         settings.json are picked up by a session that is already running, with no restart.\n\
         That is measured behaviour rather than a documented contract."
    );

    // Step 6, and the last thing install does.
    start_backfill(&stable, &data_dir, &config);
    match registered {
        Ok(_) => Ok(()),
        // Silent: the summary above already says what is missing and the
        // command that finishes it. An incomplete install never exits 0,
        // because a script reads the status (D-05).
        Err(_) => Err(Failure::Silent),
    }
}

/// What the registration step did.
enum Registered {
    /// The entry was already current, so no `claude` ran.
    Current,
    /// `claude mcp add-json` placed it.
    Added,
    /// `claude mcp remove` took out an entry in the wrong shape, and the add
    /// placed it again.
    Replaced,
}

impl Registered {
    fn describe(&self) -> &'static str {
        match self {
            Registered::Current => "already registered, no claude command run",
            Registered::Added => "registered with claude mcp add-json",
            Registered::Replaced => {
                "an entry in another shape removed with claude mcp remove, then registered with \
                 claude mcp add-json"
            }
        }
    }
}

/// A registration install could not complete (D-05): why, and the command
/// that finishes it.
struct Incomplete {
    cause: String,
    command: String,
}

/// Place the MCP entry through `claude mcp` (phase 3 D-01, D-06).
///
/// `.claude.json` is read and never written. An entry already current runs
/// nothing, and `claude mcp get` is never run: it health-checks the server and
/// writes into the file. No entry gets the add. An entry in any other shape
/// gets the remove and then the add, because `add-json` refuses a name that
/// exists; a remove that fails stops the add. A file that cannot be read runs
/// no `claude` at all.
fn register(path: &Path, stable: &Path) -> Result<Registered, Incomplete> {
    let document = Document::read(path).map_err(|failure| Incomplete {
        cause: reason(failure),
        command: targets::finishing_command(None, stable),
    })?;
    let found = document
        .value()
        .get("mcpServers")
        .and_then(|servers| servers.get(targets::MCP_SERVER_KEY));
    if found.is_some_and(|found| targets::mcp_entry_is_current(found, stable)) {
        return Ok(Registered::Current);
    }
    let entry = targets::mcp_entry(found, stable);
    let key = targets::MCP_SERVER_KEY;
    let mut registered = Registered::Added;
    if found.is_some() {
        if let Some(cause) = claude::run(&["mcp", "remove", "--scope", "user", key]).cause() {
            return Err(Incomplete {
                cause,
                command: targets::finishing_command(found, stable),
            });
        }
        registered = Registered::Replaced;
    }
    let compact = entry.compact();
    if let Some(cause) = claude::run(&["mcp", "add-json", "--scope", "user", key, &compact]).cause()
    {
        // Whatever was under the key is gone by now, so the add alone finishes
        // it.
        return Err(Incomplete {
            cause,
            command: targets::add_command(&entry),
        });
    }
    Ok(registered)
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
/// has to say where to watch it - `terminus status` reads the `runs` row the
/// pass writes.
fn start_backfill(stable: &Path, data_dir: &Path, config: &Config) {
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

    // The stable path, not this process: it is the build every hook entry runs.
    // This process may be a build outside the versions folder, such as
    // `target/release/terminus`, which the next `cargo` run replaces.
    match backfill::start_from(Some(stable), data_dir, &estimate) {
        Ok(true) => println!(
            "archiving that now, in a detached process - this install is done and the\n\
             archive fills in behind it. `terminus status` is where to watch it: there is\n\
             no log file by design, and the pass writes a `runs` row this reads."
        ),
        // Nothing on disk to archive, so nothing was started and no store was
        // created by installing. The hooks take it from the first session.
        Ok(false) => println!(
            "nothing to archive yet - the hooks above start doing that from your next\n\
             session, and `terminus status` is where to watch it."
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
fn advise(settings: &Document) -> Result<(), Failure> {
    // Re-read: the file may be the one install just wrote, and it may have been
    // rewritten by Claude Code in between either way.
    let current = Document::read(settings.path())?;
    let configured = current
        .value()
        .get("cleanupPeriodDays")
        .and_then(Json::as_i64);
    let effective = configured.unwrap_or(CLAUDE_DEFAULT_CLEANUP_DAYS);

    let how = match configured {
        Some(days) => format!("cleanupPeriodDays is {days}"),
        None => format!(
            "cleanupPeriodDays is unset, so Claude Code's default of \
             {CLAUDE_DEFAULT_CLEANUP_DAYS} applies"
        ),
    };

    println!();
    println!(
        "  {how}.\n  \
         Claude Code deletes its own transcripts that many days after they are written,\n  \
         so its transcripts older than {effective} days are already gone. What terminus\n  \
         has archived stays. install leaves the setting as it is. To keep them longer:"
    );
    println!(
        "    set \"cleanupPeriodDays\": {KEEP_CLEANUP_DAYS} in {}",
        settings.path().display()
    );
    println!(
        "  autoCompactEnabled is left as it is, and terminus never sets it. Turning it\n  \
         off is the recommendation: compaction spends tokens summarizing context this\n  \
         store already holds losslessly, and a fresh session plus targeted recall beats\n  \
         a self-summarization."
    );
    Ok(())
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
/// The shared question for `uninstall --purge` and `data move`. Install asks
/// nothing. Under `yes` it answers with `default`. With no answer to read -
/// stdin at end of file, closed, or unreadable - it fails rather than assume
/// one, and both callers replace that failure with their own wording.
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
        .map_err(|e| Failure::Operational(format!("no answer could be read: {e}")))?;
    if read == 0 {
        return Err(Failure::Operational(
            "no answer could be read: stdin is at end of file".to_owned(),
        ));
    }
    Ok(match answer.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "y" | "yes" => true,
        _ => false,
    })
}

/// The stable path holds a program that is not a terminus build (D-07).
///
/// Not hypothetical: as of 2026-08-13 `/home/john/.local/bin/verbatim`, the
/// stable path before the rename, was a 12 MB different program whose
/// `--version` printed `verbatim 0.1.0`, byte for byte this build's output then. So the refusal is the only safe answer, and the
/// command that overrides it has to be the user's to type.
pub(super) fn occupied(stable: &Path) -> Failure {
    Failure::Operational(format!(
        "{} is not a terminus build; install will not overwrite it, and nothing was changed.\n  \
         that path is where every hook entry and the mcp registration point, so it has to hold\n  \
         this binary. to replace it, remove it and run install again:\n\n    \
         rm -f '{}' && {} install",
        stable.display(),
        stable.display(),
        current_exe_display()
    ))
}
