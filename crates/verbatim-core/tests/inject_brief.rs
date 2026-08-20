//! The resume brief, against stores built by the ordinary ingest path.
//!
//! Every session here is a transcript this file wrote and `ingest::run`
//! archived, never rows a helper inserted: what INJ-01 is about is which
//! `session_meta` row a real pass produced, and a test that wrote the row
//! itself would be asserting against its own opinion of the schema.
//!
//! Each session's `cwd` is a directory beneath a root the test owns and
//! **nothing creates**, so project identity degrades to that path rather than
//! resolving through `git rev-parse` for a directory that happens to exist -
//! the same reason `testkit::FIXTURE_ROOT_TOKEN` exists.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use verbatim_core::config::Config;
use verbatim_core::ingest;
use verbatim_core::inject::{brief, Payload};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    Bench {
        _dir: dir,
        data_dir,
        work,
        root,
    }
}

/// One archived session, described by everything the brief reads off it.
struct Session<'a> {
    /// The transcript's path beneath the work directory. A path with a
    /// `subagents` component is a sidecar and gets a `parent_session_key`.
    file: &'a str,
    id: &'a str,
    project: &'a str,
    branch: Option<&'a str>,
    /// The `YYYY-MM-DD` every record of this session is stamped with.
    day: &'a str,
    prompt: &'a str,
    reply: &'a str,
}

impl Bench {
    fn project(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Archive one session through the ordinary ingest path.
    fn archive(&self, session: &Session<'_>) {
        let cwd = self.project(session.project);
        let mut body = String::new();
        for (n, (kind, text)) in [("user", session.prompt), ("assistant", session.reply)]
            .into_iter()
            .enumerate()
        {
            let mut record = serde_json::json!({
                "parentUuid": null,
                "isSidechain": false,
                "cwd": cwd.to_string_lossy(),
                "sessionId": session.id,
                "type": kind,
                "uuid": format!("{}-{n}", session.id),
                "timestamp": format!("{}T1{n}:00:00.000Z", session.day),
                "message": {
                    "role": kind,
                    "content": [{"type": "text", "text": text}],
                },
            });
            if let Some(branch) = session.branch {
                record["gitBranch"] = serde_json::Value::String(branch.to_owned());
            }
            body.push_str(&record.to_string());
            body.push('\n');
        }

        let path = self.work.join(session.file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{}: {other:?}", session.file),
        }
    }

    /// A config whose `[injection] brief_chars` is what the test says, loaded
    /// through the file the binary loads one from - the budget has to reach the
    /// query, and a struct built in memory would not prove that it does.
    fn config(&self, brief_chars: usize) -> Config {
        let dir = self.work.join(format!("config-{brief_chars}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("verbatim.toml"),
            format!(
                "roots = [{:?}]\n\n[injection]\nbrief_chars = {brief_chars}\n",
                self.work.to_string_lossy()
            ),
        )
        .unwrap();
        Config::load_from(&dir).unwrap()
    }

    /// The brief a `SessionStart` in one project would carry, at the default
    /// budget.
    fn brief(&self, project: &str) -> Option<String> {
        self.brief_with(project, &Config::default())
    }

    /// The brief a `SessionStart` in one project would carry, under one config.
    fn brief_with(&self, project: &str, config: &Config) -> Option<String> {
        let cwd = self.project(project);
        let payload = Payload {
            session_id: Some("0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55"),
            transcript_path: Some("/home/user/.claude/projects/-p/s.jsonl"),
            cwd: cwd.to_str(),
            prompt: None,
            source: Some("startup"),
        };
        brief::session_start(&self.data_dir, config, &payload)
    }
}

/// The two sessions of `project-alpha`, one clearly later than the other.
const EARLY: Session<'static> = Session {
    file: "early.jsonl",
    id: "11111111-1111-4111-8111-111111111111",
    project: "project-alpha",
    branch: Some("branch-of-the-early-one"),
    day: "2026-08-10",
    prompt: "the early prompt, about widgets",
    reply: "the early reply, about widgets",
};

const LATE: Session<'static> = Session {
    file: "late.jsonl",
    id: "22222222-2222-4222-8222-222222222222",
    project: "project-alpha",
    branch: Some("branch-of-the-late-one"),
    day: "2026-08-14",
    prompt: "the late prompt, about sprockets",
    reply: "the late reply, about sprockets",
};

