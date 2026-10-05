//! The hook path: what Claude Code calls, what it costs, and what survives it.
//!
//! Two properties are worth more than the rest and they are both about
//! processes rather than about output. The hook must return to the harness
//! having written nothing and holding nothing, and the ingest it started must
//! outlive both of the kills Claude Code aims at a hook it has given up on
//! (D-03). Neither is provable from a return value, so these tests read the
//! process table.
//!
//! Every spawn here sets `TERMINUS_DATA_DIR`, `TERMINUS_CONFIG_DIR` **and**
//! `CLAUDE_CONFIG_DIR` at temporary directories, the same rule
//! `crates/terminus/tests/cli.rs` states: a bare `terminus ingest` walks the
//! *configured* roots, so a spawn that set only the data directory would
//! resolve the developer's real config and walk a live 2,000-file `~/.claude`
//! tree.
//!
//! The kill test needs the fault points, so it needs
//! `cargo test -p terminus --features testkit --test hook`. It is gated on the
//! feature by itself rather than the file being gated, because a
//! `#![cfg(feature = "testkit")]` binary runs zero tests under a bare
//! `cargo test` and still reports green - which would silently retire the
//! budget assertion this file exists to hold.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use terminus_core::store::DB_FILE_NAME;
use terminus_core::testkit;

/// The first argument that asks for the reparenting hand-off. This is
/// `cmd::spawn::HANDOFF` spelled again: `terminus` is a binary crate with no
/// library target, so a test cannot name the constant and has to agree with it.
/// Changing one without the other makes this test's hand-off an unexpected
/// argument (exit 2), which is the failure mode that says which way the drift
/// went.
const HANDOFF: &str = "--reparent";

/// The environment variable the hand-off used to be selected by, kept here with
/// no code left that reads it. It names the regression
/// `an_ambient_environment_marker_cannot_divert_an_ordinary_invocation` exists
/// to hold shut.
const RETIRED_MARKER: &str = "TERMINUS_REPARENT";

/// Every directory a spawned hook may reach, all temporary, plus a copy of the
/// binary at a path no other test uses.
///
/// The copy is what makes a `ps` scan able to name *this* test's ingest. Every
/// hook spawns the identical command line - `<current_exe> ingest` - so two
/// tests running side by side against the shared `CARGO_BIN_EXE_terminus` path
/// are indistinguishable in the process table, and `std::env::current_exe()`
/// reports whichever copy was actually executed.
struct Hook {
    _dir: tempfile::TempDir,
    exe: PathBuf,
    data_dir: PathBuf,
    config_dir: PathBuf,
    claude_dir: PathBuf,
}

fn hook() -> Hook {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let config_dir = dir.path().join("config");
    let claude_dir = dir.path().join("claude");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::create_dir_all(&bin).unwrap();

    let exe = bin.join(if cfg!(windows) {
        "terminus.exe"
    } else {
        "terminus"
    });
    // `fs::copy` carries the permission bits, so the copy is executable. The
    // guard is what keeps a sibling test's fork out of the window where this
    // destination is open for writing (see [`COPYING`]).
    {
        let _guard = COPYING.write().unwrap_or_else(PoisonError::into_inner);
        std::fs::copy(env!("CARGO_BIN_EXE_terminus"), &exe).unwrap();
    }

    Hook {
        _dir: dir,
        exe,
        data_dir,
        config_dir,
        claude_dir,
    }
}

impl Hook {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.exe);
        command
            .args(args)
            .env("TERMINUS_DATA_DIR", &self.data_dir)
            .env("TERMINUS_CONFIG_DIR", &self.config_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir);
        command
    }

    /// The command line every spawn of this test's binary shows in `ps`.
    fn ingest_argv(&self) -> String {
        format!("{} ingest", self.exe.display())
    }

    /// A transcript big enough that the pass is still running while the process
    /// table is read. A 1.3 MB transcript ingests in ~90 ms in a debug build,
    /// so this one buys roughly two thirds of a second - long enough that a
    /// `ps` call lands inside it, short enough to pay for on every run.
    fn big_transcript(&self, project: &str, name: &str, repeats: usize) -> PathBuf {
        let mut bytes = Vec::new();
        for _ in 0..repeats {
            bytes.extend_from_slice(&testkit::fixture_bytes("session-basic.jsonl"));
            bytes.extend_from_slice(&testkit::fixture_bytes("session-large-record.jsonl"));
        }
        let path = self.claude_dir.join("projects").join(project).join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        path
    }
}

