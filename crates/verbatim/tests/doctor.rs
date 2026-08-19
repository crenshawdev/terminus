//! `verbatim doctor`: the report, the commands it prints, and the fact that it
//! writes nothing anywhere (INST-06, AC6).
//!
//! Every spawn here points `VERBATIM_BIN_DIR`, `CLAUDE_CONFIG_DIR`,
//! `VERBATIM_DATA_DIR`, `VERBATIM_CONFIG_DIR` and `HOME` at temporary
//! directories, for the reason `tests/install.rs` states: without them a test
//! would read - and its `install` half would write - the settings files and the
//! `~/.local/bin/verbatim` this machine is actually running on.
//!
//! The assertions read the report rather than grepping it. Doctor prints one
//! line per check, `<state>  <name>  <finding>`, with a `fix` line under any
//! check that has a command, so a test can ask "what state is `binary` in" and
//! "what command did it print" without a substring match that would pass for
//! the wrong reason.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The events install writes an entry for. `cmd::hook::EVENTS` spelled again:
/// `verbatim` is a binary crate with no library target, so a test cannot name
/// the constant and has to agree with it.
const EVENTS: [&str; 4] = [
    "SessionStart",
    "UserPromptSubmit",
    "SessionEnd",
    "PostCompact",
];

/// A `settings.json` shaped like the real one, and the same seed
/// `tests/install.rs` uses: keys in an order no sort produces, and a `hooks`
/// object already holding the user's own script.
const SETTINGS: &str = r#"{
  "cleanupPeriodDays": 7,
  "hooks": {
    "SessionStart": [],
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "$HOME/.claude/hooks/terse-answers.sh",
            "timeout": 5
          }
        ]
      }
    ]
  },
  "autoCompactEnabled": true,
  "theme": "dark"
}
"#;

const CLAUDE_JSON: &str = r#"{
  "numStartups": 412,
  "mcpServers": {
    "context7": {
      "type": "http",
      "url": "https://example.invalid/mcp"
    }
  }
}"#;

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    bin_dir: PathBuf,
    claude_dir: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let bin_dir = root.join("bin");
    let claude_dir = root.join("claude");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    std::fs::write(claude_dir.join("settings.json"), SETTINGS).unwrap();
    std::fs::write(claude_dir.join(".claude.json"), CLAUDE_JSON).unwrap();
    Fixture {
        _dir: dir,
        root,
        bin_dir,
        claude_dir,
    }
}

impl Fixture {
    fn stable(&self) -> PathBuf {
        self.bin_dir.join(if cfg!(windows) {
            "verbatim.exe"
        } else {
            "verbatim"
        })
    }

    fn settings(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    fn env(&self, command: &mut Command) {
        command
            .env("VERBATIM_BIN_DIR", &self.bin_dir)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("VERBATIM_DATA_DIR", self.root.join("data"))
            .env("VERBATIM_CONFIG_DIR", self.root.join("config"))
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root);
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
        command.args(args);
        // Inside the fixture, because doctor reads the project settings scopes
        // relative to the working directory: a `.claude/settings.json` beside
        // the checkout would otherwise decide what these tests measure.
        command.current_dir(&self.root);
        self.env(&mut command);
        command
    }

    /// Run with stdin at end of file, which is what a command in a script sees.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .stdin(Stdio::null())
            .output()
            .expect("the binary runs")
    }

    /// Wire verbatim in, the way a user would, answering every question with
    /// its default.
    fn install(&self) -> &Self {
        let out = self.run(&["install", "--yes"]);
        assert!(
            out.status.success(),
            "install did not succeed: {}",
            text(&out)
        );
        self
    }

    /// `verbatim doctor`, as its exit code and its parsed report.
    fn doctor(&self) -> (Option<i32>, Report) {
        self.doctor_with(&[])
    }

    /// The same, with something extra in the environment.
    fn doctor_with(&self, env: &[(&str, &str)]) -> (Option<i32>, Report) {
        let mut command = self.command(&["doctor"]);
        for (name, value) in env {
            command.env(name, value);
        }
        let out = command.stdin(Stdio::null()).output().expect("the binary runs");
        (out.status.code(), Report::parse(&text(&out)))
    }

    /// Run a command doctor printed, through a shell, exactly as it was
    /// printed.
    ///
    /// The point of AC6 is that the command in the report is a command: not a
    /// description, not a sentence with a path in it. So the test runs the
    /// string rather than asserting on its text, and answers the confirmation
    /// `install` asks (INST-03) the way a user at a terminal would.
    #[cfg(unix)]
    fn shell(&self, line: &str) -> Output {
        let mut command = Command::new("sh");
        command.arg("-c").arg(line);
        self.env(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh runs");
        child.stdin.take().unwrap().write_all(b"y\n").unwrap();
        child.wait_with_output().unwrap()
    }
}

