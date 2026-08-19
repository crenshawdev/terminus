//! `verbatim doctor`: what is wired up, what is not, and the exact command that
//! fixes each thing that is not (INST-06).
//!
//! # It never writes and it never repairs
//!
//! Not one file, not one directory, on any path. That is the requirement and it
//! is also the only way the report can be trusted: a doctor that created what it
//! was asked about would report the state it had just produced. The two places
//! that rule bites are the store, which is opened through
//! [`crate::cmd::read`]'s read-only path rather than `Store::open` (D-12), and
//! the data directory, whose writability is read out of its permissions rather
//! than probed with a file.
//!
//! Repair is a separate command the user types. `verbatim install` is
//! idempotent and is the fix for almost everything here, so printing it costs
//! the user one line and keeps doctor a report.
//!
//! # A check is a state, a finding, and a command
//!
//! Every check carries a machine-stable [`Check::name`], one [`State`], one line
//! of finding, and - where there is something to run - a literally runnable
//! command. Not a description of what to do: "reinstall verbatim" is a sentence,
//! `/home/you/.local/bin/verbatim install` is a fix. The exit code is 1 when any
//! check is [`State::Problem`] and 0 otherwise, so an advisory and a state that
//! is simply what a fresh machine looks like never make `doctor` report failure.
//!
//! # What it will not print
//!
//! `settings.json` is `0600` on this machine and can carry an `env` block of
//! credentials. Doctor reads it for exactly the keys it names - `hooks`, and in
//! the settings checks `cleanupPeriodDays` and `autoCompactEnabled` - and never
//! renders the file, a diff of it, or any other key back at the terminal.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::install::binary;
use super::install::json_file::{Document, Json};
use super::install::targets;
use super::{hook, Failure};

/// The oldest Claude Code verbatim has seen carry exec-form hook `args` (D-16).
///
/// Measured on 2.1.227, 2.1.229 and 2.1.231; the release that introduced `args`
/// is not established, so this is a floor verbatim has checked rather than a
/// proven minimum, and the finding says so.
const CLAUDE_FLOOR: &str = "2.1.227";

/// How long doctor waits for a `--version` it spawned.
///
/// Generous against a local process that prints one line and exits, and short
/// enough that a wedged binary at the stable path costs a diagnostic rather than
/// the terminal.
const VERSION_BUDGET: Duration = Duration::from_millis(2_000);

/// What one check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// As it should be.
    Ok,
    /// True, worth saying, and not a failure: a machine before its first ingest,
    /// or a setting verbatim would choose differently and will not change.
    Note,
    /// Doctor could not tell, and says so rather than guessing. Never counted
    /// against the exit code.
    Unknown,
    /// Broken, and the only state that makes `doctor` exit 1.
    Problem,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Ok => "ok",
            State::Note => "note",
            State::Unknown => "unknown",
            State::Problem => "problem",
        }
    }
}

/// One thing doctor looked at.
pub struct Check {
    /// Stable across runs and across releases: this is the key `--json` reports
    /// the check under, so it is a name rather than a sentence.
    name: String,
    state: State,
    finding: String,
    /// What to run. `None` when the state is fine, and also when the repair is
    /// an edit only the user can make.
    fix: Option<String>,
}

impl Check {
    fn new(name: impl Into<String>, state: State, finding: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            state,
            finding: finding.into(),
            fix: None,
        }
    }

    fn with_fix(mut self, fix: impl Into<String>) -> Check {
        self.fix = Some(fix.into());
        self
    }
}

/// Doctor takes no arguments yet; `--json` arrives with the document (D-24).
pub fn parse(parser: &mut lexopt::Parser) -> Result<(), Failure> {
    if let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        return Err(Failure::Misuse(crate::unexpected(arg)));
    }
    Ok(())
}

pub fn run() -> Result<(), Failure> {
    let mut doctor = Doctor::new();
    doctor.wiring();
    let checks = doctor.checks;

    write_out(&report(&checks))?;

    if checks.iter().any(|check| check.state == State::Problem) {
        // Silent: every problem has already been printed with the command that
        // fixes it, and a trailing "verbatim: ..." line would be a second
        // account of the same thing.
        return Err(Failure::Silent);
    }
    Ok(())
}

