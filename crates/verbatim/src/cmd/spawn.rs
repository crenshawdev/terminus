//! Starting the ingest a hook exists to start, and losing it on purpose.
//!
//! A hook has one job here: get `verbatim ingest` running and return to the
//! harness. Everything in this module is about the second half of that - the
//! process it started must outlive the hook, including the way Claude Code
//! kills a hook it has given up on.
//!
//! # Two kills, not one (D-03)
//!
//! Claude Code 2.1.231 terminates a timed-out hook twice over. It calls
//! `process.kill(-pid, SIGKILL)` on the hook's process **group**, and then it
//! walks a descendant set it builds by parsing `pid ppid` pairs out of `ps`
//! output and breadth-first searching from the hook's pid, killing every pid it
//! finds. A new process group answers the first mechanism and not the second: a
//! child still parented to the hook is in that walk whatever group it sits in.
//!
//! So the working process leaves both sets. [`detached`] spawns an
//! intermediate in a new process group with [`HANDOFF`] in front of the
//! arguments; the intermediate, at startup and before any argument parsing,
//! re-spawns itself with the arguments *behind* the marker through this same
//! module and exits. Its child is reparented to init within microseconds, so it
//! is in no process group the hook belongs to and under no branch of a
//! descendant walk rooted at the hook. That is the classic double fork, spelled
//! in [`std::process`] because that is all this crate is allowed to spell it
//! in.
//!
//! # `std` alone (D-04)
//!
//! No `libc`, no `nix`, no `windows-sys`.
//! [`std::os::unix::process::CommandExt::process_group`] and
//! [`std::os::windows::process::CommandExt::creation_flags`] are the whole
//! mechanism, exactly as `ingest::lock` uses [`std::fs::File::try_lock`] rather
//! than naming `flock` and `LockFileEx` itself. Every dependency in the root
//! `Cargo.toml` carries a comment justifying its cost against a measured
//! 0.408 ms startup floor, and this path is the floor's whole reason to exist.

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Command, Stdio};

/// Marks the intermediate process: the first argument of the child [`detached`]
/// spawns, and absent from the arguments that child hands on.
///
/// It is an argument and not an environment variable, and that is the whole
/// point. An environment variable is ambient - inherited by every descendant,
/// settable from a shell profile or a `.envrc`, and impossible to distinguish
/// from one this module set - so `verbatim search error` under a stray
/// `VERBATIM_REPARENT=1` would have re-spawned itself and exited 0 with no
/// output, turning an unknown hook event into a success instead of a misuse.
/// An argument is inherited by nothing. What this marker therefore guarantees
/// is narrow and real: no environment a caller brings, deliberately or by
/// accident, can put an ordinary invocation on this path.
///
/// What it does not claim is that the marker is a secret. Typing it re-spawns
/// the rest of the command line detached, which grants a caller nothing they
/// could not do by running the same command themselves, and costs them its
/// output. It is deliberately absent from `USAGE` for that reason: a mechanism,
/// not an interface. It is `--` prefixed so that if this hand-off is ever
/// removed while something still spells it, the parser rejects it as an
/// unexpected argument (exit 2) rather than accepting it as a positional.
pub const HANDOFF: &str = "--reparent";

/// Windows: no console, and no parent to be killed with.
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start this binary again with `args`, detached, and return without waiting.
///
/// The environment is inherited unchanged, which is how `VERBATIM_DATA_DIR`,
/// `VERBATIM_CONFIG_DIR` and `CLAUDE_CONFIG_DIR` reach the work.
pub fn detached<S: AsRef<OsStr>>(args: &[S]) -> io::Result<()> {
    let mut marked: Vec<OsString> = Vec::with_capacity(args.len() + 1);
    marked.push(OsString::from(HANDOFF));
    marked.extend(args.iter().map(|arg| arg.as_ref().to_os_string()));
    spawn(&marked)
}

/// The reparenting hand-off, called by `main` before it parses anything.
///
/// Returns `true` when this process is the intermediate and has done its whole
/// job - the caller must exit 0 immediately and touch nothing else, because the
/// arguments it was given belong to the process it just started.
///
/// The test is positional: [`HANDOFF`] as the *first* argument and nothing
/// else. An invocation that merely mentions it later - `verbatim search
/// --reparent` - is an ordinary command line and reaches its subcommand's own
/// parser, which rejects it. And the marker is dropped here rather than passed
/// on, so the working process runs the arguments it was given and cannot hand
/// them on again.
pub fn handed_off() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_none_or(|first| first != HANDOFF) {
        return false;
    }
    let work: Vec<OsString> = args.collect();
    // Nowhere to report to: all three of this process's stdio are already
    // `Stdio::null()`. The hook has returned to the harness either way, which
    // is the property that matters more than this run of the ingest.
    let _ = spawn(&work);
    true
}

fn spawn<S: AsRef<OsStr>>(args: &[S]) -> io::Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(args)
        // A hook that leaves the write end of its stdout pipe open in a child
        // hangs any caller reading that pipe to EOF for as long as the ingest
        // runs, which on a backfill is minutes.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 0 means "a new group led by the child", which is what takes it out of
        // the group `kill(-pid)` names.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }

    // Spawned and dropped: waiting is the one thing a detached spawn must never
    // do. The intermediate exits within microseconds and is reaped by whatever
    // outlives the hook.
    command.spawn().map(drop)
}
