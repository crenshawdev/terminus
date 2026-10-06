//! `terminus data move <path>`: put the whole data directory somewhere else and
//! record where it went (STOR-07, D-06).
//!
//! **The whole directory, never just the database.** The data directory holds
//! the `LOCK` file, the per-session injection-state directory
//! (`inject::decision::DIR_NAME`), the rolling snapshots and `terminus.db`'s two
//! WAL sidecars. A move that carried only `terminus.db` would strand the
//! undrained decision files, and the next pass would report "the decision log
//! could not be drained" into `runs.error` while the replay history sat at the
//! old path.
//!
//! **A copy and then a delete, never `fs::rename`.** A rename across
//! filesystems fails, and relocating onto another disk is the whole point of
//! the command.
//!
//! **Human-only, no `--json` (D-17).** It shows what it is about to do and asks
//! once, and a single JSON document on stdout cannot be both of those things -
//! the reason `main.rs` already gives for `install` and `uninstall`. `--yes` is
//! the way to skip the question, exactly as it is for `uninstall --purge`.
//!
//! **It touches no settings file.** The hook entries are written as
//! `args: ["hook", <event>]` and the MCP registration as `args: ["mcp"]`, so
//! neither names a store path and there is nothing there to rewrite (D-14).
//! Rewriting it would break INST-05's "hooks are written once and never
//! rewritten".
//!
//! **It warns about network filesystems and asks anyway (D-16).** No `std` API
//! detects a network mount on any of the three first-class targets, and adding
//! one would be a platform-specific dependency on the hook-path binary. The
//! command asks regardless of destination, so an unconditional warning costs
//! nothing and is the mitigation this phase ships.

use std::path::{Path, PathBuf};

use terminus_core::ingest::lock;
use terminus_core::store;

use super::Failure;

/// The one verb `data` takes.
const VERB: &str = "move";

/// The hazard nobody can detect for the user, said every time (D-16).
const NETWORK_WARNING: &str =
    "  sqlite is not safe on a network filesystem (nfs, smb, sshfs, or a synced folder\n  \
     that a background agent rewrites). terminus cannot detect one, so if that path is\n  \
     on a remote mount, answer no.";

pub struct Options {
    destination: PathBuf,
    yes: bool,
}

/// The second two-word subcommand in this binary, parsed the way
/// `cmd::observations::parse` parses the first.
///
/// The verb comes first or not at all, and any other second word is misuse
/// rather than a word that was silently ignored: `terminus data /somewhere`
/// reads like a move and must not be one.
pub fn parse(parser: &mut lexopt::Parser) -> Result<Options, Failure> {
    use lexopt::prelude::*;

    let mut verb: Option<String> = None;
    let mut destination: Option<PathBuf> = None;
    let mut yes = false;

    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Value(word) if verb.is_none() => {
                let word = word.to_string_lossy().into_owned();
                if word != VERB {
                    return Err(Failure::Misuse(format!(
                        "unknown `data` subcommand '{word}'; the only verb is `data {VERB} <path>`"
                    )));
                }
                verb = Some(word);
            }
            Value(path) if destination.is_none() => destination = Some(PathBuf::from(path)),
            Value(extra) => {
                return Err(Failure::Misuse(format!(
                    "`data {VERB}` takes one destination, and '{}' is a second",
                    extra.to_string_lossy()
                )))
            }
            Short('y') | Long("yes") => yes = true,
            // Including `--json`, deliberately (D-17).
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }

    if verb.is_none() {
        return Err(Failure::Misuse(format!(
            "`data` needs a verb: `data {VERB} <path>`"
        )));
    }
    let Some(destination) = destination else {
        return Err(Failure::Misuse(format!(
            "`data {VERB}` needs the path to move the data directory to"
        )));
    };
    Ok(Options { destination, yes })
}

