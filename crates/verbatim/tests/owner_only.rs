//! Owner-only at creation, observed through the shipped binary under a umask
//! that would otherwise widen everything (PRIV-04, AC1).
//!
//! **The umask is set for the spawn, not for the test process.** Every command
//! here goes through `sh -c 'umask 022; exec "$0" "$@"'` (D-08). A test that
//! called `umask()` in-process would need a `libc` dev-dependency this project
//! does not have, would leak into every other test in the binary, and - worse -
//! a developer whose own umask is already 077 would get a green run out of a
//! build that sets no mode at all. That is the exact false green these tests
//! exist to prevent, so the umask is stated rather than inherited.
//!
//! The binary path and its arguments are POSITIONAL parameters of that `sh`,
//! never interpolated into the shell text: a temporary directory's name is
//! chosen by the operating system and a path with a space or a quote in it must
//! not become two words.
//!
//! Names are spelled out here - `verbatim.db`, `LOCK`, `snapshots` - rather
//! than imported from `verbatim_core::store`. What these tests assert is what a
//! user finds on disk, and a constant that moved with the code would move the
//! assertion with it.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use verbatim_core::testkit;

/// The encoded project directory a fixture is placed under.
const PROJECT: &str = "-data-projects-cadence";

/// The manifest every export writes, and the one file in it that is not a
/// transcript.
const MANIFEST: &str = "manifest.json";

/// Every directory a spawned `verbatim` may touch, all of them temporary.
///
/// The same isolation `tests/lifecycle.rs` uses: a spawn that set only
/// `VERBATIM_DATA_DIR` would resolve the developer's real config and walk the
/// live `~/.claude` tree.
struct Bench {
    _dir: tempfile::TempDir,
    /// The temporary root, which is where an export destination goes.
    root: PathBuf,
    data_dir: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    work: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let config_dir = root.join("config");
    let claude_dir = root.join("claude");
    let work = root.join("work");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    Bench {
        _dir: dir,
        data_dir: root.join("data"),
        config_dir,
        claude_dir,
        work,
        root,
    }
}

/// The shell program every spawn runs: set the umask, then become the binary.
///
/// `exec` so there is no shell left in the process tree to confuse a test that
/// cares about what the binary spawned, and `"$0" "$@"` so the path and the
/// arguments are words the shell never re-splits.
const UMASK_022: &str = "umask 022; exec \"$0\" \"$@\"";

impl Bench {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(UMASK_022)
            .arg(env!("CARGO_BIN_EXE_verbatim"))
            .args(args)
            .current_dir(&self.work)
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// Run with stdin at end of file, which is what a command in a script sees.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .stdin(Stdio::null())
            .output()
            .expect("sh runs the binary")
    }

    fn ok(&self, args: &[&str]) -> Output {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// One fixture transcript under the bench's own Claude projects tree.
    fn place(&self, fixture: &str) -> PathBuf {
        let name = "00000001-1111-4111-8111-111111111111.jsonl";
        let dest = self.claude_dir.join("projects").join(PROJECT).join(name);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
        dest
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap_or_else(|e| panic!("{} is not there: {e}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

/// One path, one assertion, and the path in the message.
///
/// Deliberately not the batched `wrong_modes` shape `tests/tighten.rs` uses.
/// That file is about a repair which either ran or did not, so naming every
/// offender at once is what its reader needs; here each path is set by a
/// different creation site, and the first failure should say which site.
#[track_caller]
fn owner_only(path: &Path, want: u32) {
    let found = mode(path);
    assert!(
        found == want,
        "{} is {found:o}, want {want:o}",
        path.display()
    );
}

/// Everything directly inside a directory, sorted, and never empty.
///
/// The emptiness check is the falsifying half of every loop below: a `for` over
/// no entries asserts nothing at all, and a hook that wrote no file would make
/// this file green while the thing it is about went untested.
fn contents(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "{} is empty", dir.display());
    paths
}

/// AC1's export clause: `verbatim export` writes a directory nobody else on the
/// machine can open, holding files nobody else can read.
///
/// This is the one command whose destination the USER names, and it is still
/// forced to 0700/0600 (D-11) - what lands there is every prompt and every tool
/// result, unredacted, and the fact that the path was asked for does not make
/// it less so.
#[test]
fn an_export_directory_and_every_file_in_it_are_owner_only() {
    let bench = bench();
    bench.place("session-basic.jsonl");
    bench.ok(&["ingest"]);

    let destination = bench.root.join("export");
    bench.ok(&["export", destination.to_str().unwrap(), "--json"]);

    owner_only(&destination, 0o700);

    let files = contents(&destination);
    let transcripts = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .count();
    assert!(transcripts > 0, "the export wrote no transcript: {files:?}");
    assert!(
        destination.join(MANIFEST).is_file(),
        "the export wrote no manifest: {files:?}"
    );
    for file in &files {
        owner_only(file, 0o600);
    }
}
