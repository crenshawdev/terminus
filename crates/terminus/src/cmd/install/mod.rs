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
use std::path::{Path, PathBuf};

use terminus_core::Config;

use json_file::{Document, Json};

use crate::cmd::Failure;

/// What Claude Code deletes after, when `cleanupPeriodDays` is not set at all.
/// Doctor reads this definition too (#107).
pub(super) const CLAUDE_DEFAULT_CLEANUP_DAYS: i64 = 30;

/// What the printed line raises it to: ten years, which is "keep them". Doctor
/// prints the same line from this definition.
pub(super) const KEEP_CLEANUP_DAYS: i64 = 3650;

/// A new owner-only file in `dir`, and the path it landed at.
///
/// Every atomic write install makes - the settings files, the backups, the
/// stored version - goes through a temporary in the destination directory and a
/// rename, and this is the one place those temporaries are made. The create is
/// `owner_only::create_temporary`, which says why it is exclusive and why it is
/// 0600 from the open. What install adds is the names, `{prefix}{pid}` and then
/// `{prefix}{pid}-{n}`, kept because the stale-name test plants exactly
/// `.terminus-install-<pid>` and `.settings.json.terminus-<pid>`, and 64
/// attempts, enough to step over a stale name or two. Past that, something is
/// wrong with the directory rather than with the name.
///
/// A caller that needs another final mode sets it before the rename, as
/// `binary::copy_into` sets 0755.
pub(super) fn create_temporary(
    dir: &Path,
    prefix: &str,
) -> Result<(std::path::PathBuf, std::fs::File), Failure> {
    terminus_core::owner_only::create_temporary(dir, ATTEMPTS, temporary_name(prefix))
        .map_err(|e| unmade(dir, prefix, &e))
}

/// [`create_temporary`] for a symlink to `target`, which is how the link install
/// puts over the stable path is made. The names and the attempts are the same.
pub(super) fn create_temporary_symlink(
    dir: &Path,
    prefix: &str,
    target: &Path,
) -> Result<std::path::PathBuf, Failure> {
    terminus_core::owner_only::create_temporary_symlink(
        dir,
        ATTEMPTS,
        temporary_name(prefix),
        target,
    )
    .map_err(|e| unmade(dir, prefix, &e))
}

const ATTEMPTS: u32 = 64;

/// `{prefix}{pid}` for the first attempt and `{prefix}{pid}-{n}` after it.
fn temporary_name(prefix: &str) -> impl Fn(u32) -> String + '_ {
    let pid = std::process::id();
    move |attempt| match attempt {
        0 => format!("{prefix}{pid}"),
        n => format!("{prefix}{pid}-{n}"),
    }
}

/// What a failed temporary create says: every name taken, or something else.
fn unmade(dir: &Path, prefix: &str, error: &std::io::Error) -> Failure {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return Failure::Operational(format!(
            "{} already holds {ATTEMPTS} files named '{prefix}{}...'; terminus will not write \
             over any of them. remove them and run install again",
            dir.display(),
            std::process::id()
        ));
    }
    Failure::Operational(format!("{} could not be created: {error}", dir.display()))
}

