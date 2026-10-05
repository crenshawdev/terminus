//! `terminus doctor`: what is wired up, what is not, and the exact command that
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
//! Repair is a separate command the user types. `terminus install` is
//! idempotent and is the fix for almost everything here, so printing it costs
//! the user one line and keeps doctor a report.
//!
//! # A check is a state, a finding, and a command
//!
//! Every check carries a machine-stable [`Check::name`], one [`State`], one line
//! of finding, and - where there is something to run - a literally runnable
//! command. Not a description of what to do: "reinstall terminus" is a sentence,
//! `/home/you/.local/bin/terminus install` is a fix. The exit code is 1 when any
//! check is [`State::Problem`] and 0 otherwise, so an advisory and a state that
//! is simply what a fresh machine looks like never make `doctor` report failure.
//!
//! # What it will not print
//!
//! `settings.json` is `0600` on this machine and can carry an `env` block of
//! credentials. Doctor reads it for exactly the keys it names - `hooks`, and in
//! the settings checks `cleanupPeriodDays` and `autoCompactEnabled` - and never
//! renders the file, a diff of it, or any other key back at the terminal.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use terminus_core::config::{config_dir, visible, CONFIG_FILE_NAME};
use terminus_core::credentials::{self, Permissions};
use terminus_core::Config;

use super::install::binary;
use super::install::json_file::{Document, Json};
use super::install::targets;
use super::json::Document as Envelope;
use super::read::{self, Opened};
use super::{hook, Failure};

/// The oldest Claude Code terminus has seen carry exec-form hook `args` (D-16).
///
/// Measured on 2.1.227, 2.1.229 and 2.1.231; the release that introduced `args`
/// is not established, so this is a floor terminus has checked rather than a
/// proven minimum, and the finding says so.
const CLAUDE_FLOOR: &str = "2.1.227";

/// How long doctor waits for a `--version` it spawned.
///
/// Generous against a local process that prints one line and exits, and short
/// enough that a wedged binary at the stable path costs a diagnostic rather than
/// the terminal.
const VERSION_BUDGET: Duration = Duration::from_millis(2_000);

/// How much of a failed run's error doctor puts on one line.
///
/// The whole of it is `terminus status`'s to print; this is the first line, cut
/// so a pass that failed on a path 4 KB long is still a report.
const MAX_ERROR_LINE: usize = 200;

/// The environment variable that turns Claude Code's auto-compaction off, and
/// the one source that outranks every settings file.
const AUTO_COMPACT_ENV: &str = "DISABLE_AUTO_COMPACT";

/// Below this many days of Claude Code's own retention, doctor says what that
/// costs. `cmd::install` holds the same three numbers for the advisory it gives
/// at install time; they are the same policy said in two places, and a change to
/// one belongs in both.
const LOW_CLEANUP_DAYS: i64 = 90;

/// What Claude Code deletes after when `cleanupPeriodDays` is not set at all.
const CLAUDE_DEFAULT_CLEANUP_DAYS: i64 = 30;

/// What doctor's printed edit sets it to: ten years, which is "keep them".
const KEEP_CLEANUP_DAYS: i64 = 3650;

/// What one check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// As it should be.
    Ok,
    /// True, worth saying, and not a failure: a machine before its first ingest,
    /// or a setting terminus would choose differently and will not change.
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

pub fn run(json: bool) -> Result<(), Failure> {
    let mut doctor = Doctor::new();
    doctor.wiring();
    doctor.archive();
    doctor.credentials();
    doctor.config_mode();
    doctor.mode_checks();
    doctor.claude_settings();
    let checks = doctor.checks;

    let problems: Vec<&str> = checks
        .iter()
        .filter(|check| check.state == State::Problem)
        .map(|check| check.name.as_str())
        .collect();

    if json {
        emit(&checks, &problems)?;
    } else {
        write_out(&report(&checks))?;
    }

    if problems.is_empty() {
        return Ok(());
    }
    // Silent: every problem is already in the report, with the command that
    // fixes it, and a trailing "terminus: ..." line would be a second account
    // of the same thing.
    Err(Failure::Silent)
}