/// The checks, and the one thing they all need: this build's own path.
struct Doctor {
    checks: Vec<Check>,
    /// What every fix command starts with.
    ///
    /// This build and not the stable path: the commonest problem doctor reports
    /// is that there is nothing at the stable path, and a fix command naming a
    /// file that does not exist is not a fix. `verbatim` unqualified would
    /// depend on a PATH this install deliberately never edits (D-08).
    exe: String,
}

impl Doctor {
    fn new() -> Doctor {
        Doctor {
            checks: Vec::new(),
            exe: match std::env::current_exe() {
                Ok(path) => path.display().to_string(),
                Err(_) => "verbatim".to_owned(),
            },
        }
    }

    fn install_command(&self) -> String {
        format!("{} install", self.exe)
    }

    fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    // -----------------------------------------------------------------------
    // The wiring: the copy at the stable path, the four hook entries, the mcp
    // registration, and the Claude Code that has to run them.
    // -----------------------------------------------------------------------

    fn wiring(&mut self) {
        let stable = match binary::stable_path() {
            Ok(path) => path,
            Err(failure) => {
                // Without it there is no path to check anything against, and
                // guessing one would be reporting on a file nothing points at.
                let why = detail(failure);
                self.push(Check::new(
                    "binary",
                    State::Unknown,
                    format!("the stable binary path could not be resolved: {why}"),
                ));
                return;
            }
        };
        self.binary(&stable);
        self.hooks(&stable);
        self.mcp_server(&stable);
        self.claude_code();
    }

    /// Is this build at the path every settings entry names (D-07, D-08)?
    fn binary(&mut self, stable: &Path) {
        let check = match binary::occupant(stable) {
            Err(failure) => Check::new(
                "binary",
                State::Problem,
                format!("{} could not be read: {}", stable.display(), detail(failure)),
            )
            .with_fix(self.install_command()),
            Ok(binary::Occupant::Vacant) => Check::new(
                "binary",
                State::Problem,
                format!(
                    "nothing at {}, which is the path every hook entry and the mcp registration \
                     names",
                    stable.display()
                ),
            )
            .with_fix(self.install_command()),
            // The marker and not the version, because version cannot tell them
            // apart: the incumbent measured at this path on 2026-08-13 is a
            // different program whose `--version` prints `verbatim 0.1.0` (D-07).
            Ok(binary::Occupant::Foreign) => Check::new(
                "binary",
                State::Problem,
                format!(
                    "{} does not carry verbatim's marker, so it is not a verbatim build",
                    stable.display()
                ),
            )
            .with_fix(format!("{} && {}", removal(stable), self.install_command())),
            Ok(binary::Occupant::Ours) => self.version_at(stable),
        };
        self.push(check);
    }

    /// The marked copy is a verbatim build; is it *this* one?
    ///
    /// Asked by running it, which is safe here and only here: the marker check
    /// upstream is what establishes that the file is verbatim's, and doctor
    /// never executes a file that failed it.
    fn version_at(&self, stable: &Path) -> Check {
        let mine = env!("CARGO_PKG_VERSION");
        match version_of(stable) {
            None => Check::new(
                "binary",
                State::Unknown,
                format!(
                    "{} carries verbatim's marker and would not report a version",
                    stable.display()
                ),
            ),
            Some(found) if found == mine => Check::new(
                "binary",
                State::Ok,
                format!("{} is this build ({mine})", stable.display()),
            ),
            Some(found) => Check::new(
                "binary",
                State::Problem,
                format!(
                    "{} is verbatim {found} and this build is {mine}",
                    stable.display()
                ),
            )
            .with_fix(self.install_command()),
        }
    }

    /// One check per event (D-01, INST-05).
    ///
    /// Four checks rather than one, because the four events fail
    /// independently: an entry hand-edited for `SessionStart` says nothing
    /// about `PostCompact`, and a finding that did not name the event would
    /// leave the user reading the file to find out which.
    fn hooks(&mut self, stable: &Path) {
        let path = match targets::settings_path() {
            Ok(path) => path,
            Err(failure) => {
                let why = detail(failure);
                self.push(Check::new(
                    "settings_file",
                    State::Unknown,
                    format!("Claude Code's settings file could not be located: {why}"),
                ));
                self.unknown_hooks("the settings file could not be located");
                return;
            }
        };
        let settings = match Document::read(&path) {
            Ok(document) => document,
            Err(failure) => {
                // Unreadable or not JSON. Install refuses to write a settings
                // file it cannot read, so this is why nothing is wired up.
                self.push(Check::new(
                    "settings_file",
                    State::Problem,
                    detail(failure),
                ));
                self.unknown_hooks("the settings file could not be read");
                return;
            }
        };

        if settings.source().trim().is_empty() {
            self.push(Check::new(
                "settings_file",
                State::Note,
                format!("no settings file at {} yet", path.display()),
            ));
        } else {
            self.push(Check::new(
                "settings_file",
                State::Ok,
                format!("{} reads as JSON", path.display()),
            ));
        }

        let stable = stable.display().to_string();
        for event in hook::EVENTS {
            let check = self.hook(settings.value(), event, &stable, &path);
            self.push(check);
        }
    }

