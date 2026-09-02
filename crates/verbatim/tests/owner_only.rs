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

/// How a spawn is told where its store is.
enum Located {
    /// `VERBATIM_DATA_DIR` names it outright, which is what every other bench
    /// in this crate does and what the export and end-to-end tests want.
    Explicitly,
    /// The PLATFORM data directory, with `VERBATIM_DATA_DIR` removed. `data
    /// move` writes a location pointer and the environment override outranks it
    /// (D-06), so a bench that set the variable would pass whether the move
    /// worked or not - the reason `tests/datamove.rs` gives for the same shape.
    ByPlatform,
}

/// Every directory a spawned `verbatim` may touch, all of them temporary.
///
/// The same isolation `tests/lifecycle.rs` uses: a spawn that set only
/// `VERBATIM_DATA_DIR` would resolve the developer's real config and walk the
/// live `~/.claude` tree.
struct Bench {
    _dir: tempfile::TempDir,
    /// The temporary root, which is where an export or a move destination goes.
    root: PathBuf,
    data_dir: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
    work: PathBuf,
    located: Located,
}

fn bench() -> Bench {
    bench_located(Located::Explicitly)
}

fn bench_located(located: Located) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let config_dir = root.join("config");
    let claude_dir = root.join("claude");
    let work = root.join("work");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let data_dir = match located {
        Located::Explicitly => root.join("data"),
        // `store::open::platform_data_dir`'s own answer on this target.
        Located::ByPlatform if cfg!(target_os = "macos") => root
            .join("Library")
            .join("Application Support")
            .join("verbatim"),
        Located::ByPlatform => root.join("share").join("verbatim"),
    };
    Bench {
        _dir: dir,
        data_dir,
        config_dir,
        claude_dir,
        work,
        root,
        located,
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
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        match self.located {
            Located::Explicitly => {
                command.env("VERBATIM_DATA_DIR", &self.data_dir);
            }
            // Removed rather than merely unset by the bench: the developer
            // running the suite may have it exported.
            Located::ByPlatform => {
                command
                    .env_remove("VERBATIM_DATA_DIR")
                    .env("XDG_DATA_HOME", self.root.join("share"))
                    .env("HOME", &self.root);
            }
        }
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

/// What a build before this phase left on disk: every directory 0755, every
/// file 0644.
///
/// The source is widened DELIBERATELY, because `std::fs::copy` reproduces the
/// mode it reads and a source that was already owner-only would let the old
/// code pass. This is the falsifying half of the test below.
fn widen(dir: &Path) {
    set(dir, 0o755);
    for path in contents(dir) {
        match path.is_dir() {
            true => widen(&path),
            false => set(&path, 0o644),
        }
    }
}

fn set(path: &Path, bits: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits))
        .unwrap_or_else(|e| panic!("chmod {}: {e}", path.display()));
}

/// Every directory 0700 and every file 0600, all the way down.
///
/// Its own `read_dir` rather than [`contents`]: a subdirectory that happens to
/// be empty is a fine thing to find in a moved tree, and the non-emptiness the
/// tests care about is asserted where it means something.
fn assert_tree_is_owner_only(dir: &Path) {
    owner_only(dir, 0o700);
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        match path.is_dir() {
            true => assert_tree_is_owner_only(&path),
            false => owner_only(&path, 0o600),
        }
    }
}

/// AC1's move clause: `verbatim data move` writes the modes this build states
/// rather than the modes it found (D-10).
///
/// The one command that reads a mode off one place and writes it to another.
/// Left as `std::fs::copy`, it is the single path that can undo the whole
/// phase: a store carried off an older build, or restored out of a tar, would
/// arrive at its new home exactly as wide as it left. So the source here is
/// deliberately made 755/644 first and the destination is asserted owner-only
/// anyway - and the `LOCK` the move creates fresh at the destination is on the
/// list, because it is the one file in the new directory that was not copied.
#[test]
fn a_data_move_writes_an_owner_only_tree_however_wide_the_source_was() {
    let bench = bench_located(Located::ByPlatform);
    bench.place("session-basic.jsonl");
    bench.ok(&["ingest"]);

    let source = bench.data_dir.clone();
    // The first pass takes a snapshot (STOR-06), which is what puts a
    // SUBDIRECTORY in the tree about to move - `contents` refuses an empty one,
    // so a pass that took none fails here rather than leaving the recursive
    // half of `copy_tree` untested.
    contents(&source.join("snapshots"));
    widen(&source);

    let destination = bench.root.join("elsewhere").join("verbatim");
    bench.ok(&["data", "move", destination.to_str().unwrap(), "--yes"]);

    // Named one by one first, so a failure says which site is wrong, and then
    // the whole tree, so a file this test did not think of is covered too.
    owner_only(&destination, 0o700);
    owner_only(&destination.join("verbatim.db"), 0o600);
    owner_only(&destination.join("LOCK"), 0o600);
    let moved_snapshots = destination.join("snapshots");
    owner_only(&moved_snapshots, 0o700);
    for snapshot in contents(&moved_snapshots) {
        owner_only(&snapshot, 0o600);
    }
    assert_tree_is_owner_only(&destination);
}