/// D-17: the session with the greatest `last_turn_at`, at day resolution, with
/// the branch it ended on and the last thing said in each direction.
#[test]
fn the_brief_names_the_last_session_of_the_project_and_nothing_of_the_earlier_one() {
    let bench = bench();
    // Archived in the order that would trap an implementation reading the first
    // or the newest row rather than the greatest timestamp.
    bench.archive(&LATE);
    bench.archive(&EARLY);

    let brief = bench.brief("project-alpha").expect("a brief");

    for present in [
        LATE.day,
        LATE.branch.unwrap(),
        "sprockets",
        // The pointer is still there: the last-session block is added to the
        // brief, not swapped for it.
        "recall_search",
    ] {
        assert!(
            brief.contains(present),
            "the brief omits {present:?}: {brief}"
        );
    }
    for absent in [EARLY.day, EARLY.branch.unwrap(), "widgets"] {
        assert!(
            !brief.contains(absent),
            "the brief carries the earlier session's {absent:?}: {brief}"
        );
    }
    // Both directions of the exchange, not just the prompt.
    assert!(
        brief.contains(LATE.prompt) && brief.contains(LATE.reply),
        "the brief carries only one side of the last exchange: {brief}"
    );
}

/// A subagent sidecar is not a session the brief may name, however recent it is
/// (`DESIGN-BRIEF.md:239`). Its `parent_session_key` is what excludes it, and
/// that column comes from the sidecar's path (D-03).
#[test]
fn a_later_subagent_sidecar_does_not_become_the_named_session() {
    let bench = bench();
    bench.archive(&EARLY);
    bench.archive(&LATE);
    bench.archive(&Session {
        // `<project>/<sessionId>/subagents/agent-*.jsonl` is what
        // `lineage::sidecar_parent` reads a parent out of.
        file: "33333333-3333-4333-8333-333333333333/subagents/agent-late.jsonl",
        id: "33333333-3333-4333-8333-333333333333",
        project: "project-alpha",
        branch: Some("branch-of-the-subagent"),
        day: "2026-08-20",
        prompt: "the subagent prompt, about flanges",
        reply: "the subagent reply, about flanges",
    });

    // The falsifying check: the sidecar really is archived, and really is the
    // most recent thing in the project, so what follows is the parent filter
    // and not an ingest that quietly skipped the file.
    let conn = rusqlite::Connection::open(bench.data_dir.join(verbatim_core::store::DB_FILE_NAME))
        .unwrap();
    let (newest, parent): (String, Option<String>) = conn
        .query_row(
            "SELECT last_turn_at, parent_session_key FROM session_meta \
             ORDER BY last_turn_at DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        newest.starts_with("2026-08-20") && parent.is_some(),
        "the sidecar is not the newest archived session with a parent: \
         {newest} {parent:?}"
    );

    let brief = bench.brief("project-alpha").expect("a brief");
    assert!(
        brief.contains(LATE.day) && brief.contains(LATE.branch.unwrap()),
        "the brief does not name the last non-subagent session: {brief}"
    );
    for absent in ["2026-08-20", "branch-of-the-subagent", "flanges"] {
        assert!(
            !brief.contains(absent),
            "the brief carries the subagent's {absent:?}: {brief}"
        );
    }
}

/// A session whose records carry no `gitBranch` at all - a transcript from a
/// directory that is not a repository. The block it cannot render is the branch,
/// and the rest of the brief is unaffected.
#[test]
fn a_session_with_no_branch_still_renders_the_rest() {
    let bench = bench();
    bench.archive(&Session {
        file: "unversioned.jsonl",
        id: "44444444-4444-4444-8444-444444444444",
        project: "project-beta",
        branch: None,
        day: "2026-08-11",
        prompt: "the prompt from a directory with no repository",
        reply: "the reply from a directory with no repository",
    });

    let brief = bench.brief("project-beta").expect("a brief");
    assert!(
        brief.contains("2026-08-11") && brief.contains("no repository"),
        "the brief lost the blocks it could render: {brief}"
    );
    assert!(
        !brief.contains("branch"),
        "the brief names a branch for a session that has none: {brief}"
    );
}

