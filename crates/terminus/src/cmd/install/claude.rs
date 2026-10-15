//! Running `claude` to place or remove the MCP entry.
//!
//! Claude Code is the only writer of `.claude.json` (phase 3 D-03), so install
//! and uninstall ask it to change that file rather than editing it themselves.
//! The call follows doctor's `version_of`: `claude` found by scanning `PATH`
//! here and run by absolute path, no shell, stdin at null, and a deadline. Only
//! the exit status and the first line of stderr are read. Stdout is display
//! text with no stable format, so it goes to null.
//!
//! Every process gets `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` (D-04).
//! Without it `claude mcp add-json` and `claude mcp remove` connect to
//! `api.anthropic.com`, and wiring terminus up must open no connection. The
//! rest of the environment is inherited, so a `CLAUDE_CONFIG_DIR` reaches
//! `claude` and it writes the same file install read.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long one `claude` command may run (D-07). Measured runs took 197 to
/// 252 ms; Baley allows the same 30 s.
pub const DEADLINE: Duration = Duration::from_secs(30);

/// Set on every `claude` process terminus starts (D-04).
pub const NONESSENTIAL_TRAFFIC: &str = "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC";

/// How much stderr is kept. Only its first line is ever reported.
const STDERR_LIMIT: u64 = 64 * 1024;

/// What one `claude` command came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It exited 0.
    Done,
    /// No `claude` on `PATH`.
    NotOnPath,
    /// It ran past the deadline and was killed.
    TimedOut(Duration),
    /// It was found and could not be started.
    NotStarted(String),
    /// It exited non-zero: the first line of stderr, or the exit status when
    /// stderr was empty.
    Failed(String),
}

impl Outcome {
    /// Why it did not complete, in words for the summary. `None` for
    /// [`Outcome::Done`].
    pub fn cause(&self) -> Option<String> {
        match self {
            Outcome::Done => None,
            Outcome::NotOnPath => Some("claude is not on PATH".to_owned()),
            Outcome::TimedOut(after) => Some(format!(
                "claude did not finish within {}s and was stopped",
                after.as_secs()
            )),
            Outcome::NotStarted(why) => Some(format!("claude could not be started: {why}")),
            Outcome::Failed(why) => Some(why.clone()),
        }
    }
}

/// Run `claude` with `args`, found on this process's `PATH`.
pub fn run(args: &[&str]) -> Outcome {
    run_with(std::env::var_os("PATH").as_deref(), args, DEADLINE)
}

/// [`run`], with the `PATH` to search and the deadline given.
fn run_with(path: Option<&OsStr>, args: &[&str], deadline: Duration) -> Outcome {
    match on_path(path, "claude") {
        Some(program) => run_at(&program, args, deadline),
        None => Outcome::NotOnPath,
    }
}

/// The first `name` in `path`'s directories, as doctor's `on_path` finds it.
fn on_path(path: Option<&OsStr>, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn run_at(program: &Path, args: &[&str], deadline: Duration) -> Outcome {
    let mut child = match Command::new(program)
        .args(args)
        .env(NONESSENTIAL_TRAFFIC, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return Outcome::NotStarted(e.to_string()),
    };

    // Drained on its own thread while the child runs, so one that writes more
    // than a pipe holds is not left blocked until the deadline.
    let (sender, stderr) = mpsc::channel();
    if let Some(pipe) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut pipe = pipe;
            let mut kept = Vec::new();
            let _ = (&mut pipe).take(STDERR_LIMIT).read_to_end(&mut kept);
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            let _ = sender.send(kept);
        });
    }

    let until = Instant::now() + deadline;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= until => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::TimedOut(deadline);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::Failed(format!("claude could not be waited on: {e}"));
            }
        }
    };
    if status.success() {
        return Outcome::Done;
    }

    // A process `claude` left behind can hold stderr open after it exits, so
    // the wait for the text is bounded by what is left of the deadline.
    let left = until.saturating_duration_since(Instant::now());
    let text = stderr.recv_timeout(left).unwrap_or_default();
    let text = String::from_utf8_lossy(&text);
    match text.lines().next().map(str::trim) {
        Some(first) if !first.is_empty() => Outcome::Failed(first.to_owned()),
        _ => Outcome::Failed(exit_description(status)),
    }
}

fn exit_description(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("claude exited with status {code}"),
        (None, Some(signal)) => format!("claude was killed by signal {signal}"),
        (None, None) => "claude ended without an exit status".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A directory holding a `claude` that runs `body` under `/bin/sh`.
    fn fake(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        dir
    }

    /// [`run_with`] on the fake in `dir`, run again while exec says the script
    /// is busy. Another test's fork can hold the script's write descriptor
    /// until that child execs, and an exec in that window fails with ETXTBSY.
    fn run_fake(dir: &tempfile::TempDir, deadline: Duration) -> Outcome {
        for _ in 0..100 {
            match run_with(Some(dir.path().as_os_str()), &["mcp"], deadline) {
                Outcome::NotStarted(why) if why.contains("os error 26") => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                outcome => return outcome,
            }
        }
        panic!("the fake claude stayed busy");
    }

    #[test]
    fn a_claude_that_never_ends_is_stopped_at_the_deadline() {
        let dir = fake("while :; do :; done");
        let started = Instant::now();
        let outcome = run_fake(&dir, Duration::from_millis(200));
        assert_eq!(outcome, Outcome::TimedOut(Duration::from_millis(200)));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_failure_is_reported_by_its_first_stderr_line() {
        let dir = fake("printf 'first\\nsecond\\n' >&2\nexit 3");
        let outcome = run_fake(&dir, DEADLINE);
        assert_eq!(outcome, Outcome::Failed("first".to_owned()));
    }

    #[test]
    fn a_failure_with_nothing_on_stderr_is_reported_by_its_status() {
        let dir = fake("exit 4");
        let outcome = run_fake(&dir, DEADLINE);
        assert_eq!(
            outcome,
            Outcome::Failed("claude exited with status 4".to_owned())
        );
    }

    #[test]
    fn a_path_without_claude_is_not_on_path() {
        let empty = tempfile::tempdir().unwrap();
        let outcome = run_with(Some(empty.path().as_os_str()), &["mcp"], DEADLINE);
        assert_eq!(outcome, Outcome::NotOnPath);
        assert_eq!(run_with(None, &["mcp"], DEADLINE), Outcome::NotOnPath);
    }

    #[test]
    fn every_claude_gets_the_traffic_variable() {
        let dir = fake(&format!(
            "[ \"${NONESSENTIAL_TRAFFIC}\" = 1 ] || {{ echo unset >&2; exit 1; }}"
        ));
        let outcome = run_fake(&dir, DEADLINE);
        assert_eq!(outcome, Outcome::Done);
    }
}