    fn unknown_hooks(&mut self, why: &str) {
        for event in hook::EVENTS {
            self.push(Check::new(
                format!("hook_{event}"),
                State::Unknown,
                format!("{event} could not be checked: {why}"),
            ));
        }
    }

    fn hook(&self, settings: &Json, event: &str, stable: &str, path: &Path) -> Check {
        let name = format!("hook_{event}");
        let entries = registered(settings, event);
        let ours: Vec<&&Json> = entries
            .iter()
            .filter(|entry| command_of(entry).as_deref() == Some(stable))
            .collect();
        // Entries that run `verbatim hook <event>` against some other copy of
        // verbatim: a stale install, or a path that moved. They are not ours to
        // edit, and they are exactly what "the hook points somewhere else"
        // looks like on disk.
        let elsewhere: Vec<String> = entries
            .iter()
            .filter(|entry| command_of(entry).as_deref() != Some(stable))
            .filter(|entry| args_of(entry).first().map(String::as_str) == Some("hook"))
            .filter_map(|entry| command_of(entry))
            .collect();

        if ours.len() > 1 {
            return Check::new(
                name,
                State::Problem,
                format!(
                    "{} has {} entries for {event} naming {stable}; one is enough, and the \
                     duplicate has to come out of the file by hand",
                    path.display(),
                    ours.len()
                ),
            );
        }

        let Some(entry) = ours.first() else {
            let finding = match elsewhere.first() {
                Some(other) => format!(
                    "{} runs {other} for {event}, not {stable}",
                    path.display()
                ),
                None => format!("{} has no verbatim entry for {event}", path.display()),
            };
            return Check::new(name, State::Problem, finding).with_fix(self.install_command());
        };

        // Exec form and this event, which is the whole of the shape verbatim
        // writes (D-01). An entry with the right command and no `args` runs
        // through a shell on Windows and starts no ingest anywhere.
        let exec_form = entry.get("type").and_then(Json::as_str).as_deref() == Some("command");
        let args = args_of(entry);
        let addressed = args.len() == 2 && args[0] == "hook" && args[1] == event;
        if !exec_form || !addressed {
            return Check::new(
                name,
                State::Problem,
                format!(
                    "{}'s {event} entry names {stable} and is not in the shape verbatim writes \
                     (type \"command\", args [\"hook\", \"{event}\"])",
                    path.display()
                ),
            )
            .with_fix(self.install_command());
        }

        match elsewhere.first() {
            Some(other) => Check::new(
                name,
                State::Note,
                format!(
                    "{} runs {stable} for {event}, and also {other}, which is not this install's",
                    path.display()
                ),
            ),
            None => Check::new(
                name,
                State::Ok,
                format!("{} runs {stable} for {event} in exec form", path.display()),
            ),
        }
    }