/// One `{command, ok, reason, data}` document, `data` keyed by check name
/// (D-24, RCL-06).
///
/// `ok` is the exit code's answer, as the envelope requires: a caller that
/// parses the document and a caller that checks the code never disagree.
fn emit(checks: &[Check], problems: &[&str]) -> Result<(), Failure> {
    let mut document = Envelope::new("doctor");
    for check in checks {
        document = document.field(
            &check.name,
            serde_json::json!({
                "state": check.state.label(),
                "finding": check.finding,
                // Null rather than absent, and null wherever there is nothing
                // to run - including a problem whose repair is an edit only the
                // user can make.
                "fix": check.fix,
            }),
        );
    }
    if !problems.is_empty() {
        document = document.failed().because(format!(
            "{} check(s) reported a problem: {}",
            problems.len(),
            problems.join(", ")
        ));
    }
    match document.try_emit() {
        Ok(()) => Ok(()),
        // A reader that went away is not a finding about this machine, and the
        // verdict the checks earned still stands.
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(Failure::Operational(format!(
            "the report could not be written: {e}"
        ))),
    }
}

/// The checks, and the one thing they all need: this build's own path.
struct Doctor {
    checks: Vec<Check>,
    /// What every fix command starts with.
    ///
    /// This build and not the stable path: the commonest problem doctor reports
    /// is that there is nothing at the stable path, and a fix command naming a
    /// file that does not exist is not a fix. `terminus` unqualified would
    /// depend on a PATH this install deliberately never edits (D-08).
    exe: String,
}