pub fn run(options: Options) -> Result<(), Failure> {
    let source = super::data_dir()?;
    let destination = absolute(&options.destination)?;
    let pointer = store::location_pointer_path()?;

    refuse_impossible_destinations(&source, &destination)?;

    println!("terminus data move");
    println!();
    println!("  from  {}", source.display());
    println!("  to    {}", destination.display());
    println!("  {} byte(s), including the archive itself", bytes(&source));
    println!();
    println!("{NETWORK_WARNING}");
    println!();

    if !answered(&source, options.yes)? {
        println!("  nothing was moved.");
        return Ok(());
    }

    // Under the ingest lock, so a pass mid-transaction is never left writing
    // into a directory that has moved out from under it. `try_acquire` creates
    // the directory if it is not there yet, which is how a machine that has
    // never ingested can still choose where its store will land.
    let held = match lock::try_acquire(&source)? {
        lock::Attempt::Acquired(held) => held,
        lock::Attempt::Held => {
            return Err(Failure::Operational(
                "another terminus process holds the ingest lock; nothing was moved".to_owned(),
            ))
        }
    };

    copy_tree(&source, &destination).map_err(|e| {
        Failure::Operational(format!(
            "{e}\n  nothing was deleted, and the location still points at {}.\n  \
             what was copied so far is at {}",
            source.display(),
            destination.display()
        ))
    })?;

    // The lock file's own bytes carry no meaning - `ingest::lock` says so, and
    // says only the OS lock attached to it does - so the destination gets a
    // fresh empty one rather than a copy of the file this process is holding
    // locked, which Windows would refuse to read through a second handle.
    //
    // Owner-only from the creating open, like the one `ingest::lock` makes at
    // the source (PRIV-04, D-10): the lock file is short, but a `data move`
    // that left one file in the new directory readable by the world would make
    // the destination's mode a lie on the very first listing.
    let lock_file = destination.join(lock::LOCK_FILE_NAME);
    terminus_core::owner_only::options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_file)
        .map_err(|e| {
            Failure::Operational(format!("{} could not be created: {e}", lock_file.display()))
        })?;

    // Before the source is removed, and atomically: a crash between the copy
    // and the pointer leaves both directories intact and the store still
    // resolving to the old one, which is recoverable. The other order loses the
    // store's address with the data still on disk.
    write_pointer(&pointer, &destination)?;

    // Only now: the lock file being removed is the one this process holds, and
    // Windows will not unlink an open file.
    drop(held);
    std::fs::remove_dir_all(&source).map_err(|e| {
        Failure::Operational(format!(
            "the move is done and {} could not be removed: {e}\n  \
             it is now an unused copy and can be deleted by hand",
            source.display()
        ))
    })?;

    println!("  moved. the store is at {}", destination.display());
    println!("  recorded in {}", pointer.display());
    println!(
        "  nothing in settings.json changed: the hook and mcp entries never named a store path"
    );
    Ok(())
}

/// The three destinations that cannot be honored, refused before anything is
/// printed or asked.
///
/// The nested case is the one worth naming: copying a directory into a
/// subdirectory of itself walks the copy it is making, so it is refused rather
/// than allowed to fill the disk.
fn refuse_impossible_destinations(source: &Path, destination: &Path) -> Result<(), Failure> {
    if destination == source {
        return Err(Failure::Operational(format!(
            "{} is where the data directory already is",
            destination.display()
        )));
    }
    if destination.starts_with(source) {
        return Err(Failure::Operational(format!(
            "{} is inside the data directory being moved",
            destination.display()
        )));
    }
    let occupied = std::fs::read_dir(destination)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false);
    if occupied {
        return Err(Failure::Operational(format!(
            "{} already holds files; nothing was copied.\n  \
             name a path that does not exist or is empty",
            destination.display()
        )));
    }
    Ok(())
}

/// The question, with `--yes` meaning "do it" rather than "take the default".
///
/// [`super::install::confirm`] answers with the DEFAULT under `--yes`, and the
/// default for a thing that rewrites where the archive lives is no - so a
/// `data move --yes` that went through it unchanged would print the question,
/// decline it and leave. Everything else about the question is install's,
/// including the refusal when there is no answer to read (INST-08, D-20).
fn answered(source: &Path, yes: bool) -> Result<bool, Failure> {
    const QUESTION: &str = "  move it?";

    if yes {
        println!("{QUESTION} [y/N] y (--yes)");
        return Ok(true);
    }
    super::install::confirm(QUESTION, false, false).map_err(|_| {
        Failure::Operational(format!(
            "data move needs an answer and there was none to read; {} is untouched.\n  \
             rerun with --yes to move it without being asked",
            source.display()
        ))
    })
}

