//! `verbatim uninstall`: take out exactly what install put in, and nothing
//! else (INST-07, D-13).
//!
//! # What "only what install added" means
//!
//! Every hook entry whose `command` is the stable path, wherever it sits: an
//! entry is verbatim's when its command is that path and is not read past that
//! field otherwise, which is install's own rule for ownership read backwards.
//! The walk is over the event keys the file *has* rather than the four this
//! build writes, so an entry left by an older verbatim under an event this one
//! no longer uses is still verbatim's and still goes. Plus the `verbatim` key
//! under `.mcpServers`, and nothing else in either file.
//!
//! A group left with no entries goes with them - a `{"matcher": "", "hooks":
//! []}` install created is not a thing to leave behind - and an event key left
//! with no groups goes only when install's backup proves the key was not there
//! before. Measured 2026-08-13, a real `settings.json` carries
//! `"SessionStart": []` written by Claude Code itself, so an empty array is not
//! evidence of anything on its own. No backup means no proof, and no proof
//! means the key stays.
//!
//! Never a whole-object replacement: `settings.json` carries 29 top-level keys
//! including the `theme`, `model` and `statusLine` Claude Code's own UI writes,
//! and `.claude.json` carries 240 KB of the user's project history.
//!
//! # The restore, and when it is refused (AC7)
//!
//! After the removal, the file is compared with install's backup - as values,
//! by rendering both, so formatting alone never decides it. Equal means nothing
//! but verbatim's entries has changed since install, and the backup's *bytes*
//! go back: the user's own formatting and key order, exactly as they were.
//! Different means the user has edited it since - a `/config` change to `theme`
//! is what that looks like - and then the surgically edited file stands and the
//! backup stays where it is. Uninstall says which of the two happened, because
//! they leave the machine in different states.
//!
//! # Partly broken is ordinary
//!
//! A settings file that is gone, a backup that is gone, an entry somebody
//! already removed by hand, a stable path holding somebody else's program:
//! each is one reported line and uninstall carries on to the rest. A user
//! reaching for uninstall is often reaching for it *because* something is
//! wrong, and a cleanup that aborts on the first surprise leaves them worse off
//! than when they started.
//!
//! # The archive is never collateral (INST-07)
//!
//! The data directory is left alone and its path is printed. `--purge` is the
//! only thing in this phase that destroys anything the user cannot get back, so
//! it shows the path and the size first, asks once through install's own
//! confirmation - same `--yes`, same refusal when there is no answer to read -
//! and takes the ingest lock before it deletes, so it can never pull the store
//! out from under a pass that is mid-transaction.
//!
//! Human-only, no `--json` (D-24).

use std::cell::Cell;
use std::path::Path;

use verbatim_core::ingest::lock;
use verbatim_core::store::DB_FILE_NAME;

use super::install::json_file::{Document, Json};
use super::install::{self, binary, targets};
use super::Failure;

pub struct Options {
    pub yes: bool,
    pub purge: bool,
}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Options, Failure> {
    use lexopt::prelude::*;

    let mut options = Options {
        yes: false,
        purge: false,
    };
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Short('y') | Long("yes") => options.yes = true,
            Long("purge") => options.purge = true,
            // Including `--json`, deliberately (D-24).
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }
    Ok(options)
}

pub fn run(options: Options) -> Result<(), Failure> {
    let stable = binary::stable_path()?;
    let data_dir = super::data_dir()?;

    println!("verbatim uninstall");
    println!();

    // Each of these reports its own lines and returns whether it did what it
    // set out to do. They are ordered the way install writes them, and one
    // failing never stops the next: a half-wired machine is exactly the one
    // this command exists for.
    let mut ok = clean(&targets::settings_path()?, &stable, Part::Hooks);
    ok &= clean(&targets::claude_json_path()?, &stable, Part::McpServer);
    ok &= unplace(&stable);

    println!("  data directory  {}", data_dir.display());
    if !options.purge {
        println!(
            "    kept: the archive is the product, and uninstall never deletes it.\n    \
             to delete it too: verbatim uninstall --purge"
        );
        println!();
        return finish(ok);
    }
    println!();
    purge(&data_dir, options.yes)?;
    println!();
    finish(ok)
}