    /// The registration recall is reached through, in `.claude.json` (D-05).
    fn mcp_server(&mut self, stable: &Path) {
        let path = match targets::claude_json_path() {
            Ok(path) => path,
            Err(failure) => {
                let why = detail(failure);
                self.push(Check::new(
                    "mcp_server",
                    State::Unknown,
                    format!("Claude Code's .claude.json could not be located: {why}"),
                ));
                return;
            }
        };
        let document = match Document::read(&path) {
            Ok(document) => document,
            Err(failure) => {
                self.push(Check::new("mcp_server", State::Problem, detail(failure)));
                return;
            }
        };
        let stable = stable.display().to_string();
        let server = document
            .value()
            .get("mcpServers")
            .and_then(|servers| servers.get(targets::MCP_SERVER_KEY));

        let check = match server {
            None => Check::new(
                "mcp_server",
                State::Problem,
                format!(
                    "{} registers no '{}' server under .mcpServers, so recall is unreachable from \
                     inside a session",
                    path.display(),
                    targets::MCP_SERVER_KEY
                ),
            )
            .with_fix(self.install_command()),
            Some(server) => match server.get("command").and_then(Json::as_str) {
                Some(command) if command == stable => {
                    let stdio =
                        server.get("type").and_then(Json::as_str).as_deref() == Some("stdio");
                    let args = server.get("args").map(Json::items).unwrap_or(&[]);
                    let addressed =
                        args.len() == 1 && args[0].as_str().as_deref() == Some("mcp");
                    if stdio && addressed {
                        Check::new(
                            "mcp_server",
                            State::Ok,
                            format!("{} runs {stable} as the '{}' server", path.display(), targets::MCP_SERVER_KEY),
                        )
                    } else {
                        Check::new(
                            "mcp_server",
                            State::Problem,
                            format!(
                                "{}'s '{}' server names {stable} and is not in the shape verbatim \
                                 writes (type \"stdio\", args [\"mcp\"])",
                                path.display(),
                                targets::MCP_SERVER_KEY
                            ),
                        )
                        .with_fix(self.install_command())
                    }
                }
                Some(other) => Check::new(
                    "mcp_server",
                    State::Problem,
                    format!(
                        "{}'s '{}' server runs {other}, not {stable}",
                        path.display(),
                        targets::MCP_SERVER_KEY
                    ),
                )
                .with_fix(self.install_command()),
                None => Check::new(
                    "mcp_server",
                    State::Problem,
                    format!(
                        "{}'s '{}' server has no command",
                        path.display(),
                        targets::MCP_SERVER_KEY
                    ),
                )
                .with_fix(self.install_command()),
            },
        };
        self.push(check);
    }

    /// Is the Claude Code on this machine one verbatim has seen run exec-form
    /// hooks (D-16)?
    fn claude_code(&mut self) {
        let name = if cfg!(windows) { "claude.exe" } else { "claude" };
        let Some(program) = on_path(name) else {
            self.push(Check::new(
                "claude_code",
                State::Unknown,
                format!("no {name} on PATH, so its version could not be read"),
            ));
            return;
        };
        let Some(reported) = version_of(&program) else {
            self.push(Check::new(
                "claude_code",
                State::Unknown,
                format!("{} would not report a version", program.display()),
            ));
            return;
        };
        // `claude --version` prints `2.1.235 (Claude Code)`, so the version is
        // the leading token and the rest is a product name.
        let Some(version) = reported.split_whitespace().next() else {
            self.push(Check::new(
                "claude_code",
                State::Unknown,
                format!("{} reported {reported:?}, which is not a version", program.display()),
            ));
            return;
        };

        let floor_note = format!(
            "{CLAUDE_FLOOR} is the oldest version verbatim has checked for exec-form hook args, \
             not a proven minimum"
        );
        if precedes(version, CLAUDE_FLOOR) {
            self.push(
                Check::new(
                    "claude_code",
                    State::Problem,
                    format!(
                        "claude {version} predates {CLAUDE_FLOOR}, so exec-form hook entries may \
                         not run at all; {floor_note}"
                    ),
                )
                .with_fix(format!("{} update", program.display())),
            );
        } else {
            self.push(Check::new(
                "claude_code",
                State::Ok,
                format!("claude {version} at {}; {floor_note}", program.display()),
            ));
        }
    }
}

/// Every hook entry Claude Code would run for `event`.
///
/// Flattened out of the matcher groups, because a settings file writes them as
/// groups and runs them as a list. A `hooks` object that is not an object, or a
/// group that is not one, reads as no entries rather than as an error: doctor
/// is reporting on the file, and a malformed one is reported by the entries it
/// does not have.
fn registered<'a>(settings: &'a Json, event: &str) -> Vec<&'a Json> {
    settings
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .map(Json::items)
        .unwrap_or(&[])
        .iter()
        .filter_map(|group| group.get("hooks"))
        .flat_map(Json::items)
        .collect()
}

fn command_of(entry: &Json) -> Option<String> {
    entry.get("command").and_then(Json::as_str)
}

fn args_of(entry: &Json) -> Vec<String> {
    entry
        .get("args")
        .map(Json::items)
        .unwrap_or(&[])
        .iter()
        .filter_map(Json::as_str)
        .collect()
}

