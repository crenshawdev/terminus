//! The build that hook entries and the MCP registration point at.
//!
//! # One path, forever (D-08, INST-02)
//!
//! `~/.local/bin/terminus`. Nothing here touches `PATH`: a settings entry names
//! the absolute path, so the binary does not need to be reachable by name, and
//! editing a shell profile is a change to the user's machine install has no
//! business making.
//!
//! The path never moving is what lets [`super::targets`] write the four hook
//! entries once and never rewrite them (INST-05). The settings files never
//! learn which version is behind it, because there is nothing there to learn.
//!
//! # A link to a stored version
//!
//! The stable path is a symbolic link to
//! `~/.local/lib/crenshawdev/terminus/versions/<version>/terminus`, the layout
//! Baley uses and `install.sh` makes. Install stores the running binary, found
//! through [`std::env::current_exe`], under its own version and moves the link
//! onto it, so `target/release/terminus install` installs a local build.
//!
//! An upgrade moves the link and deletes nothing. A session already running
//! keeps the file it started from, and so does the ingest its hook spawned.
//! A rebuild at the same version is renamed over that version's file, and a
//! process running the old one keeps its inode.
//!
//! The versions folder is resolved from `HOME` even when [`BIN_DIR_ENV`] is set,
//! and it sits outside the data directory, because `uninstall --purge` deletes
//! that and the data directory can move.
//!
//! # The marker, and why version cannot do its job (D-07)
//!
//! As of 2026-08-13 `/home/john/.local/bin/verbatim`, the stable path before
//! the rename to terminus, was a 12,076,464-byte *different program* - "capture
//! transcripts before cleanup", subcommands `precompact, normalize, load, ...` -
//! whose `--version` printed `verbatim 0.1.0`, byte-identical to this build's
//! output at the time. So identity cannot be a
//! version string and cannot be a filename. It is [`MARKER`]: a fixed ASCII
//! sentinel in this binary's image, referenced from [`carries_marker`] so it
//! survives the release profile's `strip = "debuginfo"`, and looked for in
//! whatever file already sits at the stable path. A file that does not carry it
//! is somebody else's and is refused, not overwritten.
//!
//! The scan streams in bounded chunks with an overlap of one sentinel length,
//! so a 12 MB executable costs a 64 KiB buffer rather than 12 MB of resident
//! memory, and a sentinel straddling a chunk boundary is still found.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::cmd::Failure;

/// Points the stable directory somewhere else, spelled after
/// `TERMINUS_DATA_DIR` and `TERMINUS_CONFIG_DIR`.
///
/// Introduced for testability and honest about it: a test that installed into
/// the developer's real `~/.local/bin` would overwrite the binary they are
/// standing on. `doctor` and `uninstall` read the same variable, or their tests
/// reach the same place.
pub const BIN_DIR_ENV: &str = "TERMINUS_BIN_DIR";

/// The sentinel that says a file at the stable path is a terminus build.
///
/// A `static` referenced from [`carries_marker`] rather than a `const` folded
/// into nothing: it has to be *in the image* of every binary this project
/// ships, including a release build that strips debug info, because the next
/// install is going to look for it there.
///
/// The UUID is the whole of the uniqueness argument. Nothing else in a
/// filesystem is going to carry these 36 bytes by accident, and a program that
/// carries them deliberately is claiming to be a terminus build.
pub static MARKER: &str = "terminus-stable-binary:8f3d1a6c-72b4-4e59-9c0d-5a1e7b34f082";

/// How much of a candidate file is held in memory at once.
const CHUNK: usize = 64 * 1024;

/// Where this build belongs, and where every settings entry points.
pub fn stable_path() -> Result<PathBuf, Failure> {
    Ok(stable_dir()?.join(file_name()))
}

/// The file name at the stable path and in every version folder.
pub fn file_name() -> &'static str {
    "terminus"
}