/// The pointer `store::open::data_dir` reads, written through a temporary file
/// and a rename.
///
/// Atomic because the alternative is a truncated pointer: a process killed
/// mid-write would leave a file naming half a path, and the next command would
/// resolve a store that does not exist rather than either the old one or the
/// new one.
fn write_pointer(pointer: &Path, destination: &Path) -> Result<(), Failure> {
    let Some(dir) = pointer.parent() else {
        return Err(Failure::Operational(format!(
            "{} has no parent directory to write into",
            pointer.display()
        )));
    };
    let io = |path: &Path, e: std::io::Error| {
        Failure::Operational(format!("{} could not be written: {e}", path.display()))
    };

    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let temporary = pointer.with_extension("tmp");
    std::fs::write(&temporary, format!("{}\n", destination.display()))
        .map_err(|e| io(&temporary, e))?;
    std::fs::rename(&temporary, pointer).map_err(|e| io(pointer, e))
}

/// A relative destination resolved against the working directory.
///
/// The pointer file holds one ABSOLUTE path and the resolver refuses anything
/// else, because a relative pointer would resolve against whatever directory a
/// hook happened to be spawned in - a store per repository rather than one
/// store. So the resolution happens here, where a working directory still means
/// what the user meant by it.
fn absolute(path: &Path) -> Result<PathBuf, Failure> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|e| {
        Failure::Operational(format!(
            "{} is a relative path and the working directory could not be read: {e}",
            path.display()
        ))
    })?;
    Ok(cwd.join(path))
}

/// Everything under `from`, copied into `to`, directories included.
///
/// The `LOCK` file is skipped at the top level: this process is holding it, and
/// [`run`] creates an empty one at the destination instead.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let failed =
        |path: &Path, e: std::io::Error| format!("{} could not be copied: {e}", path.display());

    // Every directory 0700 and every file 0600, written rather than reproduced
    // (D-10). A plain byte-for-byte copy carries the SOURCE's mode across with
    // it, which makes this the one command that can undo the whole phase: a
    // store the user moved off a build that predates this, or restored out of
    // a tar, would arrive as wide as it was at the old location.
    // What the destination gets is the mode this build writes, from the
    // creating syscall, whatever the source happens to be.
    terminus_core::owner_only::create_dir_all(to).map_err(|e| failed(to, e))?;
    let entries = std::fs::read_dir(from).map_err(|e| failed(from, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| failed(from, e))?;
        let name = entry.file_name();
        if name == std::ffi::OsStr::new(lock::LOCK_FILE_NAME) {
            continue;
        }
        let source = entry.path();
        let destination = to.join(&name);
        let kind = entry.file_type().map_err(|e| failed(&source, e))?;
        if kind.is_dir() {
            copy_tree(&source, &destination)?;
        } else {
            copy_file(&source, &destination).map_err(|e| failed(&source, e))?;
        }
    }
    Ok(())
}

/// One file copied into a destination this process creates at 0600.
///
/// `create_new`, because [`refuse_impossible_destinations`] has established
/// that the destination directory was absent or empty: a name already taken
/// here means something else is writing into the directory mid-move, and
/// truncating it would destroy a file this command never wrote.
fn copy_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    let mut reader = std::fs::File::open(source)?;
    let mut writer = terminus_core::owner_only::options()
        .write(true)
        .create_new(true)
        .open(destination)?;
    std::io::copy(&mut reader, &mut writer)?;
    Ok(())
}

/// What the move is about to carry, in bytes.
///
/// Every file under the directory rather than the three `cmd::status` sums:
/// what moves here is the injection-state directory (`decision::DIR_NAME`) and
/// the snapshots too, and a size that named only the database would understate
/// a directory holding three rolling copies of it by a factor of four.
fn bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => total += bytes(&entry.path()),
            Ok(_) => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => {}
        }
    }
    total
}