/// What doctor said, check by check.
struct Report {
    checks: BTreeMap<String, Check>,
    /// The order the checks were printed in, which is the order the report is
    /// meant to be read in.
    order: Vec<String>,
    whole: String,
}

#[derive(Clone)]
struct Check {
    state: String,
    finding: String,
    fix: Option<String>,
}

impl Report {
    fn parse(text: &str) -> Report {
        const STATES: [&str; 4] = ["ok", "note", "unknown", "problem"];
        let mut checks: BTreeMap<String, Check> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for line in text.lines() {
            let Some(body) = line.strip_prefix("  ") else {
                continue;
            };
            let mut fields = body.split_whitespace();
            let Some(first) = fields.next() else {
                continue;
            };
            if first == "fix" {
                let last = order.last().expect("a fix line before any check");
                checks.get_mut(last).unwrap().fix =
                    Some(fields.collect::<Vec<_>>().join(" "));
                continue;
            }
            if !STATES.contains(&first) {
                continue;
            }
            let name = fields.next().expect("a check line carries a name").to_owned();
            let finding = fields.collect::<Vec<_>>().join(" ");
            order.push(name.clone());
            checks.insert(
                name,
                Check {
                    state: first.to_owned(),
                    finding,
                    fix: None,
                },
            );
        }
        Report {
            checks,
            order,
            whole: text.to_owned(),
        }
    }

    fn check(&self, name: &str) -> &Check {
        self.checks
            .get(name)
            .unwrap_or_else(|| panic!("no check named {name} in:\n{}", self.whole))
    }

    fn state(&self, name: &str) -> &str {
        &self.check(name).state
    }