fn stable_dir() -> Result<PathBuf, Failure> {
    if let Some(dir) = non_empty_var(BIN_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    platform_dir()
}

fn platform_dir() -> Result<PathBuf, Failure> {
    let home = non_empty_var("HOME")
        .ok_or_else(|| Failure::Operational(format!("neither {BIN_DIR_ENV} nor HOME is set")))?;
    Ok(PathBuf::from(home).join(".local").join("bin"))
}

/// Where every stored version lives, one folder per version.
pub fn versions_dir() -> Result<PathBuf, Failure> {
    let home =
        non_empty_var("HOME").ok_or_else(|| Failure::Operational("HOME is not set".to_owned()))?;
    let home = PathBuf::from(home);
    // The link holds this path, and a relative one would resolve against the
    // stable folder rather than against anything HOME meant.
    if !home.is_absolute() {
        return Err(Failure::Operational(format!(
            "HOME is {}, which is not an absolute path",
            home.display()
        )));
    }
    Ok(home
        .join(".local")
        .join("lib")
        .join("crenshawdev")
        .join("terminus")
        .join("versions"))
}

/// An environment variable set to the empty string is unset: an empty path
/// would resolve to the process's working directory.
fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// What install found at the stable path, before it has written anything.
pub enum Occupant {
    /// Nothing there, or a link to nothing. The link is a create.
    Vacant,
    /// A terminus build, or a link to one. Replacing it rewrites no settings entry.
    Ours,
    /// Somebody else's program, and the reason to stop.
    Foreign,
}

/// Look at the stable path without touching it.
///
/// Called before any write, because a refusal that had already stored a binary
/// would not be a refusal.
pub fn occupant(path: &Path) -> Result<Occupant, Failure> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Occupant::Vacant),
        Err(e) => {
            return Err(Failure::Operational(format!(
                "{} could not be read: {e}",
                path.display()
            )))
        }
    };
    if carries_marker(file)
        .map_err(|e| Failure::Operational(format!("{} could not be read: {e}", path.display())))?
    {
        Ok(Occupant::Ours)
    } else {
        Ok(Occupant::Foreign)
    }
}

/// Does this file carry [`MARKER`] anywhere in it?
fn carries_marker(mut file: impl Read) -> std::io::Result<bool> {
    let needle = MARKER.as_bytes();
    let overlap = needle.len() - 1;
    let mut buffer = vec![0u8; CHUNK + overlap];
    let mut carried = 0usize;
    loop {
        let read = file.read(&mut buffer[carried..])?;
        if read == 0 {
            return Ok(false);
        }
        let filled = carried + read;
        if buffer[..filled]
            .windows(needle.len())
            .any(|window| window == needle)
        {
            return Ok(true);
        }
        // Keep the tail, so a sentinel split across two reads is still whole in
        // the next window.
        carried = overlap.min(filled);
        buffer.copy_within(filled - carried..filled, 0);
    }
}

/// The prefix of every temporary name install makes beside the binary.
const TEMPORARY: &str = ".terminus-install-";

/// Store this build as a version and point the stable path at it.
///
/// Returns the version file the link now names. When install runs from the
/// file the link already names, nothing is written.
pub fn place(stable: &Path, versions: &Path) -> Result<PathBuf, Failure> {
    let source = std::env::current_exe().map_err(|e| {
        Failure::Operational(format!("this build's own path could not be resolved: {e}"))
    })?;
    if let Some(target) = linked_to(stable, versions, &source) {
        return Ok(target);
    }
    let version_file = versions.join(env!("CARGO_PKG_VERSION")).join(file_name());
    store(&source, &version_file)?;
    link(stable, &version_file)?;
    Ok(version_file)
}

/// The link's target, when it is a file in a folder directly under the
/// versions folder and is the same file as `source`.
///
/// Same file means the same device and inode. A path comparison would miss a
/// HOME reached through a linked folder, because `current_exe()` is resolved
/// and the link holds whatever spelling of HOME wrote it. The folder's name is
/// never compared with this build's version: `install.sh` names it from its
/// manifest.
fn linked_to(stable: &Path, versions: &Path, source: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;

    let target = std::fs::read_link(stable).ok()?;
    let mut rest = target.strip_prefix(versions).ok()?.components();
    let (Some(Component::Normal(_)), Some(Component::Normal(_)), None) =
        (rest.next(), rest.next(), rest.next())
    else {
        return None;
    };
    let linked = std::fs::metadata(&target).ok()?;
    let running = std::fs::metadata(source).ok()?;
    (linked.dev() == running.dev() && linked.ino() == running.ino()).then_some(target)
}

