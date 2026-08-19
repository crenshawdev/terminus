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
//! The data directory is left alone and its path is printed. Nothing in this
//! command touches it.
//!
//! Human-only, no `--json` (D-24).

use std::cell::Cell;
use std::path::Path;

use super::install::json_file::{Document, Json};
use super::install::{binary, targets};
use super::Failure;

pub struct Options {}

pub fn parse(parser: &mut lexopt::Parser) -> Result<Options, Failure> {
    // Every argument is rejected, `--json` deliberately among them (D-24).
    if let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        return Err(Failure::Misuse(crate::unexpected(arg)));
    }
    Ok(Options {})
}

pub fn run(_options: Options) -> Result<(), Failure> {
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
    println!("    kept: the archive is the product, and uninstall never deletes it.");
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

fn detail(failure: Failure) -> String {
    match failure {
        Failure::Operational(message) | Failure::Misuse(message) => message,
        Failure::Silent => "it could not be read".to_owned(),
    }
}