/// `program --version`, as the first line it prints.
///
/// Absolute program, no shell, stdin and stderr at null, and a deadline: this
/// is `verbatim_core::project`'s `git` call with the same reasoning behind every
/// part of it. Every failure is the same answer - `None`, meaning "doctor could
/// not tell" - because a binary that is not there, one that is not executable
/// and one that hangs all leave the caller with the same thing to say.
fn version_of(program: &Path) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + VERSION_BUDGET;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return None,
        }
    }

    // Read after the exit, which is safe only because a `--version` prints one
    // short line and cannot fill the pipe buffer.
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// A program's absolute path, found by scanning `PATH` ourselves.
///
/// The same rule the ingest path uses for `git`: an absolute program and no
/// shell, because a bare program name resolved by the OS is the Windows
/// PATH-probing failure class this design retires.
fn on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Does `version` sort before `floor`, comparing dotted numbers as numbers?
///
/// A string comparison would put `2.1.9` after `2.1.227`, which is the whole
/// reason this is not one. A component that is not a number compares as 0, so a
/// pre-release suffix reads as the release it precedes rather than as an error.
fn precedes(version: &str, floor: &str) -> bool {
    let parts = |text: &str| -> Vec<u64> {
        text.split('.')
            .map(|part| {
                let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap_or(0)
            })
            .collect()
    };
    let found = parts(version);
    let want = parts(floor);
    for at in 0..found.len().max(want.len()) {
        let a = found.get(at).copied().unwrap_or(0);
        let b = want.get(at).copied().unwrap_or(0);
        if a != b {
            return a < b;
        }
    }
    false
}

/// The command that removes a file, on the shell the user is at.
fn removal(path: &Path) -> String {
    if cfg!(windows) {
        format!("del \"{}\"", path.display())
    } else {
        format!("rm -f '{}'", path.display())
    }
}

/// A failure's own words, for a check that reports it rather than raising it.
fn detail(failure: Failure) -> String {
    match failure {
        Failure::Operational(message) | Failure::Misuse(message) => message,
        Failure::Silent => "no detail".to_owned(),
    }
}

/// The whole report, as one string.
///
/// Built and then written once rather than printed line by line, so a closed
/// stdout is one error to answer instead of a panic in the middle of a page.
fn report(checks: &[Check]) -> String {
    let width = checks.iter().map(|check| check.name.len()).max().unwrap_or(0);
    let mut out = String::from("verbatim doctor\n\n");
    for check in checks {
        out.push_str(&format!(
            "  {:<7}  {:<width$}  {}\n",
            check.state.label(),
            check.name,
            check.finding
        ));
        if let Some(fix) = &check.fix {
            // In the finding's column, with `fix` where the state was: the
            // command is the answer to the line above it, and a user scanning
            // the left edge sees which lines have one.
            out.push_str(&format!("  {:<7}  {:<width$}  {fix}\n", "fix", ""));
        }
    }

    let problems = checks
        .iter()
        .filter(|check| check.state == State::Problem)
        .count();
    out.push('\n');
    if problems == 0 {
        out.push_str("no problems. doctor changed nothing and never does.\n");
    } else {
        out.push_str(&format!(
            "{problems} problem(s). doctor changed nothing and never does: run the commands \
             above.\n"
        ));
    }
    out
}

/// One write, and a reader that went away is not a failure of the machine.
///
/// `verbatim doctor | head` closes the pipe after the first page. The verdict
/// the checks earned still stands - a problem found and not printed is still a
/// problem - so the pipe error is dropped and the exit code is left to `run`.
fn write_out(text: &str) -> Result<(), Failure> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(Failure::Operational(format!(
            "the report could not be written: {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dotted versions compare as numbers, which is the only reason this
    /// function exists: `2.1.9 < 2.1.227` is false as a string comparison and
    /// would report every 2.1.9x Claude Code as new enough.
    #[test]
    fn a_version_below_the_floor_is_below_it_numerically() {
        assert!(precedes("2.1.9", "2.1.227"));
        assert!(precedes("2.0.999", "2.1.227"));
        assert!(precedes("1.9.9", "2.1.227"));
        assert!(!precedes("2.1.227", "2.1.227"));
        assert!(!precedes("2.1.231", "2.1.227"));
        assert!(!precedes("2.2.0", "2.1.227"));
        assert!(!precedes("3.0", "2.1.227"));
    }

    /// A suffix reads as the release it precedes rather than as an error.
    #[test]
    fn a_prerelease_suffix_does_not_break_the_comparison() {
        assert!(!precedes("2.1.231-beta.2", "2.1.227"));
        assert!(precedes("2.1.100-rc1", "2.1.227"));
    }
}
