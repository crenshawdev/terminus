//! Creating a file or a directory owner-only, at the moment it is created.
//!
//! Nothing terminus writes is readable by group or world (PRIV-02, PRIV-04),
//! and the mode is carried IN the creating syscall rather than applied to a
//! path that already exists. That is not tidiness: a `create` followed by a
//! `chmod` leaves a window in which the file is on disk at the umask's mode
//! with the first bytes already in it, and the file this project writes holds
//! whole session transcripts. Every creation site in the store, the ingest
//! lock, the snapshot copy, the injection scratch and the export goes through
//! the two helpers below, so there is one place to read the rule off.
//!
//! `std` alone does it (D-01): [`std::os::unix::fs::DirBuilderExt::mode`] and
//! [`std::os::unix::fs::OpenOptionsExt::mode`]. No `libc`, no `nix`, no
//! `windows-sys` - the hook path is measured in fractions of a millisecond and
//! does not pay for a crate to set two integers.
//!
//! On Windows the equivalent of owner-only is an ACL, and this build does not
//! set one (D-15, deferred since v0.1.0 phase 7). The `#[cfg(not(unix))]` arms
//! here create the same file and the same directory with no mode at all, so a
//! Windows install is exactly as protected as the directory it inherits from -
//! which is why `terminus doctor` says so in words rather than reporting "ok"
//! over an ACL nothing examined.

use std::path::Path;

/// Owner-only for a directory: `rwx` for the owner, nothing for anyone else.
pub const DIR_MODE: u32 = 0o700;

/// Owner-only for a file: `rw` for the owner, nothing for anyone else.
pub const FILE_MODE: u32 = 0o600;

/// `create_dir_all`, except the LEAF lands [`DIR_MODE`].
///
/// The parent chain is created at the process default and the leaf alone
/// carries the mode (D-04). A recursive [`std::fs::DirBuilder`] with a mode set
/// would be one line shorter and wrong: std applies that builder's mode to
/// every component it creates, so the first ingest on a machine whose
/// `~/.local/share` did not exist yet would leave `~/.local/share` itself 0700
/// and every other tool's data under it unreachable from the user's own
/// group-shared workflows. terminus narrows its own directory and no directory
/// above it.
///
/// A leaf that is already there is success and keeps whatever mode it has:
/// narrowing an existing directory is the repair in `store::open`, which runs
/// once per open over a known list, and not a side effect of every writer that
/// happens to call this. A path occupied by something that is not a directory
/// is still the error `std::fs::create_dir_all` would have returned.
pub fn create_dir_all(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(DIR_MODE);
    }

    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(e) => Err(e),
    }
}

/// [`std::fs::OpenOptions`] already carrying [`FILE_MODE`], for the caller to
/// finish.
///
/// The mode applies only when the open creates the file, which is what makes
/// this safe to hand to every writer regardless of its other flags: the caller
/// adds its own `create`/`create_new`/`truncate`/`read`/`write`, so the ingest
/// lock's never-truncate open and the injection scratch's exclusive create both
/// fit without a second helper. Opening a file that already exists changes no
/// mode here.
pub fn options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    options
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// D-04 in one case: the leaf is ours to narrow and the parents are not.
    #[test]
    fn only_the_leaf_directory_is_owner_only() {
        let root = tempfile::tempdir().unwrap();
        let leaf = root.path().join("a/b/leaf");

        create_dir_all(&leaf).unwrap();

        assert_eq!(mode(&leaf), 0o700, "the leaf terminus owns");
        assert_eq!(
            mode(&root.path().join("a")),
            0o755,
            "a parent created on the way keeps the umask's own default"
        );
        assert_eq!(
            mode(&root.path().join("a/b")),
            0o755,
            "and so does the next"
        );
    }

    /// An existing directory is left exactly as it was found: the repair in
    /// `store::open` narrows one, not every writer that passes through here.
    ///
    /// The wide leaf is made by plain [`std::fs::create_dir`] at the process
    /// umask - 0755 under the umask 022 this phase's tests run at - rather than
    /// by a `chmod`, because widening then narrowing is the very thing this
    /// module exists to avoid and no mode is re-applied to an existing path
    /// anywhere in this file (AC2).
    #[test]
    fn an_existing_leaf_is_success_and_keeps_its_mode() {
        let root = tempfile::tempdir().unwrap();
        let leaf = root.path().join("leaf");
        std::fs::create_dir(&leaf).unwrap();
        let before = mode(&leaf);

        create_dir_all(&leaf).unwrap();

        assert_eq!(mode(&leaf), before, "an existing leaf is not re-moded");
    }

    /// A file sitting where a directory should be is still an error, the same
    /// one `std::fs::create_dir_all` returns.
    #[test]
    fn a_file_in_the_leafs_place_is_still_an_error() {
        let root = tempfile::tempdir().unwrap();
        let occupied = root.path().join("occupied");
        std::fs::write(&occupied, b"not a directory").unwrap();

        let error = create_dir_all(&occupied).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn a_created_file_is_owner_only() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("secret");

        options().write(true).create_new(true).open(&path).unwrap();

        assert_eq!(mode(&path), 0o600);
    }

    /// The lock file's shape: created if missing, never truncated, and still
    /// 0600 the moment it exists.
    #[test]
    fn a_create_without_truncate_is_owner_only_too() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("LOCK");

        options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();

        assert_eq!(mode(&path), 0o600);
    }
}