fn finish(ok: bool) -> Result<(), Failure> {
    if ok {
        return Ok(());
    }
    // Silent: every line that did not go as it should is already in the report
    // above, and a trailing "verbatim: ..." would be a second account of the
    // same thing.
    Err(Failure::Silent)
}

/// Which of install's two edits this file holds.
///
/// Not both against both files: install writes hook entries to `settings.json`
/// and the server registration to `.claude.json` (D-05), and uninstall removes
/// what install added rather than searching each file for the other's shape.
#[derive(Clone, Copy)]
enum Part {
    Hooks,
    McpServer,
}

impl Part {
    fn what(self, count: usize) -> String {
        match self {
            Part::Hooks => format!("{count} hook entr{}", if count == 1 { "y" } else { "ies" }),
            Part::McpServer => format!("the mcp registration '{}'", targets::MCP_SERVER_KEY),
        }
    }

    fn nothing(self) -> &'static str {
        match self {
            Part::Hooks => "no hook entry of verbatim's was there",
            Part::McpServer => "no mcp registration of verbatim's was there",
        }
    }
}

/// One file: take verbatim's entries out, then decide between the edited file
/// and install's backup.
fn clean(path: &Path, stable: &Path, part: Part) -> bool {
    println!("  {}", path.display());
    let stable = stable.display().to_string();

    let document = match Document::read(path) {
        Ok(document) => document,
        Err(failure) => {
            println!("    left alone: {}", detail(failure));
            return false;
        }
    };
    if !document.exists() {
        println!("    not there: nothing to remove");
        return true;
    }

    // Install's backup decides two things: whether an empty container was
    // install's to drop, and whether this file goes back to its pre-install
    // bytes. A backup that is gone or unreadable is neither an error nor a
    // reason to stop - it means uninstall has nothing to prove either with.
    let backup_path = document.backup_path().ok();
    let backup = backup_path
        .as_ref()
        .and_then(|path| Document::read(path).ok())
        .filter(Document::exists);

    let removed = Cell::new(0usize);
    let before = backup.as_ref().map(Document::value);
    let edit = |root: &mut Json| strip(root, part, &stable, before, &removed);
    if let Err(failure) = document.apply(&edit) {
        println!("    left alone: {}", detail(failure));
        return false;
    }
    let Some(count) = std::num::NonZeroUsize::new(removed.get()) else {
        println!("    {}", part.nothing());
        // Nothing changed, so there is no restore to decide - but a backup
        // still sitting there is a file the user did not put there, and saying
        // where it is beats leaving it to be found.
        if let (Some(path), Some(_)) = (&backup_path, &backup) {
            println!(
                "    install's backup is still beside it at {}",
                path.display()
            );
        }
        return true;
    };
    println!("    removed {}", part.what(count.get()));

    restore(path, backup_path.as_deref(), backup)
}

/// Put install's backup back when nothing else has changed since (AC7).
///
/// The comparison is of *values*: both sides are re-rendered and the strings
/// compared, so a file that differs from the backup only in the formatting this
/// renderer imposed still counts as unchanged and still gets its original bytes
/// back. Anything the user actually edited shows up as a difference and stops
/// the restore.
fn restore(path: &Path, backup_path: Option<&Path>, backup: Option<Document>) -> bool {
    let (Some(backup_path), Some(backup)) = (backup_path, backup) else {
        println!("    no backup from install here, so there is nothing to put back");
        return true;
    };
    // Fresh, because the removal above just wrote it - and because Claude Code
    // may have written it since (D-06).
    let current = match Document::read(path) {
        Ok(current) => current,
        Err(failure) => {
            println!(
                "    backup kept at {}: {}",
                backup_path.display(),
                detail(failure)
            );
            return false;
        }
    };
    if current.rendered() != backup.rendered() {
        println!(
            "    this file has changed since install, so the edit above stands.\n    \
             the pre-install copy is still at {}",
            backup_path.display()
        );
        return true;
    }
    if let Err(failure) = current.overwrite(backup.source()) {
        println!(
            "    backup kept at {}: {}",
            backup_path.display(),
            detail(failure)
        );
        return false;
    }
    if let Err(e) = std::fs::remove_file(backup_path) {
        println!(
            "    restored to its pre-install bytes; {} could not be removed: {e}",
            backup_path.display()
        );
        return false;
    }
    println!("    restored to its pre-install bytes, and the backup is gone");
    true
}