impl Doctor {
    fn new() -> Doctor {
        Doctor {
            checks: Vec::new(),
            exe: match std::env::current_exe() {
                Ok(path) => path.display().to_string(),
                Err(_) => "terminus".to_owned(),
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
                // Every check still reports, as `unknown`: one run's document
                // carries the same keys as any other's, so a caller reads a
                // state rather than a missing key (D-24).
                let why = detail(failure);
                self.push(Check::new(
                    "binary",
                    State::Unknown,
                    format!("the stable binary path could not be resolved: {why}"),
                ));
                let why = format!("the stable binary path could not be resolved: {why}");
                self.unknown("settings_file", &why);
                self.unknown_hooks("the stable binary path could not be resolved");
                self.unknown("mcp_server", &why);
                self.claude_code();
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
                format!(
                    "{} could not be read: {}",
                    stable.display(),
                    detail(failure)
                ),
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
            // different program whose `--version` prints `terminus 0.1.0` (D-07).
            Ok(binary::Occupant::Foreign) => Check::new(
                "binary",
                State::Problem,
                format!(
                    "{} does not carry terminus's marker, so it is not a terminus build",
                    stable.display()
                ),
            )
            .with_fix(format!("{} && {}", removal(stable), self.install_command())),
            Ok(binary::Occupant::Ours) => self.version_at(stable),
        };
        self.push(check);
    }

    /// The marked copy is a terminus build; is it *this* one?
    ///
    /// Asked by running it, which is safe here and only here: the marker check
    /// upstream is what establishes that the file is terminus's, and doctor
    /// never executes a file that failed it.
    fn version_at(&self, stable: &Path) -> Check {
        let mine = env!("CARGO_PKG_VERSION");
        match version_of(stable) {
            None => Check::new(
                "binary",
                State::Unknown,
                format!(
                    "{} carries terminus's marker and would not report a version",
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
                    "{} is terminus {found} and this build is {mine}",
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
                self.push(Check::new("settings_file", State::Problem, detail(failure)));
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
        // Entries that run `terminus hook <event>` against some other copy of
        // terminus: a stale install, or a path that moved. They are not ours to
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
                Some(other) => format!("{} runs {other} for {event}, not {stable}", path.display()),
                None => format!("{} has no terminus entry for {event}", path.display()),
            };
            return Check::new(name, State::Problem, finding).with_fix(self.install_command());
        };

        // Exec form and this event, which is the whole of the shape terminus
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
                    "{}'s {event} entry names {stable} and is not in the shape terminus writes \
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
                    let addressed = args.len() == 1 && args[0].as_str().as_deref() == Some("mcp");
                    if stdio && addressed {
                        Check::new(
                            "mcp_server",
                            State::Ok,
                            format!(
                                "{} runs {stable} as the '{}' server",
                                path.display(),
                                targets::MCP_SERVER_KEY
                            ),
                        )
                    } else {
                        Check::new(
                            "mcp_server",
                            State::Problem,
                            format!(
                                "{}'s '{}' server names {stable} and is not in the shape terminus \
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

    /// Is the Claude Code on this machine one terminus has seen run exec-form
    /// hooks (D-16)?
    fn claude_code(&mut self) {
        let name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
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
                format!(
                    "{} reported {reported:?}, which is not a version",
                    program.display()
                ),
            ));
            return;
        };

        let floor_note = format!(
            "{CLAUDE_FLOOR} is the oldest version terminus has checked for exec-form hook args, \
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

/// The last pass an ingest left an account of.
///
/// There is no log file, by design, so the `runs` table is the only place a
/// detached ingest's failure is written down. That is why doctor reports it at
/// all: the user never saw the process, and this is the record it left.
struct Run {
    started_at: String,
    duration_ms: i64,
    files_seen: i64,
    files_committed: i64,
    files_failed: i64,
    turns_added: i64,
    error: Option<String>,
}

impl Doctor {
    // -----------------------------------------------------------------------
    // Terminus's own state: the roots it walks, the directory it writes into,
    // the store it keeps, and the last pass that ran.
    //
    // All of it through the read path (D-12). `Store::open` would
    // `create_dir_all`, initialize a database, bring columns forward and set
    // `journal_mode=wal` - so a doctor built on it would report a store its own
    // check had just created, on a machine that has never ingested anything.
    // -----------------------------------------------------------------------

    fn archive(&mut self) {
        let config = match Config::load() {
            Ok(config) => {
                self.roots(&config);
                self.provider_local(&config);
                Some(config)
            }
            Err(error) => {
                self.push(Check::new(
                    "config_roots",
                    State::Problem,
                    format!("terminus's own config could not be read: {error}"),
                ));
                self.unknown("provider_local", "terminus's config could not be read");
                None
            }
        };

        let data_dir = match crate::cmd::data_dir() {
            Ok(dir) => dir,
            Err(failure) => {
                let why = detail(failure);
                self.push(Check::new(
                    "data_directory",
                    State::Problem,
                    format!("terminus's data directory could not be resolved: {why}"),
                ));
                self.unknown("store", "the data directory could not be resolved");
                self.unknown("last_run", "the data directory could not be resolved");
                return;
            }
        };
        self.data_directory(&data_dir);

        match config {
            Some(config) => self.store(&data_dir, config),
            None => {
                // Opening the store without the config would count sessions an
                // exclusion says a read may not see (ING-08), which is a wrong
                // number rather than a missing one.
                self.unknown("store", "terminus's config could not be read");
                self.unknown("last_run", "terminus's config could not be read");
            }
        }
    }

    /// The shared credentials file: where it was looked for, whether it is
    /// there, and what its permissions say (PRIV-02, D-15).
    ///
    /// An absent file is the ORDINARY state and not a problem. It does not
    /// exist on this machine, most machines will never have one, and a provider
    /// key can also come from `[provider] api_key` or from the environment - so
    /// reporting its absence as broken would tell every user their machine is
    /// broken.
    ///
    /// Reads a path and a mode. It does not open the file, so no part of a
    /// credential can reach the report or the `--json` document, and it creates
    /// nothing - doctor never writes, and a check that created the file it was
    /// asked about would be reporting on its own work.
    fn credentials(&mut self) {
        let Some(path) = credentials::path() else {
            self.push(Check::new(
                "credentials",
                State::Unknown,
                "no config directory resolved, so there is nowhere to look for a shared \
                 credentials file",
            ));
            return;
        };
        let where_it_is = path.display().to_string();
        let check = match credentials::permissions(&path) {
            Permissions::Absent => Check::new(
                "credentials",
                State::Note,
                format!(
                    "nothing at {where_it_is}; a provider key can also come from \
                     [provider] api_key or from the environment"
                ),
            ),
            Permissions::Owner { mode } => Check::new(
                "credentials",
                State::Ok,
                format!("{where_it_is} is mode {mode:03o}"),
            ),
            // Unix only by construction: `credentials::permissions` never
            // answers this on a build without mode bits, which is why the fix
            // can be a bare `chmod` with no platform arm.
            Permissions::TooOpen { mode } => Check::new(
                "credentials",
                State::Problem,
                format!(
                    "{where_it_is} is mode {mode:03o}, so it is readable beyond its owner \
                     and terminus refuses to load a credential from it"
                ),
            )
            .with_fix(format!("chmod 600 '{where_it_is}'")),
            // D-15's deferral, said in words rather than left as an `ok` this
            // build did not earn.
            Permissions::Unchecked => Check::new(
                "credentials",
                State::Unknown,
                format!(
                    "{where_it_is} is there; this build does not check Windows ACLs, so \
                     whether anyone else can read it is unverified"
                ),
            ),
            Permissions::Unreadable { detail } => Check::new(
                "credentials",
                State::Problem,
                format!("{where_it_is} could not be read: {detail}"),
            ),
        };
        self.push(check);
    }

    /// A provider declared local whose address is not one (D-13, D-15).
    ///
    /// **A note, never a problem, and nothing here changes where a byte goes.**
    /// `provider.local` describes the DESTINATION and is declared rather than
    /// inferred: the egress filter reads that key and not the host, exactly as
    /// `Config::provider_local` documents, because deciding from the address
    /// would mean resolving a name and a resolution is a connection PRIV-03
    /// bars. So this is a mistake worth naming - `local = true` against a
    /// remote address sends session text there unfiltered - and it is not
    /// doctor's to overrule, which is why `terminus doctor` still exits 0.
    ///
    /// The host is named and the configured URL is not: a `base_url` may carry
    /// userinfo, and the one thing this report may not do is print a
    /// credential.
    fn provider_local(&mut self, config: &Config) {
        let check = match (config.provider_local(), config.provider_base_url()) {
            (false, _) => Check::new(
                "provider_local",
                State::Ok,
                "the provider is not declared local, so every request is filtered at egress",
            ),
            (true, None) => Check::new(
                "provider_local",
                State::Unknown,
                "the provider is declared local and no base_url is configured, so there is \
                 no host to check it against",
            ),
            (true, Some(base_url)) => {
                let host = host_of(base_url);
                if is_loopback(host) {
                    Check::new(
                        "provider_local",
                        State::Ok,
                        format!(
                            "the provider is declared local and its base_url is on {host}, \
                             so nothing leaves this machine"
                        ),
                    )
                } else {
                    Check::new(
                        "provider_local",
                        State::Note,
                        format!(
                            "the provider is declared local but its base_url names {host}, \
                             which is not a loopback address; the declaration decides what \
                             is filtered and the address does not, so session text is sent \
                             there unfiltered"
                        ),
                    )
                }
            }
        };
        self.push(check);
    }

    /// `terminus.toml`'s own mode, on the same test and through the same
    /// function the refusal uses (PRIV-02, D-14).
    ///
    /// A config file carrying `provider.api_key` is a credentials file whatever
    /// else is in it, and `credentials::resolve` refuses to read a key out of
    /// one anybody else can read. So this reports what that refusal would say,
    /// through `credentials::permissions` rather than a second mode test of its
    /// own: a report that disagreed with the refusal would be worse than no
    /// report.
    ///
    /// **Whether there is a key decides the state, not the mode alone.** A wide
    /// `terminus.toml` holding no key is a file of preferences and nothing is
    /// refused over it, so calling it a problem would tell most users to fix
    /// something that is not broken. A file that did not parse is the one case
    /// where the answer is unknowable, and it says so and still prints the
    /// chmod.
    ///
    /// Reads a path, a mode, and whether one key is present. It never renders
    /// the file, and it creates nothing.
    fn config_mode(&mut self) {
        // Loaded again rather than carried from `archive`: this check needs the
        // path the values came from and whether a key is among them, and a
        // second read of one small file is cheaper than a field threaded
        // through the store checks. A load failure is not fatal here - it is
        // the `Unknown` arm below.
        let loaded = Config::load();
        let (path, key) = match &loaded {
            Ok(config) => (
                config.source_path().map(Path::to_path_buf),
                Some(config.provider_api_key().is_some()),
            ),
            // The file is there and did not parse, or the directory did not
            // resolve. Either way the mode is still reportable and whether a
            // key is in it is not.
            Err(_) => (
                config_dir().ok().map(|dir| dir.join(CONFIG_FILE_NAME)),
                None,
            ),
        };
        let Some(path) = path else {
            self.push(Check::new(
                "config_mode",
                State::Unknown,
                "no config directory resolved, so there is no terminus.toml to look at",
            ));
            return;
        };
        let where_it_is = path.display().to_string();
        let fix = format!("chmod 600 '{where_it_is}'");
        let check = match credentials::permissions(&path) {
            Permissions::Absent => Check::new(
                "config_mode",
                State::Note,
                format!("nothing at {where_it_is}; terminus is running on its defaults"),
            ),
            Permissions::Owner { mode } => Check::new(
                "config_mode",
                State::Ok,
                format!("{where_it_is} is mode {mode:03o}"),
            ),
            Permissions::TooOpen { mode } => match key {
                Some(true) => Check::new(
                    "config_mode",
                    State::Problem,
                    format!(
                        "{where_it_is} is mode {mode:03o}, so the provider key in it is \
                         readable beyond its owner and terminus refuses to use it"
                    ),
                )
                .with_fix(fix),
                Some(false) => Check::new(
                    "config_mode",
                    State::Ok,
                    format!(
                        "{where_it_is} is mode {mode:03o} and holds no provider key, so \
                         there is no credential in it to refuse"
                    ),
                ),
                None => Check::new(
                    "config_mode",
                    State::Unknown,
                    format!(
                        "{where_it_is} is mode {mode:03o} and could not be parsed, so \
                         whether it holds a provider key is unknown"
                    ),
                )
                .with_fix(fix),
            },
            // D-15's deferral again. `mode_checks` is where it is said in
            // words, so that a Unix user reads it too.
            Permissions::Unchecked => Check::new(
                "config_mode",
                State::Unknown,
                format!(
                    "{where_it_is} is there; this build does not check Windows ACLs, so \
                     whether anyone else can read it is unverified"
                ),
            ),
            Permissions::Unreadable { detail } => Check::new(
                "config_mode",
                State::Problem,
                format!("{where_it_is} could not be read: {detail}"),
            ),
        };
        self.push(check);
    }

    /// What the two mode checks above actually examined (D-15, D-17).
    ///
    /// Its own check rather than a sentence inside either of them, because a
    /// check line is the only thing the printed report and the `--json`
    /// document share - and saying it inside both mode checks would say it
    /// twice. Stated on every platform, so a Unix user reads the same caveat a
    /// Windows user does rather than finding it only in an arm their machine
    /// never reaches.
    fn mode_checks(&mut self) {
        self.push(Check::new(
            "mode_checks",
            State::Note,
            "the credentials and config_mode checks read Unix mode bits; this build \
             examines no Windows ACL, so on Windows either file is accepted unverified",
        ));
    }

    /// Which Claude config roots resolved, and how many.
    ///
    /// More than one is worth naming rather than counting: every root is walked
    /// for transcripts, and only the first one's `settings.json` carries the
    /// hooks install wrote.
    fn roots(&mut self, config: &Config) {
        let roots = config.roots();
        let named = roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let check = match roots.len() {
            0 => Check::new(
                "config_roots",
                State::Problem,
                "no Claude config root resolved, so no transcript tree is walked",
            ),
            1 => Check::new("config_roots", State::Ok, format!("1 root: {named}")),
            count => Check::new(
                "config_roots",
                State::Note,
                format!(
                    "{count} roots: {named}; all of them are walked for transcripts and only the \
                     first one's settings file carries the hooks"
                ),
            ),
        };
        self.push(check);
    }

    /// The directory the store lives in, without creating it (AC6).
    ///
    /// A directory that is not there is what a machine looks like before its
    /// first ingest, so it is a state and not a failure. Writability is read out
    /// of the directory's permissions rather than probed with a file, because a
    /// probe file would create the thing this check is about.
    fn data_directory(&mut self, dir: &Path) {
        let check = match std::fs::metadata(dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Check::new(
                "data_directory",
                State::Note,
                format!(
                    "nothing at {} yet; the first ingest creates it",
                    dir.display()
                ),
            ),
            Err(e) => Check::new(
                "data_directory",
                State::Problem,
                format!("{} could not be read: {e}", dir.display()),
            ),
            Ok(meta) if !meta.is_dir() => Check::new(
                "data_directory",
                State::Problem,
                format!("{} is not a directory", dir.display()),
            ),
            // The mode's write bits, which is what `std` offers without a
            // `libc` dependency (D-04). A directory writable by somebody else
            // and not by this user reads as writable here; the ingest's own
            // error is the authority, and this catches the ordinary case of a
            // tree made read-only.
            Ok(meta) if meta.permissions().readonly() => Check::new(
                "data_directory",
                State::Problem,
                format!("{} is not writable, so no ingest can commit", dir.display()),
            )
            .with_fix(if cfg!(windows) {
                format!("attrib -r \"{}\"", dir.display())
            } else {
                format!("chmod u+w '{}'", dir.display())
            }),
            Ok(_) => Check::new("data_directory", State::Ok, dir.display().to_string()),
        };
        self.push(check);
    }

    /// The store, its counts, and the last run - one open, read-only, shared.
    ///
    /// Deliberately no blob walk: integrity over a 735 MB archive is
    /// `terminus verify`'s job and takes minutes, and doctor is what a user runs
    /// when something looks wrong.
    fn store(&mut self, data_dir: &Path, config: Config) {
        let reader = match read::open_in(data_dir, config) {
            Ok(Opened::Ready(reader)) => reader,
            Ok(Opened::Nothing(reason)) => {
                // The ordinary starting state, and `open_in` created nothing
                // finding it.
                self.push(Check::new(
                    "store",
                    State::Note,
                    format!("{reason}; the first ingest creates it"),
                ));
                self.unknown("last_run", "there is no store yet to read a run from");
                return;
            }
            Err(failure) => {
                self.push(Check::new("store", State::Problem, detail(failure)));
                self.unknown("last_run", "the store could not be opened");
                return;
            }
        };

        let path = reader.store().path().display().to_string();
        let conn = reader.store().conn();
        let check = match visible::counts(conn, reader.config()) {
            Err(error) => Check::new(
                "store",
                State::Problem,
                format!("{path} could not be counted: {error}"),
            ),
            Ok(counts) => {
                let finding = format!(
                    "{path}: {} session(s), {} turn(s), {} watermark(s)",
                    counts.sessions, counts.turns, counts.watermarks
                );
                if reader.store().predates_this_build() {
                    // Reported, never repaired (D-18). The next ingest rebuilds
                    // the derived tables on its own, so this is a state with a
                    // shortcut rather than a problem.
                    Check::new(
                        "store",
                        State::Note,
                        format!(
                            "{finding}; its derived tables predate this build, so results may be \
                             incomplete until the next ingest rebuilds them"
                        ),
                    )
                    .with_fix(format!("{} reindex", self.exe))
                } else {
                    Check::new("store", State::Ok, finding)
                }
            }
        };
        self.push(check);

        let count: Result<i64, String> = conn
            .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
            .map_err(|e| e.to_string());
        let check = match count {
            Err(error) => Check::new(
                "last_run",
                State::Problem,
                format!("the runs table could not be read: {error}"),
            ),
            Ok(0) => Check::new(
                "last_run",
                State::Note,
                "no ingest has run yet; the next hook Claude Code fires starts one",
            ),
            Ok(_) => {
                let last = conn
                    .query_row(
                        "SELECT started_at, coalesce(duration_ms, 0), files_seen, files_committed,
                                files_failed, turns_added, error
                         FROM runs ORDER BY id DESC LIMIT 1",
                        [],
                        |r| {
                            Ok(Run {
                                started_at: r.get(0)?,
                                duration_ms: r.get(1)?,
                                files_seen: r.get(2)?,
                                files_committed: r.get(3)?,
                                files_failed: r.get(4)?,
                                turns_added: r.get(5)?,
                                error: r.get(6)?,
                            })
                        },
                    )
                    .map_err(|e| e.to_string());
                match last {
                    Err(error) => Check::new(
                        "last_run",
                        State::Problem,
                        format!("the last run could not be read: {error}"),
                    ),
                    Ok(run) => self.last_run(&run),
                }
            }
        };
        self.push(check);
    }

    fn last_run(&self, run: &Run) -> Check {
        let finding = format!(
            "{}: {} file(s) walked, {} committed, {} failed, {} turn(s) added in {} ms",
            run.started_at,
            run.files_seen,
            run.files_committed,
            run.files_failed,
            run.turns_added,
            run.duration_ms
        );
        match (&run.error, run.files_failed) {
            (None, 0) => Check::new("last_run", State::Ok, finding),
            (error, _) => {
                // The first line only: a pass that failed on many files writes
                // one line per file, and the whole of it belongs to
                // `terminus status`, which prints every line of it.
                let first = error
                    .as_deref()
                    .and_then(|text| text.lines().next())
                    .unwrap_or("no error was recorded");
                let mut first = first.to_owned();
                first.truncate(MAX_ERROR_LINE);
                Check::new(
                    "last_run",
                    State::Problem,
                    format!("{finding}; {first}. `terminus status` prints the rest"),
                )
                .with_fix(format!("{} ingest", self.exe))
            }
        }
    }

    fn unknown(&mut self, name: &str, why: &str) {
        self.push(Check::new(name, State::Unknown, why));
    }

    // -----------------------------------------------------------------------
    // The two Claude Code settings terminus has an opinion about and never
    // changes (INST-06).
    //
    // Reported, and only reported. `cleanupPeriodDays` is how long Claude Code
    // keeps its own transcripts, which is terminus's second recovery path for a
    // session it never got to read; auto-compact spends tokens summarizing
    // context this store already holds losslessly. Both are the user's to set,
    // through their own editor or `/config`, and doctor prints the edit rather
    // than making it.
    // -----------------------------------------------------------------------

    fn claude_settings(&mut self) {
        let scopes = settings_scopes();
        self.cleanup_period(&scopes);
        self.auto_compact(&scopes);
    }

    fn cleanup_period(&mut self, scopes: &[(PathBuf, Json)]) {
        let found = effective(scopes, "cleanupPeriodDays");
        let (days, source, target) = match found {
            Some((path, value)) => match value.as_i64() {
                Some(days) => (days, format!("set in {}", path.display()), path.clone()),
                None => {
                    // A value Claude Code will not read as a number either.
                    self.push(Check::new(
                        "cleanup_period_days",
                        State::Problem,
                        format!("cleanupPeriodDays in {} is not a number", path.display()),
                    ));
                    return;
                }
            },
            None => (
                CLAUDE_DEFAULT_CLEANUP_DAYS,
                format!("unset, so Claude Code's default of {CLAUDE_DEFAULT_CLEANUP_DAYS} applies"),
                user_settings_path(scopes),
            ),
        };

        let finding = format!("cleanupPeriodDays is {days}, {source}");
        let check = if days < LOW_CLEANUP_DAYS {
            Check::new(
                "cleanup_period_days",
                State::Note,
                format!(
                    "{finding}. Claude Code deletes its own transcripts that many days after they \
                     are written; what terminus has archived stays, and a session it never got to \
                     read goes with them"
                ),
            )
            .with_fix(format!(
                "set \"cleanupPeriodDays\": {KEEP_CLEANUP_DAYS} in {}",
                target.display()
            ))
        } else {
            Check::new("cleanup_period_days", State::Ok, finding)
        };
        self.push(check);
    }

    fn auto_compact(&mut self, scopes: &[(PathBuf, Json)]) {
        // The environment variable outranks every settings file, so it is asked
        // first and named when it is what decided the answer.
        if let Some(value) = non_empty_var(AUTO_COMPACT_ENV) {
            let raw = value.to_string_lossy().into_owned();
            if disabling(&raw) {
                self.push(Check::new(
                    "auto_compact",
                    State::Ok,
                    format!("auto-compact is off: {AUTO_COMPACT_ENV}={raw} in this environment"),
                ));
                return;
            }
        }

        let found = effective(scopes, "autoCompactEnabled");
        let (enabled, source, target) = match found {
            Some((path, value)) => match as_bool(value) {
                Some(enabled) => (enabled, format!("set in {}", path.display()), path.clone()),
                None => {
                    self.push(Check::new(
                        "auto_compact",
                        State::Problem,
                        format!(
                            "autoCompactEnabled in {} is neither true nor false",
                            path.display()
                        ),
                    ));
                    return;
                }
            },
            None => (
                true,
                "unset, so Claude Code's default applies".to_owned(),
                user_settings_path(scopes),
            ),
        };

        let check = if enabled {
            Check::new(
                "auto_compact",
                State::Note,
                format!(
                    "autoCompactEnabled is true, {source}. Terminus never changes it and \
                     recommends off: compaction spends tokens summarizing context this store \
                     already holds losslessly, and a fresh session plus targeted recall beats a \
                     self-summarization"
                ),
            )
            .with_fix(format!(
                "set \"autoCompactEnabled\": false in {} (or /config inside Claude Code)",
                target.display()
            ))
        } else {
            Check::new(
                "auto_compact",
                State::Ok,
                format!("autoCompactEnabled is false, {source}"),
            )
        };
        self.push(check);
    }
}

/// Every settings file that can carry one of these two keys, lowest precedence
/// first.
///
/// Claude Code's own order: the user file [`super::install`] writes, then the
/// project's `.claude/settings.json`, then the project's
/// `.claude/settings.local.json`. A scope with no file is a scope reporting
/// nothing rather than an error - a user with no project settings is the
/// ordinary case - and so is one whose file will not parse, because doctor is
/// reporting what Claude Code would use and reading a file it cannot read is
/// not that.
fn settings_scopes() -> Vec<(PathBuf, Json)> {
    let mut paths = Vec::new();
    if let Ok(user) = targets::settings_path() {
        paths.push(user);
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".claude").join("settings.json"));
        paths.push(cwd.join(".claude").join("settings.local.json"));
    }
    paths
        .into_iter()
        .filter_map(|path| {
            let document = Document::read(&path).ok()?;
            // A file that is not there reads as an empty object, and an empty
            // object holds neither key, so it is dropped here rather than
            // reported as a scope that said nothing.
            (!document.source().trim().is_empty()).then(|| (path, document.value().clone()))
        })
        .collect()
}

/// The value Claude Code would use for `key`, and the file it would take it
/// from.
fn effective<'a>(scopes: &'a [(PathBuf, Json)], key: &str) -> Option<(&'a PathBuf, &'a Json)> {
    // Reversed: the last scope is the highest precedence, and the first one
    // holding the key from that end is the one that wins.
    scopes
        .iter()
        .rev()
        .find_map(|(path, value)| value.get(key).map(|found| (path, found)))
}

/// Where an edit belongs when no scope sets the key: the user's own file, which
/// is the one install writes and the one that exists on every machine.
fn user_settings_path(scopes: &[(PathBuf, Json)]) -> PathBuf {
    match targets::settings_path() {
        Ok(path) => path,
        Err(_) => scopes
            .first()
            .map(|(path, _)| path.clone())
            .unwrap_or_else(|| PathBuf::from("settings.json")),
    }
}

/// A JSON `true` or `false`, as the source text it was written as.
///
/// `json_file` keeps every scalar as the text it arrived as, so this is the
/// whole of decoding a boolean, and anything else - `"true"`, `1`, `null` - is
/// deliberately not one.
fn as_bool(value: &Json) -> Option<bool> {
    match value {
        Json::Scalar(raw) if raw == "true" => Some(true),
        Json::Scalar(raw) if raw == "false" => Some(false),
        _ => None,
    }
}

/// Does this value of `DISABLE_AUTO_COMPACT` actually disable anything?
///
/// `0` and `false` are read as "no", because a variable set to `0` that turned
/// the feature off would be the opposite of what it says, and doctor would then
/// report auto-compact as off on a machine where it is on.
fn disabling(raw: &str) -> bool {
    !matches!(raw.trim().to_ascii_lowercase().as_str(), "0" | "false")
}

fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
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
/// is `terminus_core::project`'s `git` call with the same reasoning behind every
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

/// The host out of a configured `base_url`, by hand (D-16).
///
/// No `url` crate and no regex crate: this workspace adds neither, `base_url`
/// is already treated as a string everywhere else it is used
/// (`observe::provider::endpoint`), and the whole question here is which of
/// four shapes a host is written in.
///
/// The userinfo rule is the one that earns its keep.
/// `http://127.0.0.1:11434@evil.example/` is a URL whose host is
/// `evil.example`, and a scan that stopped at the first `:` or the first `@`
/// would call it loopback and stay quiet about the one address a user could be
/// fooled by.
fn host_of(base_url: &str) -> &str {
    let rest = match base_url.find("://") {
        Some(at) => &base_url[at + 3..],
        None => base_url,
    };
    // The authority ends where the path, the query or the fragment begins.
    let authority = match rest.find(['/', '?', '#']) {
        Some(at) => &rest[..at],
        None => rest,
    };
    // Everything up to and including the LAST `@` is userinfo, which may itself
    // contain an `@`.
    let authority = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    if let Some(inside) = authority.strip_prefix('[') {
        // A bracketed IPv6 literal: `[::1]:11434`. The colons are the address.
        return match inside.find(']') {
            Some(at) => &inside[..at],
            None => inside,
        };
    }
    match authority.rfind(':') {
        // A port, and only when what follows one could be: an unbracketed
        // address full of colons is not a host with a port.
        Some(at) if authority[at + 1..].bytes().all(|b| b.is_ascii_digit()) => &authority[..at],
        _ => authority,
    }
}

/// Is this host on this machine?
///
/// Exactly three shapes, and no name resolution: `localhost` however it is
/// cased, any `127.x.x.x`, and the IPv6 loopback. Anything else is remote for
/// the purposes of the note - including a name that happens to resolve to
/// `127.0.0.1`, because looking that up would be the connection PRIV-03 bars.
fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host == "::1" {
        return true;
    }
    let octets: Vec<&str> = host.split('.').collect();
    octets.len() == 4
        && octets[0] == "127"
        && octets
            .iter()
            .all(|octet| !octet.is_empty() && octet.bytes().all(|b| b.is_ascii_digit()))
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
    let width = checks
        .iter()
        .map(|check| check.name.len())
        .max()
        .unwrap_or(0);
    let mut out = String::from("terminus doctor\n\n");
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
/// `terminus doctor | head` closes the pipe after the first page. The verdict
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