/// Held for writing while a binary copy is being made, and for reading across
/// every `fork` this file performs, so the two never overlap.
///
/// Without it these tests fail as `ETXTBSY` ("Text file busy") on an exec, at
/// whichever `spawn` lost the race. The mechanism is the copy above: while
/// `fs::copy` is writing one test's binary the destination is open for writing,
/// and a *sibling* test's `Command::spawn` on another thread forks the whole
/// process, descriptors included. Between that child's fork and its exec it
/// holds a write handle on a file it will never touch - close-on-exec closes it
/// a moment later, but a moment is enough - and Linux refuses to exec a file any
/// process holds open for writing. The copy and the fork are both blameless; it
/// is only their overlap that is the bug, so the fix is to forbid the overlap
/// rather than to retry the exec.
///
/// A read guard rather than a mutex because spawning is what this file mostly
/// does: hook spawns still run concurrently with each other, they are only kept
/// out of the window where a copy is in flight.
static COPYING: RwLock<()> = RwLock::new(());

/// `Command::spawn`, held off while any binary copy is in flight ([`COPYING`]).
///
/// Every spawn in this file goes through here or through [`output`] - including
/// `ps` and `kill`, which fork exactly like the ones that matter and inherit
/// exactly the same descriptors.
///
/// The guard is dropped when `spawn` returns, which is after the child has
/// exec'd: both of the paths `std` takes on Unix (`posix_spawn`, and the
/// `fork` fallback with its close-on-exec status pipe) report back only once
/// the exec has happened or failed.
fn spawn(command: &mut Command) -> std::io::Result<Child> {
    let _guard = COPYING.read().unwrap_or_else(PoisonError::into_inner);
    command.spawn()
}

/// `Command::output` over [`spawn`]: the same stdio `Command::output` sets, and
/// the wait happens after the guard is dropped.
fn output(command: &mut Command) -> std::io::Result<Output> {
    let child = spawn(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    child.wait_with_output()
}

/// Repeats of the two fixtures, ~10 MB, ~700 ms of debug-build ingest.
const BIG: usize = 48;

/// Every event Claude Code fires at this binary, against the payload it sends.
/// A fifth event is a line here and a line in `cmd::hook::EVENTS`.
const FIXTURES: &[(&str, &str)] = &[
    ("SessionStart", "hooks/session-start.json"),
    ("UserPromptSubmit", "hooks/user-prompt-submit.json"),
    ("SessionEnd", "hooks/session-end.json"),
    ("PostCompact", "hooks/post-compact.json"),
];

/// One hook invocation: the payload on stdin, then EOF, the way Claude Code
/// 2.1.231 writes it (`stdin.write(payload + "\n"); stdin.end()`).
fn feed(hook: &Hook, event: &str, payload: &[u8]) -> Output {
    let mut child = spawn(
        hook.command(&["hook", event])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .expect("spawn the hook");
    child
        .stdin
        .take()
        .expect("the hook's stdin")
        .write_all(payload)
        .expect("write the payload");
    child.wait_with_output().expect("wait for the hook")
}

/// One row of the process table.
#[cfg(unix)]
#[derive(Debug, Clone)]
struct Proc {
    pid: u32,
    ppid: u32,
    pgid: u32,
    argv: String,
}

/// The whole process table in one snapshot.
///
/// One `ps` call rather than one per question: a process that exits between two
/// calls turns a real answer into a flaky one, and the group, the parent and
/// the command line have to describe the same instant to mean anything.
/// `-ww` is what keeps a long temporary path in `argv` from being truncated.
#[cfg(unix)]
fn ps() -> Vec<Proc> {
    let listing = output(Command::new("ps").args(["-e", "-ww", "-o", "pid=,ppid=,pgid=,args="]))
        .expect("run ps");
    assert!(listing.status.success(), "ps failed");
    String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let pgid = fields.next()?.parse().ok()?;
            let rest = fields.collect::<Vec<_>>().join(" ");
            Some(Proc {
                pid,
                ppid,
                pgid,
                argv: rest,
            })
        })
        .collect()
}

