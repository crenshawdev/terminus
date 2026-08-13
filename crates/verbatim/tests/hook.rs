//! The hook path: what Claude Code calls, what it costs, and what survives it.
//!
//! Two properties are worth more than the rest and they are both about
//! processes rather than about output. The hook must return to the harness
//! having written nothing and holding nothing, and the ingest it started must
//! outlive both of the kills Claude Code aims at a hook it has given up on
//! (D-03). Neither is provable from a return value, so these tests read the
//! process table.
//!
//! Every spawn here sets `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` **and**
//! `CLAUDE_CONFIG_DIR` at temporary directories, the same rule
//! `crates/verbatim/tests/cli.rs` states: a bare `verbatim ingest` walks the
//! *configured* roots, so a spawn that set only the data directory would
//! resolve the developer's real config and walk a live 2,000-file `~/.claude`
//! tree.
//!
//! The kill test needs the fault points, so it needs
//! `cargo test -p verbatim --features testkit --test hook`. It is gated on the
//! feature by itself rather than the file being gated, because a
//! `#![cfg(feature = "testkit")]` binary runs zero tests under a bare
//! `cargo test` and still reports green - which would silently retire the
//! budget assertion this file exists to hold.

use std::path::PathBuf;
use std::process::Command;

use verbatim_core::testkit;

/// Marks the reparenting hand-off. This is `cmd::spawn::REPARENT` spelled
/// again: `verbatim` is a binary crate with no library target, so a test cannot
/// name the constant and has to agree with it. Changing one without the other
/// makes this test's hand-off run an ordinary ingest and the pgid assertion
/// fail, which is the failure mode that says which way the drift went.
const REPARENT: &str = "VERBATIM_REPARENT";

/// Every directory a spawned hook may reach, all temporary, plus a copy of the
/// binary at a path no other test uses.
///
/// The copy is what makes a `ps` scan able to name *this* test's ingest. Every
/// hook spawns the identical command line - `<current_exe> ingest` - so two
/// tests running side by side against the shared `CARGO_BIN_EXE_verbatim` path
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
        "verbatim.exe"
    } else {
        "verbatim"
    });
    // `fs::copy` carries the permission bits, so the copy is executable.
    std::fs::copy(env!("CARGO_BIN_EXE_verbatim"), &exe).unwrap();

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
            .env("VERBATIM_DATA_DIR", &self.data_dir)
            .env("VERBATIM_CONFIG_DIR", &self.config_dir)
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

/// Repeats of the two fixtures, ~10 MB, ~700 ms of debug-build ingest.
const BIG: usize = 48;

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
    let output = Command::new("ps")
        .args(["-e", "-ww", "-o", "pid=,ppid=,pgid=,args="])
        .output()
        .expect("run ps");
    assert!(output.status.success(), "ps failed");
    String::from_utf8_lossy(&output.stdout)
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
/// It is spawned through the hand-off half of `cmd::spawn` directly - the mark
/// on the environment is what a hook's spawn sets - so this reads the property
/// off one level of the chain. The kill test reads it off the whole chain.
#[cfg(unix)]
#[test]
fn the_working_process_leaves_the_group_and_the_parentage_of_the_process_that_spawned_it() {
    let hook = hook();
    hook.big_transcript("-p", "11111111-1111-4111-8111-111111111111.jsonl", BIG);

    let handoff = hook
        .command(&["ingest"])
        .env(REPARENT, "1")
        .output()
        .expect("spawn the hand-off");
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

    reap(worker.pid);
}

/// Let a spawned ingest finish rather than leaving it writing into a directory
/// the test is about to delete.
#[cfg(unix)]
fn reap(pid: u32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while ps().iter().any(|p| p.pid == pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "the ingest never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
