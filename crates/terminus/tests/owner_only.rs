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
//! Names are spelled out here - `terminus.db`, `LOCK`, `snapshots` - rather
//! than imported from `terminus_core::store`. What these tests assert is what a
//! user finds on disk, and a constant that moved with the code would move the
//! assertion with it.

#![cfg(unix)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use terminus_core::testkit;

/// The encoded project directory a fixture is placed under.
const PROJECT: &str = "-data-projects-cadence";

/// The manifest every export writes, and the one file in it that is not a
/// transcript.
const MANIFEST: &str = "manifest.json";

/// How a spawn is told where its store is.
enum Located {
    /// `TERMINUS_DATA_DIR` names it outright, which is what every other bench
    /// in this crate does and what the export and end-to-end tests want.
    Explicitly,
    /// The PLATFORM data directory, with `TERMINUS_DATA_DIR` removed. `data
    /// move` writes a location pointer and the environment override outranks it
    /// (D-06), so a bench that set the variable would pass whether the move
    /// worked or not - the reason `tests/datamove.rs` gives for the same shape.
    ByPlatform,
}

/// Every directory a spawned `terminus` may touch, all of them temporary.
///
/// The same isolation `tests/lifecycle.rs` uses: a spawn that set only
/// `TERMINUS_DATA_DIR` would resolve the developer's real config and walk the
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
            .join("terminus"),
        Located::ByPlatform => root.join("share").join("terminus"),
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
            .arg(env!("CARGO_BIN_EXE_terminus"))
            .args(args)
            .current_dir(&self.work)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        match self.located {
            Located::Explicitly => {
                command.env("TERMINUS_DATA_DIR", &self.data_dir);
            }
            // Removed rather than merely unset by the bench: the developer
            // running the suite may have it exported.
            Located::ByPlatform => {
                command
                    .env_remove("TERMINUS_DATA_DIR")
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

    /// One hook invocation: the payload on stdin, then EOF, the way Claude Code
    /// writes it - and under the same umask as everything else here, because
    /// the files a hook writes are the point.
    fn hook(&self, event: &str, payload: &Value) -> Output {
        let mut child = self
            .command(&["hook", event])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh runs the hook");
        let line = format!("{payload}\n");
        child
            .stdin
            .take()
            .expect("the hook's stdin")
            .write_all(line.as_bytes())
            .expect("write the payload");
        let out = child.wait_with_output().expect("wait for the hook");
        assert!(
            out.status.success(),
            "hook {event} exited {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
}

/// What is wrong with one path's mode, or nothing.
///
/// A path that is not there at all is wrong rather than a panic: `-wal` and
/// `-shm` exist only while a connection is open, and a run that lost one should
/// say so beside the modes rather than instead of them.
fn wrong(path: &Path, want: u32) -> Option<String> {
    match std::fs::symlink_metadata(path) {
        Err(e) => Some(format!("{} is not there: {e}", path.display())),
        Ok(meta) => {
            let found = meta.permissions().mode() & 0o777;
            (found != want).then(|| format!("{} is {found:o}, want {want:o}", path.display()))
        }
    }
}

/// One path, one assertion, and the path in the message.
#[track_caller]
fn owner_only(path: &Path, want: u32) {
    if let Some(complaint) = wrong(path, want) {
        panic!("{complaint}");
    }
}

/// Every path checked one at a time, and every offender named in ONE failure.
///
/// The end-to-end test needs this and the two single-command tests above do
/// not: it covers nine creation sites in different modules, and a fail-fast
/// assertion there would always stop at the data directory - leaving the reader
/// of a failing run with no idea whether `terminus.db` and the export were
/// repaired or not, which is the most useful fact in the message. Each path is
/// still its own check with its own path in its own line.
#[derive(Default)]
struct Wrong(Vec<String>);

impl Wrong {
    fn check(&mut self, path: &Path, want: u32) {
        self.0.extend(wrong(path, want));
    }

    /// A directory at `DIR_MODE` and every file directly inside it at
    /// `FILE_MODE`. [`contents`] refuses an empty one, so a hook that wrote
    /// nothing fails here rather than passing over an empty loop.
    fn directory(&mut self, dir: &Path, want_dir: u32, want_file: u32) {
        self.check(dir, want_dir);
        for path in contents(dir) {
            self.check(&path, want_file);
        }
    }

    #[track_caller]
    fn none(&self) {
        assert!(
            self.0.is_empty(),
            "readable by group or world:\n  {}",
            self.0.join("\n  ")
        );
    }
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

/// AC1's export clause: `terminus export` writes a directory nobody else on the
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

/// AC1's move clause: `terminus data move` writes the modes this build states
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

    let destination = bench.root.join("elsewhere").join("terminus");
    bench.ok(&["data", "move", destination.to_str().unwrap(), "--yes"]);

    // Named one by one first, so a failure says which site is wrong, and then
    // the whole tree, so a file this test did not think of is covered too.
    owner_only(&destination, 0o700);
    owner_only(&destination.join("terminus.db"), 0o600);
    owner_only(&destination.join("LOCK"), 0o600);
    let moved_snapshots = destination.join("snapshots");
    owner_only(&moved_snapshots, 0o700);
    for snapshot in contents(&moved_snapshots) {
        owner_only(&snapshot, 0o600);
    }
    assert_tree_is_owner_only(&destination);
}

/// The session one payload names, long enough and plain enough to be allowed to
/// name a file (`inject::state`'s allow-list, mirrored in `inject::decision`).
const SESSION_ID: &str = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";

/// AC1 whole: one fresh ingest, one `SessionStart`, one `UserPromptSubmit`, one
/// export, and then every path the phase claims, asserted one at a time.
///
/// Three things this test has to arrange for, none of them incidental:
///
/// **The ingest lock is held by the test process** for everything after the
/// first ingest. Every hook spawns a detached `terminus ingest` of its own, and
/// one that won the lock would drain `decisions/` out of existence between the
/// hook that wrote it and the stat that reads it. With the lock held, a spawned
/// pass exits 0 having written nothing - `ingest/pass.rs`'s
/// `a_pass_reports_the_lock_rather_than_waiting_for_it` is that claim.
///
/// **A read transaction is held open on `terminus.db`** for the same span.
/// SQLite deletes `-wal` and `-shm` when the last connection closes, which is
/// why CONTEXT D-02's umask-022 baseline lists neither: a test that looked for
/// them after the ingest process exited would find nothing to stat. A held
/// reader keeps both on disk. Their mode is SQLite's own doing - it takes it
/// from the main database rather than from the umask of whatever process
/// created them (D-02) - so these two assertions test that flagged assumption
/// directly, and the fact that this process made them is not a hole in the
/// test.
///
/// **The `SessionStart` says `source: "compact"`**, which
/// `inject::brief::owe_compaction` answers by saving the session's state file
/// before it looks at the store at all, so `injection/` is populated whether
/// the brief renders or not. The `UserPromptSubmit` needs no such arrangement:
/// every prompt writes a decision record, including the ones that inject
/// nothing (D-11).
#[test]
fn everything_a_fresh_install_writes_is_owner_only_under_umask_022() {
    let bench = bench();
    bench.place("session-basic.jsonl");
    bench.ok(&["ingest"]);

    let data = bench.data_dir.clone();

    // Held for the rest of the test. Both are dropped at the end of the
    // function, after every assertion has run.
    let _held = match terminus_core::ingest::lock::try_acquire(&data).unwrap() {
        terminus_core::ingest::lock::Attempt::Acquired(held) => held,
        terminus_core::ingest::lock::Attempt::Held => panic!("the ingest lock is already taken"),
    };
    let conn = rusqlite::Connection::open(data.join("terminus.db")).unwrap();
    conn.execute_batch("BEGIN").unwrap();
    let sessions: i64 = conn
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sessions, 1, "the ingest archived nothing to hold open");

    bench.hook(
        "SessionStart",
        &json!({
            "session_id": SESSION_ID,
            "transcript_path": "/home/user/.claude/projects/-p/session.jsonl",
            "cwd": bench.work.to_str().unwrap(),
            "hook_event_name": "SessionStart",
            "source": "compact",
        }),
    );
    bench.hook(
        "UserPromptSubmit",
        &json!({
            "session_id": SESSION_ID,
            "transcript_path": "/home/user/.claude/projects/-p/session.jsonl",
            "cwd": bench.work.to_str().unwrap(),
            "prompt_id": "7c1d4b90-33af-4e05-9a6c-2f8e5b71c0a4",
            "hook_event_name": "UserPromptSubmit",
            "prompt": "where did we settle the detached spawn's kill behaviour",
            "session_title": "phase 3: owner-only on disk",
        }),
    );

    let destination = bench.root.join("export");
    bench.ok(&["export", destination.to_str().unwrap(), "--json"]);

    let mut wrong = Wrong::default();

    // The data directory itself, and the four files that sit directly in it.
    wrong.check(&data, 0o700);
    wrong.check(&data.join("terminus.db"), 0o600);
    wrong.check(&data.join("terminus.db-wal"), 0o600);
    wrong.check(&data.join("terminus.db-shm"), 0o600);
    wrong.check(&data.join("LOCK"), 0o600);

    // Every directory below it, each one required to hold something.
    for name in ["snapshots", "injection", "decisions"] {
        wrong.directory(&data.join(name), 0o700, 0o600);
    }

    wrong.directory(&destination, 0o700, 0o600);
    wrong.none();
}
