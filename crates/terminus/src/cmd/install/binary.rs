//! The copy of this build that hook entries and the MCP registration point at.
//!
//! # One path, forever (D-08, INST-02)
//!
//! `~/.local/bin/terminus` on Linux and macOS,
//! `%LOCALAPPDATA%\Programs\Terminus\terminus.exe` on Windows. Nothing here
//! touches `PATH`: a settings entry names the absolute path, so the copy does
//! not need to be reachable by name, and editing a shell profile is a change to
//! the user's machine install has no business making.
//!
//! The path never moving is what lets [`super::targets`] write the four hook
//! entries once and never rewrite them (INST-05). An upgrade replaces the bytes
//! at this path; the settings files do not learn about it, because there is
//! nothing there to learn.
//!
//! # The source is this process (D-09)
//!
//! [`std::env::current_exe`], not a search of an npm package layout. The npm
//! shim execs the per-platform binary, so the process running `install` IS the
//! artifact to copy, and install needs no knowledge of how npm resolved it.
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

/// The file name the copy carries on this platform.
pub fn file_name() -> &'static str {
    if cfg!(windows) {
        "terminus.exe"
    } else {
        "terminus"
    }
}

fn stable_dir() -> Result<PathBuf, Failure> {
    if let Some(dir) = non_empty_var(BIN_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    platform_dir()
}

#[cfg(not(windows))]
fn platform_dir() -> Result<PathBuf, Failure> {
    let home = non_empty_var("HOME")
        .ok_or_else(|| Failure::Operational(format!("neither {BIN_DIR_ENV} nor HOME is set")))?;
    Ok(PathBuf::from(home).join(".local").join("bin"))
}

#[cfg(windows)]
fn platform_dir() -> Result<PathBuf, Failure> {
    let local = non_empty_var("LOCALAPPDATA").ok_or_else(|| {
        Failure::Operational(format!("neither {BIN_DIR_ENV} nor LOCALAPPDATA is set"))
    })?;
    Ok(PathBuf::from(local).join("Programs").join("Terminus"))
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
    /// Nothing there. The copy is a create.
    Vacant,
    /// A terminus build. The copy is an upgrade and rewrites no settings entry.
    Ours,
    /// Somebody else's program, and the reason to stop.
    Foreign,
}

/// Look at the stable path without touching it.
///
/// Called before the confirmation and before any write, because a refusal that
/// had already copied a binary would not be a refusal.
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

/// Copy this build to the stable path, atomically.
///
/// Through a temporary file in the destination directory and a rename, on every
/// platform. That is atomic everywhere, and on Windows it is also the only way
/// to replace an executable that may be running - the running image holds the
/// old inode and the name points at the new one.
///
/// [`occupant`] is asked again immediately before the rename, and a foreign
/// occupant refuses here too. The check `run` made is from before the
/// confirmation, which is as old as the user took to answer, and the rename
/// clobbers whatever it lands on. What is left is the microseconds between this
/// answer and the rename itself: closing that needs a rename that refuses to
/// replace, which no platform offers through `std`.
pub fn place(dest: &Path) -> Result<(), Failure> {
    let source = std::env::current_exe().map_err(|e| {
        Failure::Operational(format!("this build's own path could not be resolved: {e}"))
    })?;
    let dir = dest.parent().ok_or_else(|| {
        Failure::Operational(format!("{} has no parent directory", dest.display()))
    })?;
    std::fs::create_dir_all(dir).map_err(|e| {
        Failure::Operational(format!("{} could not be created: {e}", dir.display()))
    })?;

    // Exclusively, and not with `File::create`: a stale `.terminus-install-<pid>`
    // symlink would otherwise be followed, truncating whatever it points at and
    // then renaming the link itself over the stable path.
    let (temporary, file) = super::create_temporary(dir, ".terminus-install-")?;
    let outcome = copy_into(&source, file, &temporary)
        .and_then(|()| match occupant(dest)? {
            // Not the check `run` made: that one is from before the question was
            // asked. This is the last look anything gets at the path the rename
            // is about to take.
            Occupant::Foreign => Err(super::occupied(dest)),
            Occupant::Vacant | Occupant::Ours => Ok(()),
        })
        .and_then(|()| {
            std::fs::rename(&temporary, dest).map_err(|e| {
                Failure::Operational(format!("{} could not be replaced: {e}", dest.display()))
            })
        });
    if outcome.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o755))
            .map_err(|e| {
                Failure::Operational(format!(
                    "{} could not be made executable: {e}",
                    temporary.display()
                ))
            })?;
    }
    // Before the rename, not after: a crash between the two would otherwise put
    // an empty file at the path every hook entry names.
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