/// Poll the process table until one process is running exactly `argv`.
#[cfg(unix)]
fn await_process(argv: &str) -> Proc {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let mut found: Vec<Proc> = ps().into_iter().filter(|p| p.argv == argv).collect();
        // Exactly one, always: the hand-off has exited by the time its child is
        // visible, and two would mean the path is not unique after all - which
        // would make every assertion below true of an arbitrary process.
        if found.len() == 1 {
            return found.pop().unwrap();
        }
        assert!(found.len() < 2, "two processes running {argv:?}: {found:?}");
        assert!(
            std::time::Instant::now() < deadline,
            "no process ever ran {argv:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Task 1 / D-03: the process that does the work is in neither set Claude Code
/// kills a hook through.
///
/// It is spawned through the hand-off half of `cmd::spawn` directly - the
/// marker in front of the arguments is what a hook's spawn puts there - so this
/// reads the property off one level of the chain. The kill test reads it off
/// the whole chain.
#[cfg(unix)]
#[test]
fn the_working_process_leaves_the_group_and_the_parentage_of_the_process_that_spawned_it() {
    let hook = hook();
    hook.big_transcript("-p", "11111111-1111-4111-8111-111111111111.jsonl", BIG);

    let handoff = output(&mut hook.command(&[HANDOFF, "ingest"])).expect("spawn the hand-off");
    assert!(handoff.status.success(), "the hand-off did not exit 0");
    assert!(handoff.stdout.is_empty(), "the hand-off wrote to stdout");

    let worker = await_process(&hook.ingest_argv());
    assert_eq!(
        worker.pgid, worker.pid,
        "the ingest leads no group of its own, so kill(-pgid) on the hook's \
         group would reach it: {worker:?}"
    );
    assert_ne!(
        worker.ppid,
        std::process::id(),
        "the ingest is a child of the test process: {worker:?}"
    );
    // The hand-off has exited, so its pid is free to be reused, but a reused
    // pid cannot be this child's parent: an exited process's children are
    // reparented, never re-adopted.
    assert_ne!(
        worker.ppid, worker.pid,
        "nonsense parentage in the process table: {worker:?}"
    );
    assert!(
        !ps().iter().any(|p| p.pid == worker.ppid && p.argv == hook.ingest_argv()),
        "the ingest is still parented to the process that spawned it"
    );

    drain(&hook.ingest_argv());
}

/// Nothing a caller's environment carries can put an ordinary invocation on the
/// hand-off path.
///
/// The hand-off used to be selected by [`RETIRED_MARKER`], read before argument
/// parsing, so any process that inherited it - a shell export, a `.envrc`, a CI
/// job, a hook that leaked its own child's environment - made `terminus`
/// re-spawn itself and return SUCCESS with an empty stdout for *every* command.
/// `--version` printed nothing, `search` printed nothing, and an unknown hook
/// event reported success where the contract says misuse (exit 2), which is the
/// one exit code `terminus install`'s settings file is checked by.
///
/// A first-argument marker is inherited by nothing, so the two contracts
/// `main.rs` opens with hold whatever the environment says. That the marker
/// also does not survive into the working process is proved next door: the
/// hand-off test finds its ingest by an exact `<exe> ingest` command line, and
/// a marker passed on would make that argv `<exe> --reparent ingest` and never
/// match.
#[test]
fn an_ambient_environment_marker_cannot_divert_an_ordinary_invocation() {
    let hook = hook();

    let version = output(hook.command(&["--version"]).env(RETIRED_MARKER, "1"))
        .expect("run terminus --version");
    assert!(
        version.status.success(),
        "--version exited {:?} under {RETIRED_MARKER}",
        version.status.code()
    );
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        env!("CARGO_PKG_VERSION"),
        "--version printed nothing of its own under {RETIRED_MARKER}: the \
         invocation was diverted"
    );

    let unknown = output(hook.command(&["hook", "NotAnEvent"]).env(RETIRED_MARKER, "1"))
        .expect("run terminus hook NotAnEvent");
    assert_eq!(
        unknown.status.code(),
        Some(2),
        "an unknown hook event exited {:?} under {RETIRED_MARKER}, not the \
         misuse code install's settings file is checked by",
        unknown.status.code()
    );
    assert!(
        unknown.stdout.is_empty(),
        "the misuse path wrote {:?} to stdout",
        String::from_utf8_lossy(&unknown.stdout)
    );

    drain(&hook.ingest_argv());
}

/// Wait for every ingest this test started to finish.
///
/// Not tidiness: the temporary directory is removed when the test returns, and
/// a pass still walking it would be reading a tree that is being deleted under
/// it.
#[cfg(unix)]
fn drain(argv: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while ps().iter().any(|p| p.argv == argv) {
        assert!(Instant::now() < deadline, "an ingest never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(unix))]
fn drain(_argv: &str) {}

/// AC1: the four events, the exact payloads, the clean stdout and the budget.
///
/// The p99 is printed rather than only asserted, so a regression names a number
/// a reader can compare against the next run instead of a boolean.
#[test]
fn every_event_exits_zero_with_an_empty_stdout_inside_the_budget() {
    const RUNS: usize = 100;
    const BUDGET: f64 = 10.0;

    let hook = hook();
    for (event, fixture) in FIXTURES {
        let payload = testkit::fixture_bytes(fixture);
        let mut millis = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let started = Instant::now();
            let output = feed(&hook, event, &payload);
            millis.push(started.elapsed().as_secs_f64() * 1000.0);

            assert!(
                output.status.success(),
                "{event} exited {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
            // D-15: Claude Code validates any hook stdout that parses as JSON
            // against the event name, so a stray line is a protocol error and
            // not noise.
            assert!(
                output.stdout.is_empty(),
                "{event} wrote to stdout: {:?}",
                String::from_utf8_lossy(&output.stdout)
            );
        }

        millis.sort_by(f64::total_cmp);
        let p50 = millis[RUNS / 2 - 1];
        let p99 = millis[(RUNS * 99) / 100 - 1];
        println!("{event}: p50 {p50:.2} ms, p99 {p99:.2} ms over {RUNS} runs");
        assert!(p99 < BUDGET, "{event}: p99 {p99:.2} ms is over {BUDGET} ms");
    }
    drain(&hook.ingest_argv());
}

/// A writer that opens stdin, sends no newline and never closes it does not get
/// to decide when the hook returns.
///
/// This is the state the harness itself is never in - Claude Code 2.1.231
/// writes the payload and calls `stdin.end()` in the same tick - which is
/// exactly why it has to be tested: nothing in ordinary use would ever show it.
/// An unbounded blocking read here sits until the harness's hook timeout kills
/// the process, and on `UserPromptSubmit` that timeout is the user's prompt
/// waiting.
///
/// The bound is 500 ms in `cmd::hook`; the 10 s here is that plus room for a
/// loaded machine, and it is a bound rather than an unbounded `wait()` so a
/// regression fails the test instead of hanging the suite.
#[test]
fn a_stdin_that_is_opened_and_never_closed_does_not_hold_the_hook() {
    let hook = hook();

    let mut child = spawn(
        hook.command(&["hook", "UserPromptSubmit"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .expect("spawn the hook");
    // A payload that has begun and will never end: no newline, and the handle
    // stays in scope for the whole test, so the hook's stdin never sees EOF.
    let mut stdin = child.stdin.take().expect("the hook's stdin");
    stdin
        .write_all(br#"{"session_id":"11111111-1111-4111-8111-111111111111""#)
        .expect("write a partial payload");
    stdin.flush().expect("flush the partial payload");

    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the hook") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the hook is still running {:?} after a writer stopped writing: it \
             is blocked in an unbounded read on stdin",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(
        status.success(),
        "the hook exited {:?} on an unclosed stdin",
        status.code()
    );
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .expect("the hook's stdout")
        .read_to_end(&mut stdout)
        .expect("read the hook's stdout");
    assert!(stdout.is_empty(), "the hook wrote {stdout:?} to stdout");
    // The module doc's promise for this case: one line on stderr, still exit 0.
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("the hook's stderr")
        .read_to_string(&mut stderr)
        .expect("read the hook's stderr");
    assert!(
        stderr.lines().count() == 1 && stderr.contains("stdin"),
        "giving up on stdin should be one line on stderr, got {stderr:?}"
    );

    drop(stdin);
    drain(&hook.ingest_argv());
}

/// A line that never ends is read up to a cap and no further.
///
/// The hook reads the payload only so the writer does not see a closed pipe.
/// Nothing downstream reads a byte of it - the ingest was spawned before the
/// read - so an unbounded `read_until` on a line with no newline in it is a
/// `Vec` that grows for as long as something keeps writing.
///
/// 64 MiB of unterminated line against a 1 MiB cap: the hook must stop reading
/// and exit, which the writer sees as a broken pipe well before it has handed
/// over everything. The byte count is what separates the cap from the deadline:
/// 500 ms of an uncapped pipe copy would move far more than the 8 MiB asserted
/// here.
#[test]
fn an_unterminated_line_stops_being_read_at_the_cap() {
    const CHUNK: usize = 64 * 1024;
    const TOTAL: usize = 64 * 1024 * 1024;
    const CEILING: usize = 8 * 1024 * 1024;

    let hook = hook();
    let mut child = spawn(
        hook.command(&["hook", "SessionStart"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .expect("spawn the hook");
    let mut stdin = child.stdin.take().expect("the hook's stdin");

    // No newline anywhere in it, so nothing but a cap can end the read.
    let chunk = vec![b'x'; CHUNK];
    let mut written = 0usize;
    let mut refused = None;
    while written < TOTAL {
        match stdin.write_all(&chunk) {
            Ok(()) => written += CHUNK,
            Err(e) => {
                refused = Some(e);
                break;
            }
        }
    }

    let e = refused.unwrap_or_else(|| {
        panic!("the hook read all {TOTAL} bytes of a line that never ends");
    });
    assert_eq!(
        e.kind(),
        std::io::ErrorKind::BrokenPipe,
        "the writer stopped for a reason other than the hook exiting: {e}"
    );
    assert!(
        written <= CEILING,
        "the hook read {written} bytes of an unterminated line before it \
         stopped: that is past any cap worth calling one"
    );

    let output = child.wait_with_output().expect("wait for the hook");
    assert!(
        output.status.success(),
        "the hook exited {:?} on an oversized payload",
        output.status.code()
    );
    assert!(
        output.stdout.is_empty(),
        "the hook wrote {:?} to stdout",
        String::from_utf8_lossy(&output.stdout)
    );

    drop(stdin);
    drain(&hook.ingest_argv());
}

/// The hook hands the ingest nothing the harness gave it.
///
/// A caller reading the hook's stdout to EOF must get an empty read as soon as
/// the hook exits. A child holding the inherited write end of that pipe would
/// keep the read blocked for the whole pass instead - which on a backfill is
/// minutes of a harness waiting on a hook that already returned.
#[cfg(unix)]
#[test]
fn reading_the_hooks_stdout_to_eof_returns_at_once_while_the_ingest_still_runs() {
    let hook = hook();
    hook.big_transcript("-p", "22222222-2222-4222-8222-222222222222.jsonl", BIG);

    let mut child = spawn(
        hook.command(&["hook", "SessionStart"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped()),
    )
    .expect("spawn the hook");
    child
        .stdin
        .take()
        .expect("the hook's stdin")
        .write_all(&testkit::fixture_bytes("hooks/session-start.json"))
        .expect("write the payload");

    let started = Instant::now();
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .expect("the hook's stdout")
        .read_to_end(&mut stdout)
        .expect("read the hook's stdout to EOF");
    let elapsed = started.elapsed();

    assert!(stdout.is_empty(), "the hook wrote {stdout:?} to stdout");
    // The pass this hook started runs for roughly 700 ms, so anything near it
    // means the read was waiting on the ingest and not on the hook.
    assert!(
        elapsed < Duration::from_millis(250),
        "reading the hook's stdout to EOF took {elapsed:?}: the ingest is \
         holding a handle the hook inherited"
    );
    assert!(child.wait().expect("wait for the hook").success());

    // And it returned *while* the pass was running, which is what makes the
    // reading above an answer rather than a coincidence.
    await_process(&hook.ingest_argv());
    drain(&hook.ingest_argv());
}

/// The tree the kill test walks: four transcripts, one project, ~1.4 s of
/// debug-build pass. Big enough that the whole of it is still ahead of the
/// ingest when the kill lands, so finishing it is what proves the survival.
#[cfg(unix)]
const TREE: usize = 4;

/// Sessions committed so far, read through a second connection while the pass
/// is running - which is what WAL is on for.
///
/// Read-only and never `Connection::open`: an ordinary open would *create* the
/// database file, and this poll runs before the ingest has made one.
#[cfg(unix)]
fn committed(hook: &Hook) -> usize {
    let db = hook.data_dir.join(DB_FILE_NAME);
    if !db.is_file() {
        return 0;
    }
    let Ok(conn) = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return 0;
    };
    conn.query_row("SELECT count(*) FROM sessions", [], |row| {
        row.get::<_, i64>(0)
    })
    .map(|n| n as usize)
    .unwrap_or(0)
}

/// Every pid below `root`, breadth first, the way Claude Code 2.1.231 builds
/// the list it kills: `pid ppid` pairs out of `ps`, walked from the hook.
#[cfg(unix)]
fn descendants(root: u32, table: &[Proc]) -> Vec<u32> {
    let mut found: Vec<u32> = Vec::new();
    let mut frontier = vec![root];
    while let Some(pid) = frontier.pop() {
        for child in table.iter().filter(|p| p.ppid == pid) {
            if !found.contains(&child.pid) {
                found.push(child.pid);
                frontier.push(child.pid);
            }
        }
    }
    found
}

#[cfg(unix)]
fn sigkill(target: &str) {
    output(Command::new("kill").args(["-KILL", target])).expect("run kill");
}

/// AC2 / D-03: the kill Claude Code actually performs, and an ingest that
/// finishes anyway.
///
/// The hook is left blocked on a stdin nobody closes, which is the only state a
/// hook is ever killed in - Claude Code kills the ones that time out. It is
/// spawned into a process group of its own for the same reason: `kill(-pid)`
/// against a hook sharing the harness's group would take the harness with it,
/// so a harness that kills that way spawns that way.
///
/// The descendant set is taken from a snapshot while the hook is still alive
/// AND again after the group kill, and every pid in either is killed. That is a
/// superset of what Claude Code kills, which is the point: the ingest has to be
/// outside the union, not merely outside whichever snapshot happened to be
/// taken first.
///
/// **Everything between the spawn and the kill is on a 500 ms clock**, and that
/// is the whole reason this test is shaped the way it is. The hook gives a
/// writer `cmd::hook::DRAIN_DEADLINE` - 500 ms - before it stops waiting on
/// stdin and exits 0, so a hook that is killed late is not killed at all: the
/// group kill reaches an empty group, nothing dies, and the survival assertions
/// below pass over a hook that was never killed. So the kill is aimed as soon
/// as the ingest is nameable in the process table, tens of milliseconds in,
/// rather than after waiting for the pass to commit its first file - which cost
/// ~400 ms of the 500 on an idle machine and ran past it under a loaded one.
/// Nothing is lost by killing early, because `SIGKILL` cannot be handled: what
/// decides survival is whether the ingest's pid is in the group or the sweep,
/// which its `setsid` settled at spawn and no later instant can change. The
/// pass being wholly unfinished at the kill is asserted rather than waited for,
/// and the drain below then proves it ran the entire tree afterwards.
#[cfg(unix)]
#[test]
fn the_ingest_survives_a_group_kill_and_a_descendant_sweep() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    let hook = hook();
    for i in 0..TREE {
        hook.big_transcript(
            "-p",
            &format!("3333333{i}-3333-4333-8333-333333333333.jsonl"),
            24,
        );
    }

    let mut child = spawn(
        hook.command(&["hook", "SessionEnd"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Its own group, so the group kill below names the hook and nothing
            // else. Claude Code spawns hooks this way for the same reason.
            .process_group(0),
    )
    .expect("spawn the hook");
    // Held open and never written: the hook is now blocked in its one-line
    // read, exactly where a timed-out hook is when the harness gives up on it.
    let _stdin = child.stdin.take().expect("the hook's stdin");
    let hook_pid = child.id();

    // Nameable in the process table is all the kill needs; the pass being
    // unfinished is read, not waited for. A kill that lands after the pass has
    // committed everything would prove nothing at all.
    let ingest = await_process(&hook.ingest_argv());
    let mid_pass = committed(&hook);
    assert!(mid_pass < TREE, "the pass finished before the kill: {mid_pass}");

    let mut doomed = descendants(hook_pid, &ps());
    // The group, by its leader's negated pid - `process.kill(-pid)`.
    sigkill(&format!("-{hook_pid}"));
    for pid in descendants(hook_pid, &ps()) {
        if !doomed.contains(&pid) {
            doomed.push(pid);
        }
    }
    for pid in &doomed {
        sigkill(&pid.to_string());
    }

    let status = child.wait().expect("wait for the killed hook");
    assert_eq!(
        status.signal(),
        Some(9),
        "the hook did not die of the group kill: {status:?}. An exit of 0 here \
         means the hook gave up on stdin and returned before the kill landed, \
         so the group was empty and nothing below is evidence of anything"
    );
    assert!(
        !doomed.contains(&ingest.pid),
        "the descendant sweep named the ingest: {ingest:?} in {doomed:?}"
    );
    assert!(
        ps().iter().any(|p| p.pid == ingest.pid),
        "the ingest did not survive: {ingest:?}"
    );

    // And it does not merely survive: it finishes the tree and records the pass.
    drain(&hook.ingest_argv());
    let db = hook.data_dir.join(DB_FILE_NAME);
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open the store read-only");
    assert_eq!(
        committed(&hook),
        TREE,
        "the store is missing sessions the pass should have committed"
    );
    let (files, error): (i64, Option<String>) = conn
        .query_row(
            "SELECT files_committed, error FROM runs ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the pass wrote no runs row");
    assert_eq!(files as usize, TREE, "the runs row is short of the tree");
    assert_eq!(error, None, "the pass recorded an error: {error:?}");
}

/// PRIV-03's knob, written the only way anything can turn it on.
const KNOB_ON: &str = "[privacy]\nredact_recall = true\n";

/// AC7: the same four events, the same hundred runs, the same budget, with
/// `[privacy] redact_recall = true` on the config the hook loads.
///
/// The knob puts the egress rule set on the `SessionStart` path, between the
/// projection and the budget, and D-06 measured that at roughly 1.1 ms worst
/// case for the brief's two quotes. That is a claim about a cost, so it is
/// asserted where the cost is paid: the wall clock of a spawn the harness
/// waits on, against the same 10 ms the run without the knob is held to.
///
/// It lives in this file rather than in one of its own because [`COPYING`] is
/// what keeps ETXTBSY out of a fully parallel suite - the lock is held between
/// this file's binary copy and every spawn made from it, and a second file
/// copying the binary would sit outside it.
#[test]
fn every_event_stays_inside_the_budget_with_the_redaction_knob_on() {
    const RUNS: usize = 100;
    const BUDGET: f64 = 10.0;

    let hook = hook();
    std::fs::write(hook.config_dir.join("terminus.toml"), KNOB_ON).unwrap();

    for (event, fixture) in FIXTURES {
        let payload = testkit::fixture_bytes(fixture);
        let mut millis = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let started = Instant::now();
            let output = feed(&hook, event, &payload);
            millis.push(started.elapsed().as_secs_f64() * 1000.0);

            assert!(
                output.status.success(),
                "{event} with the knob on exited {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.stdout.is_empty(),
                "{event} with the knob on wrote to stdout: {:?}",
                String::from_utf8_lossy(&output.stdout)
            );
        }

        millis.sort_by(f64::total_cmp);
        let p50 = millis[RUNS / 2 - 1];
        let p99 = millis[(RUNS * 99) / 100 - 1];
        println!("{event} (redact_recall): p50 {p50:.2} ms, p99 {p99:.2} ms over {RUNS} runs");
        assert!(
            p99 < BUDGET,
            "{event} with the knob on: p99 {p99:.2} ms is over {BUDGET} ms"
        );
    }
    drain(&hook.ingest_argv());
}