    fn problems(&self) -> Vec<&str> {
        self.order
            .iter()
            .filter(|name| self.checks[*name].state == "problem")
            .map(String::as_str)
            .collect()
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

// ---------------------------------------------------------------------------
// The wiring checks
// ---------------------------------------------------------------------------

/// The state install leaves behind is the state doctor calls fine.
///
/// Both halves matter: a doctor that reported a problem after a successful
/// install would be crying wolf, and one that reported ok whatever it found
/// would be worth nothing. The rest of this file is the second half.
#[test]
fn after_install_every_wiring_check_is_ok() {
    let fixture = fixture();
    let (code, report) = fixture.install().doctor();

    assert_eq!(
        report.problems(),
        Vec::<&str>::new(),
        "a freshly installed machine reported problems:\n{}",
        report.whole
    );
    assert_eq!(code, Some(0));
    assert_eq!(report.state("binary"), "ok");
    assert_eq!(report.state("mcp_server"), "ok");
    for event in EVENTS {
        assert_eq!(report.state(&format!("hook_{event}")), "ok", "{event}");
    }
    assert!(
        report.check("binary").finding.contains("0.1.0"),
        "the binary check should name the version it found: {}",
        report.check("binary").finding
    );
}

/// AC6, the whole of it: a problem, a command, and the command works.
///
/// The command is run rather than matched, because the requirement is not that
/// doctor prints a plausible string - it is that what it prints fixes the thing
/// it reported.
#[cfg(unix)]
#[test]
fn a_missing_binary_is_a_problem_whose_printed_command_fixes_it() {
    let fixture = fixture();
    fixture.install();
    std::fs::remove_file(fixture.stable()).unwrap();

    let (code, report) = fixture.doctor();
    assert_eq!(code, Some(1), "a missing binary must not exit 0");
    assert_eq!(report.state("binary"), "problem");
    let fix = report
        .check("binary")
        .fix
        .clone()
        .unwrap_or_else(|| panic!("the binary problem printed no command:\n{}", report.whole));

    let out = fixture.shell(&fix);
    assert!(
        out.status.success(),
        "`{fix}` did not succeed: {}",
        text(&out)
    );

    let (code, report) = fixture.doctor();
    assert_eq!(
        report.state("binary"),
        "ok",
        "the printed command did not fix what it was printed for:\n{}",
        report.whole
    );
    assert_eq!(code, Some(0));
}

/// An entry that runs some other copy of verbatim is a different finding from a
/// missing one, and it names the event it is under.
///
/// This is what a stale install leaves: the entry is there, it is in exec form,
/// and the binary it names is gone. A check that only counted entries would
/// report it as wired up.
#[test]
fn an_entry_pointing_somewhere_else_names_its_event() {
    let fixture = fixture();
    fixture.install();

    let mut settings = read(&fixture.settings());
    let stable = serde_json::Value::from(fixture.stable().display().to_string());
    let mut rewritten = 0;
    for group in settings["hooks"]["SessionStart"].as_array_mut().unwrap() {
        for entry in group["hooks"].as_array_mut().unwrap() {
            if entry["command"] == stable {
                entry["command"] = serde_json::Value::from("/nonexistent/verbatim");
                rewritten += 1;
            }
        }
    }
    assert_eq!(rewritten, 1, "install wrote no SessionStart entry to rewrite");
    write(&fixture.settings(), &settings);

    let (code, report) = fixture.doctor();
    assert_eq!(code, Some(1));
    assert_eq!(report.state("hook_SessionStart"), "problem");
    let finding = &report.check("hook_SessionStart").finding;
    assert!(
        finding.contains("SessionStart") && finding.contains("/nonexistent/verbatim"),
        "the finding names neither the event nor where the entry points: {finding}"
    );
    // The other three are untouched, which is what makes this a finding about
    // one event rather than about the file.
    for event in ["UserPromptSubmit", "SessionEnd", "PostCompact"] {
        assert_eq!(report.state(&format!("hook_{event}")), "ok", "{event}");
    }
}

/// A duplicate is the third finding: not missing, not pointing elsewhere, and
/// not something `install` can fix, because install treats one entry of its own
/// as done.
#[test]
fn a_duplicated_entry_is_its_own_finding() {
    let fixture = fixture();
    fixture.install();

    let mut settings = read(&fixture.settings());
    let groups = settings["hooks"]["SessionEnd"].as_array().unwrap().clone();
    settings["hooks"]["SessionEnd"]
        .as_array_mut()
        .unwrap()
        .extend(groups);
    write(&fixture.settings(), &settings);

    let (code, report) = fixture.doctor();
    assert_eq!(code, Some(1));
    assert_eq!(report.state("hook_SessionEnd"), "problem");
    assert!(
        report.check("hook_SessionEnd").finding.contains("SessionEnd"),
        "{}",
        report.check("hook_SessionEnd").finding
    );
    assert_eq!(report.state("hook_SessionStart"), "ok");
}

/// A settings file that is not JSON is one problem with the file named, not
/// four identical hook findings and a guess.
#[test]
fn a_settings_file_that_is_not_json_is_reported_as_itself() {
    let fixture = fixture();
    fixture.install();
    std::fs::write(fixture.settings(), "{ this is not json").unwrap();

    let (code, report) = fixture.doctor();
    assert_eq!(code, Some(1));
    assert_eq!(report.state("settings_file"), "problem");
    for event in EVENTS {
        assert_eq!(
            report.state(&format!("hook_{event}")),
            "unknown",
            "an unreadable settings file cannot make {event} a known state"
        );
    }
}

/// An mcp registration pointing at a binary that has moved is a problem, and
/// install is the fix: it repairs a registration of its own in place.
#[test]
fn an_mcp_registration_that_points_elsewhere_is_a_problem() {
    let fixture = fixture();
    fixture.install();

    let claude_json = fixture.claude_dir.join(".claude.json");
    let mut value = read(&claude_json);
    value["mcpServers"]["verbatim"]["command"] = serde_json::Value::from("/old/bin/verbatim");
    write(&claude_json, &value);

    let (_, report) = fixture.doctor();
    assert_eq!(report.state("mcp_server"), "problem");
    assert!(
        report.check("mcp_server").finding.contains("/old/bin/verbatim"),
        "{}",
        report.check("mcp_server").finding
    );
    assert!(report.check("mcp_server").fix.is_some());
}

// ---------------------------------------------------------------------------
// The store and the data directory, neither of which doctor creates
// ---------------------------------------------------------------------------

/// AC6's first half: a machine before its first ingest is a state, not a
/// failure, and the report leaves it exactly that.
///
/// The listing is taken of the *parent*, before and after, so a created WAL
/// file, a `LOCK`, or a directory made on the way to somewhere else fails this
/// test too. `Store::open` would create all three (D-12), which is why doctor
/// opens through the read path instead.
#[test]
fn a_data_directory_that_does_not_exist_is_a_state_and_stays_absent() {
    let fixture = fixture();
    fixture.install();

    let data_dir = fixture.root.join("data");
    assert!(!data_dir.exists(), "install created the data directory");
    let before = entries(&fixture.root);

    let (code, report) = fixture.doctor();
    assert_eq!(
        code,
        Some(0),
        "a machine that has never ingested is not a failure:\n{}",
        report.whole
    );
    assert_eq!(report.state("store"), "note");
    assert_eq!(report.state("data_directory"), "note");
    assert!(
        report.check("store").finding.contains("no verbatim store"),
        "{}",
        report.check("store").finding
    );
    assert!(!data_dir.exists(), "doctor created the data directory");
    assert_eq!(
        entries(&fixture.root),
        before,
        "doctor created something beside the data directory"
    );
}

/// After one ingest the same checks carry the numbers, which is the half that
/// proves the read path was really opened rather than reported as absent.
#[test]
fn after_one_ingest_the_store_and_the_last_run_are_reported() {
    let fixture = fixture();
    fixture.install();

    // A `<uuid>.jsonl` directly inside a project directory, because that is
    // what `discover` calls a transcript (D-16); a fixture kept under its own
    // name would be walked past and the store would stay empty.
    let project = fixture.claude_dir.join("projects").join("-data-code-x");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("44444444-4444-4444-8444-444444444444.jsonl"),
        verbatim_core::testkit::fixture_bytes("session-basic.jsonl"),
    )
    .unwrap();
    let ingest = fixture.run(&["ingest"]);
    assert!(ingest.status.success(), "ingest failed: {}", text(&ingest));

    let (code, report) = fixture.doctor();
    assert_eq!(code, Some(0), "{}", report.whole);
    assert_eq!(report.state("store"), "ok");
    assert!(
        report.check("store").finding.contains("1 session(s)"),
        "the store check should carry the session count: {}",
        report.check("store").finding
    );
    assert_eq!(report.state("data_directory"), "ok");
    assert_eq!(report.state("last_run"), "ok");
    let last = &report.check("last_run").finding;
    assert!(
        last.starts_with("20") && last.contains("committed"),
        "the last run should be reported by its timestamp: {last}"
    );
}

/// Every name directly inside `dir`, sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

// ---------------------------------------------------------------------------
// The two settings doctor reads and never changes
// ---------------------------------------------------------------------------

/// Both values, the file each came from, and not one byte written back.
///
/// The byte comparison is the requirement: INST-06 says doctor never repairs,
/// and the settings file is the thing it would be most tempting to repair,
/// since `install` already knows how to raise `cleanupPeriodDays`.
#[test]
fn the_settings_it_reads_are_reported_and_left_exactly_as_they_are() {
    let fixture = fixture();
    fixture.install();

    let before = std::fs::read(fixture.settings()).unwrap();
    let (code, report) = fixture.doctor();
    assert_eq!(code, Some(0), "an advisory is not a failure:\n{}", report.whole);

    let cleanup = &report.check("cleanup_period_days").finding;
    assert!(
        cleanup.contains("7") && cleanup.contains(&fixture.settings().display().to_string()),
        "the finding names neither the value nor the file it came from: {cleanup}"
    );
    assert_eq!(report.state("cleanup_period_days"), "note");
    assert!(report.check("cleanup_period_days").fix.is_some());

    let compact = &report.check("auto_compact").finding;
    assert!(
        compact.contains("true") && compact.contains(&fixture.settings().display().to_string()),
        "the finding names neither the value nor the file it came from: {compact}"
    );

    assert_eq!(
        std::fs::read(fixture.settings()).unwrap(),
        before,
        "doctor changed the settings file it was reading"
    );
}

/// The environment variable outranks the settings file, and the finding says
/// so rather than reporting the value the file carries.
#[test]
fn disable_auto_compact_in_the_environment_wins_and_is_named() {
    let fixture = fixture();
    fixture.install();

    let (code, report) = fixture.doctor_with(&[("DISABLE_AUTO_COMPACT", "1")]);
    assert_eq!(code, Some(0));
    assert_eq!(report.state("auto_compact"), "ok");
    let finding = &report.check("auto_compact").finding;
    assert!(
        finding.contains("DISABLE_AUTO_COMPACT"),
        "the winning source is not named: {finding}"
    );
    assert!(
        !finding.contains(&fixture.settings().display().to_string()),
        "the settings file did not decide this and should not be named: {finding}"
    );
}

/// A project scope outranks the user file, which is the order Claude Code
/// itself resolves them in.
#[test]
fn a_project_settings_file_outranks_the_user_one() {
    let fixture = fixture();
    fixture.install();

    let project = fixture.root.join(".claude");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("settings.local.json"),
        r#"{"cleanupPeriodDays": 3650}"#,
    )
    .unwrap();

    let (code, report) = fixture.doctor();
    assert_eq!(code, Some(0));
    assert_eq!(
        report.state("cleanup_period_days"),
        "ok",
        "the project value should be the effective one:\n{}",
        report.whole
    );
    let finding = &report.check("cleanup_period_days").finding;
    assert!(
        finding.contains("3650") && finding.contains("settings.local.json"),
        "{finding}"
    );
}