/// Take verbatim's entries out of one parsed document.
fn strip(
    root: &mut Json,
    part: Part,
    stable: &str,
    before: Option<&Json>,
    removed: &Cell<usize>,
) -> Result<(), Failure> {
    match part {
        Part::Hooks => strip_hooks(root, stable, before, removed),
        Part::McpServer => strip_server(root, before, removed),
    }
}

fn strip_hooks(
    root: &mut Json,
    stable: &str,
    before: Option<&Json>,
    removed: &Cell<usize>,
) -> Result<(), Failure> {
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(());
    };
    let events: Vec<String> = hooks.keys().iter().map(|key| (*key).to_owned()).collect();
    let mut emptied = Vec::new();
    for event in &events {
        let Some(Json::Array(groups)) = hooks.get_mut(event) else {
            // Not an array of groups, so not a shape verbatim wrote or reads.
            // Uninstall is not the command that rewrites a malformed file.
            continue;
        };
        let mut kept = Vec::with_capacity(groups.len());
        for mut group in std::mem::take(groups) {
            let mut ours_left_it_empty = false;
            if let Some(Json::Array(entries)) = group.get_mut("hooks") {
                let was = entries.len();
                entries.retain(|entry| {
                    entry.get("command").and_then(Json::as_str).as_deref() != Some(stable)
                });
                removed.set(removed.get() + (was - entries.len()));
                ours_left_it_empty = entries.is_empty() && was > 0;
            }
            if !ours_left_it_empty {
                kept.push(group);
            }
        }
        *groups = kept;
        if groups.is_empty() && install_created(before, &["hooks", event.as_str()]) {
            emptied.push(event.clone());
        }
    }
    for event in emptied {
        hooks.remove(&event);
    }
    if empty(hooks) && install_created(before, &["hooks"]) {
        root.remove("hooks");
    }
    Ok(())
}

fn strip_server(
    root: &mut Json,
    before: Option<&Json>,
    removed: &Cell<usize>,
) -> Result<(), Failure> {
    let Some(servers) = root.get_mut("mcpServers") else {
        return Ok(());
    };
    if servers.remove(targets::MCP_SERVER_KEY).is_some() {
        removed.set(removed.get() + 1);
    }
    if empty(servers) && install_created(before, &["mcpServers"]) {
        root.remove("mcpServers");
    }
    Ok(())
}

/// Does install's backup prove this container was not in the file before?
///
/// `false` when there is no backup: no proof, and an empty container the user
/// may have written themselves is not verbatim's to delete on a guess.
fn install_created(before: Option<&Json>, path: &[&str]) -> bool {
    let Some(before) = before else {
        return false;
    };
    let mut at = before;
    for key in path {
        match at.get(key) {
            Some(next) => at = next,
            None => return true,
        }
    }
    false
}

fn empty(value: &Json) -> bool {
    match value {
        Json::Object(members) => members.is_empty(),
        Json::Array(items) => items.is_empty(),
        Json::Scalar(_) => false,
    }
}

/// Remove the copy at the stable path, and only if it is verbatim's (D-07).
fn unplace(stable: &Path) -> bool {
    println!("  {}", stable.display());
    match binary::occupant(stable) {
        Ok(binary::Occupant::Vacant) => {
            println!("    not there: nothing to remove");
            true
        }
        Ok(binary::Occupant::Ours) => match std::fs::remove_file(stable) {
            Ok(()) => {
                println!("    removed");
                true
            }
            Err(e) => {
                println!("    could not be removed: {e}");
                if std::env::current_exe().is_ok_and(|exe| exe == stable) {
                    // Windows will not unlink a running image. Saying so beats
                    // leaving the user with an errno.
                    println!(
                        "    that file is this running program; on Windows it can only be \
                         removed after this process exits"
                    );
                }
                false
            }
        },
        Ok(binary::Occupant::Foreign) => {
            println!("    left alone: not a verbatim build, so it is not verbatim's to remove");
            true
        }
        Err(failure) => {
            println!("    left alone: {}", detail(failure));
            false
        }
    }
}