/// Put `source`'s bytes at `dest`, unless they are already there.
///
/// Through a temporary file in the same folder and a rename, so a process
/// running the old file keeps its inode. Only a regular file is compared: a
/// link at `dest` is renamed over, never written through.
fn store(source: &Path, dest: &Path) -> Result<(), Failure> {
    let dir = parent(dest)?;
    create_dir(dir)?;
    if std::fs::symlink_metadata(dest).is_ok_and(|meta| meta.file_type().is_file())
        && same_bytes(source, dest).map_err(|e| {
            Failure::Operational(format!(
                "{} could not be compared with {}: {e}",
                dest.display(),
                source.display()
            ))
        })?
    {
        return Ok(());
    }

    // Exclusively, and not with `File::create`: a stale `.terminus-install-<pid>`
    // symlink would otherwise be followed and whatever it points at truncated.
    let (temporary, file) = super::create_temporary(dir, TEMPORARY)?;
    let outcome = copy_into(source, file, &temporary).and_then(|()| {
        std::fs::rename(&temporary, dest).map_err(|e| {
            Failure::Operational(format!("{} could not be replaced: {e}", dest.display()))
        })
    });
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

/// Move a link to `target` over the stable path.
///
/// [`occupant`] is asked again immediately before the rename, and a foreign
/// occupant refuses here too. The check `run` made is from before anything was
/// written, and the rename clobbers whatever it lands on. What is left is the
/// microseconds between this answer and the rename itself: closing that needs
/// a rename that refuses to replace, which no platform offers through `std`.
fn link(stable: &Path, target: &Path) -> Result<(), Failure> {
    let dir = parent(stable)?;
    create_dir(dir)?;
    let (temporary, ()) = super::temporary(dir, TEMPORARY, |path| {
        std::os::unix::fs::symlink(target, path)
    })?;
    let outcome = match occupant(stable) {
        Ok(Occupant::Foreign) => Err(super::occupied(stable)),
        Ok(Occupant::Vacant | Occupant::Ours) => std::fs::rename(&temporary, stable).map_err(|e| {
            Failure::Operational(format!("{} could not be replaced: {e}", stable.display()))
        }),
        Err(failure) => Err(failure),
    };
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

fn parent(path: &Path) -> Result<&Path, Failure> {
    path.parent()
        .ok_or_else(|| Failure::Operational(format!("{} has no parent directory", path.display())))
}

fn create_dir(dir: &Path) -> Result<(), Failure> {
    std::fs::create_dir_all(dir)
        .map_err(|e| Failure::Operational(format!("{} could not be created: {e}", dir.display())))
}

/// Do the two files hold the same bytes? Streamed, a chunk of each at a time.
fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    let mut a = std::fs::File::open(a)?;
    let mut b = std::fs::File::open(b)?;
    if a.metadata()?.len() != b.metadata()?.len() {
        return Ok(false);
    }
    let mut left = vec![0u8; CHUNK];
    let mut right = vec![0u8; CHUNK];
    loop {
        let n = fill(&mut a, &mut left)?;
        let m = fill(&mut b, &mut right)?;
        if left[..n] != right[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}

/// Read until `buffer` is full or the file ends, so two files are compared in
/// chunks that line up.
fn fill(file: &mut impl Read, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

fn copy_into(source: &Path, mut file: std::fs::File, temporary: &Path) -> Result<(), Failure> {
    let mut reading = std::fs::File::open(source).map_err(|e| {
        Failure::Operational(format!("{} could not be read: {e}", source.display()))
    })?;
    std::io::copy(&mut reading, &mut file).map_err(|e| {
        Failure::Operational(format!(
            "{} could not be copied to {}: {e}",
            source.display(),
            temporary.display()
        ))
    })?;
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
        .map_err(|e| {
            Failure::Operational(format!(
                "{} could not be made executable: {e}",
                temporary.display()
            ))
        })?;
    // Before the rename, not after: a crash between the two would otherwise put
    // an empty file behind the path every hook entry names.
    file.sync_all().map_err(|e| {
        Failure::Operational(format!("{} could not be flushed: {e}", temporary.display()))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentinel is in this test binary's own image, which is the property
    /// every install run depends on: `carries_marker` is asked about a file,
    /// and the answer is only worth anything if a real terminus build answers
    /// yes.
    #[test]
    fn this_build_carries_its_own_marker() {
        let exe = std::env::current_exe().unwrap();
        let file = std::fs::File::open(&exe).unwrap();
        assert!(
            carries_marker(file).unwrap(),
            "{} does not carry the marker install looks for",
            exe.display()
        );
    }

    /// A sentinel straddling a chunk boundary is still found: that is the whole
    /// reason the scan carries an overlap rather than reading chunk by chunk.
    #[test]
    fn a_marker_split_across_two_reads_is_still_found() {
        let mut bytes = vec![b'.'; CHUNK - MARKER.len() / 2];
        bytes.extend_from_slice(MARKER.as_bytes());
        bytes.extend(std::iter::repeat_n(b'.', 4096));
        assert!(carries_marker(bytes.as_slice()).unwrap());
    }

    #[test]
    fn a_file_without_the_marker_is_foreign() {
        let bytes = vec![b'x'; CHUNK * 2 + 17];
        assert!(!carries_marker(bytes.as_slice()).unwrap());
    }
}