/// Install takes no arguments. Anything given is misuse, `--json` and `--yes`
/// included.
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
    // refusing the install.
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

    outln!();
    match &registered {
        Ok(_) => outln!("installed"),
        Err(_) => outln!("install incomplete: the mcp server is not registered"),
    }
    outln!(
        "  binary            {} (a link to {})",
        stable.display(),
        version_file.display()
    );
    outln!(
        "  hook entries      {} ({})",
        settings.path().display(),
        outcome(wrote_hooks, "four entries added", "already current")
    );
    if wrote_hooks && settings.reformats() {
        // The file was not in the two-space form this renderer writes, so the
        // write re-rendered all of it.
        outln!(
            "                    rewritten in full in two-space form, keys and values unchanged"
        );
        match backups.first() {
            Some(backup) => outln!(
                "                    the original bytes are in {}",
                backup.display()
            ),
            None => outln!("                    no backup was written"),
        }
    }
    match &registered {
        Ok(done) => outln!(
            "  mcp server        {} ({})",
            claude_json.display(),
            done.describe()
        ),
        Err(incomplete) => {
            outln!(
                "  mcp server        {} (not registered: {})",
                claude_json.display(),
                incomplete.cause
            );
            outln!("  to finish, run    {}", incomplete.command);
        }
    }
    if backups.is_empty() {
        // True whichever way it got here: a file that did not exist has nothing
        // to copy, and a file that did not change is not backed up either.
        outln!("  backups           none: a backup is written for a file that existed and changed");
    } else {
        for backup in &backups {
            outln!("  backup            {}", backup.display());
        }
    }
    outln!("  data directory    {}", data_dir.display());
    let roots = config.roots();
    outln!(
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
        outln!("                    only the first root's settings file carries the hooks");
    }
    outln!("  undo              terminus uninstall");
    outln!();
    outln!(
        "observed on the Claude Code versions on this machine: hook entries written into\n\
         settings.json are picked up by a session that is already running, with no restart.\n\
         That is measured behaviour rather than a documented contract."
    );

    // Step 6, and the last thing install does.
    let refused = start_backfill(&stable, &data_dir, &config);
    match registered {
        // Silent for the same reason as the arm below: the refusal is printed
        // with its steps. `install.sh` runs this last and returns its status,
        // so a store no hook can archive into fails the script.
        Ok(_) if refused => Err(Failure::Silent),
        Ok(_) => Ok(()),
        // Silent: the summary above already says what is missing and the
        // command that finishes it. An incomplete install never exits 0,
        // because a script reads the status.
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

/// A registration install could not complete: why, and the command
/// that finishes it.
struct Incomplete {
    cause: String,
    command: String,
}

/// Place the MCP entry through `claude mcp`.
///
/// `.claude.json` is read and never written. An entry already current runs
/// nothing, and `claude mcp get` is never run: it health-checks the server and
/// writes into the file. No entry gets the add. An entry in any other shape
/// gets the remove and then the add, because `add-json` refuses a name that
/// exists; a remove that fails stops the add. A file that cannot be read runs
/// no `claude` at all.
fn register(path: &Path, stable: &Path) -> Result<Registered, Incomplete> {
    let document = Document::read(path).map_err(|failure| Incomplete {
        cause: failure.text(),
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

/// Start archiving the history that is already on disk, and return.
///
/// The design brief's step 6, and deliberately the last thing install does: it
/// gates nothing above it, and it returns nothing. Both halves of that matter.
///
/// **A backfill problem never fails an install, with one exception.** By the
/// time this runs the binary is placed and both settings files are written, and
/// the next hook that fires would archive the same tree anyway, so a backfill
/// that will not start is one reported line rather than a non-zero exit. The
/// exception is a store in an archive format this build refuses: no
/// hook will archive anything into it, so the refusal is printed with its
/// clean-install steps and this returns `true`, which `run` turns into a
/// non-zero exit. Returning a plain flag rather than an error keeps every other
/// problem from being propagated by accident.
///
/// **It does not wait.** The work runs detached, so install returns while a
/// gigabyte is still being read. There is no log file by design, so the summary
/// has to say where to watch it - `terminus status` reads the `runs` row the
/// pass writes.
fn start_backfill(stable: &Path, data_dir: &Path, config: &Config) -> bool {
    use crate::cmd::backfill;

    outln!();
    // Asked of the core directly, where the error is still typed: a `Failure`
    // carries only text. A store that is not there yet is not refused.
    let rebuilds = match terminus_core::Store::open_read_only(data_dir) {
        Err(refused @ terminus_core::Error::ArchiveFormatTooOld { .. }) => {
            outln!("  problem: {refused}");
            return true;
        }
        Ok(store) => store.predates_this_build(),
        Err(_) => false,
    };
    let estimate = match backfill::estimate(data_dir, config) {
        Ok(estimate) => estimate,
        Err(why) => {
            outln!(
                "  problem: the backfill could not be sized ({}).",
                why.text()
            );
            outln!("  nothing else is affected; the next hook archives the same tree.");
            return false;
        }
    };
    backfill::print_estimate(&estimate);

    // The stable path, not this process: it is the build every hook entry runs.
    // This process may be a build outside the versions folder, such as
    // `target/release/terminus`, which the next `cargo` run replaces.
    let started = match backfill::start_from(Some(stable), data_dir, &estimate) {
        Ok(started) => started,
        Err(why) => {
            outln!("  problem: {}", why.text());
            false
        }
    };
    if started {
        outln!(
            "archiving that now, in a detached process - this install is done and the\n\
             archive fills in behind it. `terminus status` is where to watch it: there is\n\
             no log file by design, and the pass writes a `runs` row this reads."
        );
    } else if estimate.transcripts == 0 {
        // Nothing on disk to archive, so nothing was started and no store was
        // created by installing. The hooks take it from the first session.
        outln!(
            "nothing to archive yet - the hooks above start doing that from your next\n\
             session, and `terminus status` is where to watch it."
        );
    }
    if rebuilds {
        warn_of_rebuild(started);
    }
    false
}

/// An update's one surprise, said last so it's what's on screen: the hooks stay
/// silent until the rebuild is done, and nothing else would tell you why.
fn warn_of_rebuild(started: bool) {
    let when = if started {
        "that has started\n  in the background"
    } else {
        "that starts with\n  your next session"
    };
    outln!();
    outln!(
        "  this version rebuilds your archive's search tables, once, and {when}. It took about 4 minutes on a 3 GB archive.\n  \
         until it finishes, new sessions get no brief, prompts get no earlier turns, and\n  \
         search results may be incomplete. your archived sessions aren't touched.\n  \
         `terminus status` stops saying the tables predate this build when it's done."
    );
}

/// The advisories install gives and the settings it never changes.
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

    outln!();
    outln!(
        "  {how}.\n  \
         Claude Code deletes its own transcripts once they are older than that, in a\n  \
         sweep after a session starts, so any older than {effective} days may already be gone.\n  \
         What terminus has archived stays. install leaves the setting as it is. To keep\n  \
         them longer:"
    );
    outln!(
        "    set \"cleanupPeriodDays\": {KEEP_CLEANUP_DAYS} in {}",
        settings.path().display()
    );
    outln!(
        "  autoCompactEnabled is left as it is, and terminus never sets it. Either value\n  \
         has a cost. On, Claude Code summarizes older history as context fills, which\n  \
         spends tokens and loses detail. Off, a session that reaches the context limit\n  \
         stops with an error until you run /compact or /clear. The archive keeps the\n  \
         full history either way but does not manage active context: recall offers at\n  \
         most three matching turns on a prompt."
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
    resolved_display(std::env::current_exe())
}

/// On macOS `current_exe` is the path the program was started by, so run as
/// `~/.local/bin/terminus` it is the link, and a command naming it would
/// delete the link and then run it. Resolved, it names this build's file.
fn resolved_display(exe: std::io::Result<PathBuf>) -> String {
    match exe.and_then(std::fs::canonicalize) {
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
        outln!(
            "{question} {hint} {} (--yes)",
            if default { "y" } else { "n" }
        );
        return Ok(default);
    }

    let interactive = std::io::stdin().is_terminal();
    if interactive {
        out!("{question} {hint} ");
        let _ = std::io::stdout().flush();
    } else {
        outln!("{question} {hint}");
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

/// The stable path holds a program that is not a terminus build.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Handed the link, as macOS's `current_exe` hands it over, the refusal
    /// names the file behind it, so its `rm -f` never removes what it runs.
    #[test]
    fn the_refusal_names_the_file_behind_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("terminus");
        std::fs::write(&file, b"").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();

        let shown = resolved_display(Ok(link));

        assert_eq!(shown, file.canonicalize().unwrap().display().to_string());
    }

    /// A temporary is 0600 before a byte is in it, whatever the umask.
    #[test]
    fn a_temporary_is_owner_only_from_the_open() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let probe = dir.path().join("probe");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .unwrap();
        let umask_mode = std::fs::metadata(&probe).unwrap().permissions().mode() & 0o777;
        if umask_mode & 0o077 == 0 {
            // A umask of 077 makes every new file owner-only, so the check
            // below would pass without the fix.
            println!("skipped: this process's umask already gives new files {umask_mode:o}");
            return;
        }

        let (_path, file) = create_temporary(dir.path(), ".temporary.").unwrap();

        let mode = file.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the temporary was opened at {mode:o}");
    }
}