/// D-10 and D-18, asserted against the source rather than against behaviour.
///
/// Neither is observable from the outside: a `git` subprocess that costs 10-30
/// ms still returns the right answer, and a query against `observations` would
/// only fail once phase 7 exists to make the table appear on some machines and
/// not others. The comments are stripped first because this module's own doc
/// explains at length why neither appears - a raw grep would match the
/// explanation and never the thing.
#[test]
fn nothing_in_the_injection_path_spawns_a_process_or_reads_observations() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/inject");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("read src/inject") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let code: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for banned in ["Command", "observations"] {
            assert!(!code.contains(banned), "{} names {banned}", path.display());
        }
        checked += 1;
    }
    assert!(checked >= 3, "only {checked} files in {}", dir.display());
}

// ---------------------------------------------------------------------------
// The budget (INJ-01, D-16)

/// A turn long enough that the brief cannot carry both of them whole.
fn long_turn(subject: &str) -> String {
    format!("{subject} ").repeat(400)
}

/// The variable parts are cut and the fixed ones survive: a budget is a reason
/// to quote less of the last exchange, never a reason to drop the date, the
/// branch or the pointer.
#[test]
fn a_configured_budget_cuts_the_quoted_turns_and_keeps_the_date_branch_and_counts() {
    let bench = bench();
    let prompt = long_turn("a very long prompt about sprockets");
    let reply = long_turn("a very long reply about sprockets");
    bench.archive(&Session {
        prompt: &prompt,
        reply: &reply,
        ..LATE
    });

    const BUDGET: usize = 500;
    let brief = bench
        .brief_with("project-alpha", &bench.config(BUDGET))
        .expect("a brief");

    // The falsifying half: an uncut brief is longer than the budget, so what
    // follows is a cut and not a brief that happened to fit.
    let uncut = bench.brief("project-alpha").expect("a brief");
    assert!(
        uncut.chars().count() > BUDGET,
        "the unbudgeted brief is only {} characters, so this test proves \
         nothing about cutting: {uncut}",
        uncut.chars().count()
    );

    assert!(
        brief.chars().count() <= BUDGET,
        "the brief is {} characters against a budget of {BUDGET}: {brief}",
        brief.chars().count()
    );
    for kept in [
        LATE.day,
        LATE.branch.unwrap(),
        "1 session",
        "2 turns",
        "recall_search",
    ] {
        assert!(
            brief.contains(kept),
            "the budget dropped {kept:?} instead of cutting a quoted turn: {brief}"
        );
    }
    assert!(
        brief.contains(verbatim_core::recall::excerpt::ELISION),
        "the brief was cut without saying so: {brief}"
    );
}

/// The ceiling that is not the user's to raise (bundle 2.1.237 persists a hook
/// stdout over 10,000 characters to disk and hands the model a file reference
/// instead of the text).
///
/// The over-long part here is the project key itself, which is the one piece of
/// a brief with no bound of its own: it is a `cwd` string out of a transcript,
/// it appears twice, and no cut to the quoted turns can shorten it. A path this
/// long cannot exist on disk, which is exactly why the resolver degrades to the
/// string and the key becomes it.
#[test]
fn no_brief_exceeds_the_ceiling_whatever_the_config_says() {
    let bench = bench();
    let huge = format!("project-{}", "d".repeat(11_000));
    bench.archive(&Session {
        file: "huge.jsonl",
        project: &huge,
        ..LATE
    });

    let brief = bench
        .brief_with(&huge, &bench.config(brief::MAX_BRIEF_CHARS * 2))
        .expect("a brief");

    assert!(
        brief.chars().count() <= brief::MAX_BRIEF_CHARS,
        "the brief is {} characters against a ceiling of {}",
        brief.chars().count(),
        brief::MAX_BRIEF_CHARS
    );
    assert!(
        brief.ends_with(verbatim_core::recall::excerpt::ELISION),
        "the brief was cut at the ceiling without saying so"
    );
}