/// `--purge`: the one irreversible thing in this phase.
///
/// The path and the size go on the screen before the question and not after
/// it, because the answer is the last moment either of them can change
/// anything.
fn purge(data_dir: &Path, yes: bool) -> Result<(), Failure> {
    if !data_dir.join(DB_FILE_NAME).exists() {
        // Either nothing was ever ingested here or this is not a data
        // directory. Either way there is no store to delete, and
        // `remove_dir_all` on a directory verbatim cannot recognize is not a
        // mistake worth the one time it would be right.
        println!("  no store at that path, so there is nothing to purge");
        return Ok(());
    }
    let bytes = size_bytes(data_dir);
    println!("  --purge deletes the archive itself. this cannot be undone.");
    println!("    {}", data_dir.display());
    println!("    {bytes} byte(s) ({})", human(bytes));
    println!();

    match answered(data_dir, yes) {
        Ok(true) => {}
        Ok(false) => {
            println!("  the archive was kept.");
            return Ok(());
        }
        Err(failure) => {
            println!("  the archive was kept.");
            return Err(failure);
        }
    }

    // Under the ingest lock, so a pass mid-transaction is never left writing
    // into a store that has been unlinked out from under it. The lock file
    // lives in the directory being deleted, which is why it is released - by
    // the drop at the end of this scope - only after the delete.
    let held = match lock::try_acquire(data_dir)? {
        lock::Attempt::Acquired(held) => held,
        lock::Attempt::Held => {
            println!("  an ingest is running, so the archive was kept.");
            return Err(Failure::Operational(
                "another verbatim process holds the ingest lock; nothing was deleted".to_owned(),
            ));
        }
    };
    std::fs::remove_dir_all(data_dir).map_err(|e| {
        Failure::Operational(format!("{} could not be removed: {e}", data_dir.display()))
    })?;
    drop(held);
    println!("  deleted {bytes} byte(s) at {}", data_dir.display());
    Ok(())
}

/// The one question `--purge` asks, and the one place `--yes` means something
/// different here than it does in install.
///
/// Install's `--yes` takes every default; this one takes the delete. The
/// default for a thing that cannot be undone is no, so a `--yes` that inherited
/// it would make `uninstall --purge --yes` print the size, decline its own
/// question and leave - which is not what anyone typing it asked for.
///
/// Everything else is [`install::confirm`] unchanged: the same reading of y/n,
/// the same terminal test, and above all the same refusal when there is no
/// answer to read rather than an assumed one (INST-08, D-20). That rule is why
/// this calls into install instead of asking here.
fn answered(data_dir: &Path, yes: bool) -> Result<bool, Failure> {
    const QUESTION: &str = "  delete it?";

    if yes {
        println!("{QUESTION} [y/N] y (--yes)");
        return Ok(true);
    }
    install::confirm(QUESTION, false, false).map_err(|_| {
        Failure::Operational(format!(
            "--purge needs an answer and there was none to read; {} is untouched.\n  \
             rerun with --yes to delete it without being asked",
            data_dir.display()
        ))
    })
}

/// The store's footprint on disk: the database and its two sidecars.
///
/// The same three files and the same rule as `cmd::status`'s `size_bytes`, and
/// deliberately so - but not always the same number. `status` measures while
/// its own connection is open, and SQLite's `-shm` and `-wal` exist only for
/// the life of a connection, so `status` counts them and this, which opens
/// nothing, does not. Measured 2026-08-18 against a one-session store:
/// `status --json` reports 159,744 and the directory holds 126,976, the
/// difference being the 32 KiB `-shm` that goes when the connection does.
///
/// What `--purge` shows is therefore what is on disk at that moment, which is
/// the number that is about to go.
fn size_bytes(data_dir: &Path) -> u64 {
    let mut total = 0u64;
    for name in [
        DB_FILE_NAME.to_owned(),
        format!("{DB_FILE_NAME}-wal"),
        format!("{DB_FILE_NAME}-shm"),
    ] {
        if let Ok(meta) = std::fs::metadata(data_dir.join(name)) {
            total += meta.len();
        }
    }
    total
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn detail(failure: Failure) -> String {
    match failure {
        Failure::Operational(message) | Failure::Misuse(message) => message,
        Failure::Silent => "it could not be read".to_owned(),
    }
}
