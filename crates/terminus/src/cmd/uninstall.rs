//! `terminus uninstall`: take out what install put in, and nothing else
//! (INST-07).
//!
//! Install adds four hook entries to `settings.json` and the binary at the
//! stable path, and uninstall removes both. The MCP registration is removed by
//! running `claude mcp remove`, and only when `.claude.json` holds one. That
//! file is read and never written: Claude Code is its only writer. An old
//! `.claude.json.terminus-backup` is left where it is and its path printed,
//! because putting it back would roll back Claude Code's own state.
//!
//! # The hook entries
//!
//! An entry is terminus's when its `command` is the stable path. The walk is
//! over every event key the file has, so an entry an older terminus left under
//! an event this build no longer uses still goes. A group left empty goes with
//! it. An event key left empty goes only when install's backup proves it was
//! not there before, because Claude Code itself writes `"SessionStart": []`.
//!
//! Then the file is compared with install's backup, as values. Equal means
//! nothing but terminus's entries changed, and the backup's bytes go back.
//! Different means the user edited it since, so the edited file stands and the
//! backup stays. Uninstall says which happened.
//!
//! # Partly broken is ordinary
//!
//! A missing file, a missing backup, an entry already removed by hand, a
//! `claude` that is not there or fails: each is reported and uninstall carries
//! on to the rest, then exits 1 if anything did not go as it should.
//!
//! # The archive
//!
//! The data directory is left alone and its path printed. `--purge` deletes it
//! after showing the path and the size and asking once, and takes the ingest
//! lock first so it never pulls the store out from under a running pass.
//!
//! Human-only, no `--json` (D-24).

use std::cell::Cell;
use std::path::Path;

use terminus_core::ingest::lock;
use terminus_core::store::DB_FILE_NAME;

use super::install::json_file::{Document, Json};
use super::install::{self, binary, claude, targets};
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

    println!("terminus uninstall");
    println!();

    // Each of these reports its own lines and returns whether it did what it
    // set out to do. They are ordered the way install writes them, and one
    // failing never stops the next: a half-wired machine is exactly the one
    // this command exists for.
    let mut ok = clean(&targets::settings_path()?, &stable);
    ok &= unregister(&targets::claude_json_path()?);
    ok &= unplace(&stable);

    println!("  data directory  {}", data_dir.display());
    if !options.purge {
        println!(
            "    kept: the archive is the product, and uninstall never deletes it.\n    \
             to delete it too: terminus uninstall --purge"
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
    // above, and a trailing "terminus: ..." would be a second account of the
    // same thing.
    Err(Failure::Silent)
}

/// `settings.json`: take terminus's hook entries out, then decide between the
/// edited file and install's backup.
fn clean(path: &Path, stable: &Path) -> bool {
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
    let edit = |root: &mut Json| strip_hooks(root, &stable, before, &removed);
    if let Err(failure) = document.apply(&edit) {
        println!("    left alone: {}", detail(failure));
        return false;
    }
    let Some(count) = std::num::NonZeroUsize::new(removed.get()) else {
        println!("    no hook entry of terminus's was there");
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
    let count = count.get();
    println!(
        "    removed {count} hook entr{}",
        if count == 1 { "y" } else { "ies" }
    );

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

/// Take terminus's hook entries out of one parsed document.
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
            // Not an array of groups, so not a shape terminus wrote or reads.
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

/// Does install's backup prove this container was not in the file before?
///
/// `false` when there is no backup: no proof, and an empty container the user
/// may have written themselves is not terminus's to delete on a guess.
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

/// Remove the MCP registration with `claude mcp remove`, when there is one
/// (D-13).
///
/// Always `--scope user`: without it the remove fails when a local entry of the
/// same name exists too. With no entry, no `claude` runs. An old
/// `.claude.json.terminus-backup` is reported and never restored or deleted
/// (D-15).
fn unregister(path: &Path) -> bool {
    const REMOVE: [&str; 5] = ["mcp", "remove", "--scope", "user", targets::MCP_SERVER_KEY];

    println!("  {}", path.display());
    let document = match Document::read(path) {
        Ok(document) => document,
        Err(failure) => {
            println!("    left alone: {}", detail(failure));
            return false;
        }
    };
    let registered = document
        .value()
        .get("mcpServers")
        .and_then(|servers| servers.get(targets::MCP_SERVER_KEY))
        .is_some();

    let ok = if !registered {
        println!("    no mcp registration of terminus's was there");
        true
    } else {
        match claude::run(&REMOVE).cause() {
            None => {
                println!("    removed the mcp registration with claude mcp remove");
                true
            }
            Some(cause) => {
                println!("    the mcp registration is still there: {cause}");
                println!("    to remove it, run  claude {}", REMOVE.join(" "));
                false
            }
        }
    };

    if let Ok(backup) = document.backup_path() {
        if std::fs::symlink_metadata(&backup).is_ok() {
            println!(
                "    an old backup from install is at {}\n    \
                 it is left in place and not restored, because it holds an older copy of \
                 Claude Code's own state",
                backup.display()
            );
        }
    }
    ok
}

/// Remove the copy at the stable path, and only if it is terminus's (D-07).
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
                false
            }
        },
        Ok(binary::Occupant::Foreign) => {
            println!("    left alone: not a terminus build, so it is not terminus's to remove");
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
        // `remove_dir_all` on a directory terminus cannot recognize is not a
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
                "another terminus process holds the ingest lock; nothing was deleted".to_owned(),
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

/// The one question `--purge` asks.
///
/// `--yes` takes the delete. The default for something that cannot be undone
/// is no, so a `--yes` that took the default would print the size, decline and
/// leave, which is not what anyone typing it asked for.
///
/// The rest is [`install::confirm`]: the same reading of y/n, and a refusal
/// rather than an assumed answer when there is nothing to read (INST-08, D-20).
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
