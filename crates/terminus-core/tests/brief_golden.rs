//! The whole resume brief, pinned byte for byte against a golden captured from stores this file builds.
//!
//! The golden was captured before the brief learned to read the summary tables, so it is the reference the new reads must match.
//! Every store goes through `ingest::run`, and the document is fixture-only: nothing in it comes from a real archive.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use terminus_core::config::Config;
use terminus_core::inject::{brief, Payload};
use terminus_core::retention::{self, Selection};
use terminus_core::store::Store;
use terminus_core::{ingest, testkit};

/// Setting this rewrites the golden and fails that run, so the golden never re-baselines itself.
const REGENERATE_ENV: &str = "TERMINUS_REGENERATE_BRIEF_GOLDEN";

/// The checked-in document, relative to `crates/terminus-core/tests`.
const GOLDEN: &str = "goldens/resume-briefs.txt";

const HEADER: &str = "\
# The resume brief, rendered for seven shapes of project from stores the test builds.
#
# Regenerate deliberately, never as a fix for a failing run:
#   TERMINUS_REGENERATE_BRIEF_GOLDEN=1 cargo test -p terminus-core \\
#     --all-features --test brief_golden
#
# Every temporary path is written back to the fixture root token, because the
# brief's project label carries the tempdir and a golden holding it would
# differ on every run.
";

/// What a section holds when the brief renders nothing.
const NO_BRIEF: &str = "(no brief)";

struct Bench {
    dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir_in(testkit::temp_root()).unwrap();
    let data_dir = dir.path().join("data");
    let work = dir.path().join("work");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    Bench {
        dir,
        data_dir,
        work,
        root,
    }
}

/// One archived session, described by everything the brief reads off it.
struct Session<'a> {
    file: &'a str,
    id: &'a str,
    project: &'a str,
    branch: Option<&'a str>,
    day: &'a str,
    prompt: &'a str,
    reply: &'a str,
}

fn text_record(kind: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": kind,
        "message": {
            "role": kind,
            "content": [{"type": "text", "text": text}],
        },
    })
}

fn ai_title(title: &str) -> serde_json::Value {
    serde_json::json!({"type": "ai-title", "aiTitle": title})
}

/// One assistant turn calling one tool per path.
fn tool_calls(tool: &str, paths: &[String]) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = paths
        .iter()
        .enumerate()
        .map(|(n, path)| {
            serde_json::json!({
                "type": "tool_use",
                "id": format!("toolu_{tool}_{n}"),
                "name": tool,
                "input": {"file_path": path},
            })
        })
        .collect();
    serde_json::json!({
        "type": "assistant",
        "message": {"role": "assistant", "content": blocks},
    })
}

impl Bench {
    fn project(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// An absolute path inside one project, the way a tool call records it.
    fn inside(&self, project: &str, file: &str) -> String {
        self.project(project).join(file).display().to_string()
    }

    /// Archive one session through the ordinary ingest path: the prompt, then the reply.
    fn archive(&self, session: &Session<'_>) {
        self.archive_records(
            session,
            &[
                text_record("user", session.prompt),
                text_record("assistant", session.reply),
            ],
        );
    }

    /// Archive one session whose records are spelled out, at most nine of them.
    fn archive_records(&self, session: &Session<'_>, records: &[serde_json::Value]) {
        assert!(records.len() < 10, "the hour in the timestamp is one digit");
        let mut body = String::new();
        for (n, particular) in records.iter().enumerate() {
            body.push_str(&self.line(session, n, particular));
        }

        let path = self.work.join(session.file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{}: {other:?}", session.file),
        }
    }

    fn line(&self, session: &Session<'_>, n: usize, particular: &serde_json::Value) -> String {
        let cwd = self.project(session.project);
        let mut record = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "cwd": cwd.to_string_lossy(),
            "sessionId": session.id,
            "uuid": format!("{}-{n}", session.id),
            "timestamp": format!("{}T1{n}:00:00.000Z", session.day),
        });
        if let Some(branch) = session.branch {
            record["gitBranch"] = serde_json::Value::String(branch.to_owned());
        }
        for (key, value) in particular.as_object().expect("a record is an object") {
            record[key] = value.clone();
        }
        let mut line = record.to_string();
        line.push('\n');
        line
    }

    fn archive_titled(&self, session: &Session<'_>, title: &str) {
        self.archive_records(
            session,
            &[
                text_record("user", session.prompt),
                ai_title(title),
                text_record("assistant", session.reply),
            ],
        );
    }

    /// A repository under one project with `head` as its `HEAD`; call it after the archive calls.
    fn check_out(&self, project: &str, head: &str) {
        let git = self.project(project).join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), head).unwrap();
    }

    /// The brief a `SessionStart` in one project would carry, as a section's text.
    fn render(&self, project: &str, config: &Config) -> String {
        let cwd = self.project(project);
        let payload = Payload {
            session_id: Some("0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55"),
            transcript_path: Some("/home/user/.claude/projects/-p/s.jsonl"),
            cwd: cwd.to_str(),
            prompt: None,
            source: Some("startup"),
        };
        brief::session_start(&self.data_dir, config, &payload).unwrap_or_else(|| NO_BRIEF.into())
    }
}

fn on_branch(branch: &str) -> String {
    format!("ref: refs/heads/{branch}\n")
}

fn push_section(out: &mut String, name: &str, text: &str) {
    out.push_str(&format!("\n== {name}\n{text}\n"));
}

/// Every case, rendered in a fixed order into one document.
fn capture(bench: &Bench) -> String {
    let config = Config::default();
    let mut out = String::from(HEADER);

    // 1. Other sessions listed by title.
    let titled = |file: &'static str, id: &'static str, day: &'static str| Session {
        file,
        id,
        project: "listed",
        branch: None,
        day,
        prompt: "the prompt somebody typed in this session",
        reply: "the reply",
    };
    bench.archive_titled(&titled("listed/a.jsonl", "la", "2026-08-10"), "Oldest work");
    bench.archive_titled(&titled("listed/b.jsonl", "lb", "2026-08-11"), "Middle work");
    bench.archive_titled(
        &titled("listed/c.jsonl", "lc", "2026-08-12"),
        "Last session",
    );
    push_section(&mut out, "others-listed", &bench.render("listed", &config));

    // 2. The later-session line: the checked-out branch's session is the older one.
    let early = Session {
        file: "later/early.jsonl",
        id: "11111111-1111-4111-8111-111111111111",
        project: "later",
        branch: Some("branch-of-the-early-one"),
        day: "2026-08-10",
        prompt: "the early prompt, about widgets",
        reply: "the early reply, about widgets",
    };
    let late = Session {
        file: "later/late.jsonl",
        id: "22222222-2222-4222-8222-222222222222",
        branch: Some("branch-of-the-late-one"),
        day: "2026-08-14",
        prompt: "the late prompt, about sprockets",
        reply: "the late reply, about sprockets",
        ..early
    };
    bench.archive(&early);
    bench.archive(&late);
    bench.check_out("later", &on_branch("branch-of-the-early-one"));
    push_section(&mut out, "later-session", &bench.render("later", &config));

    // 3. An edited-files line that ends "and N more".
    let paths: Vec<String> = (0..11)
        .map(|n| bench.inside("edits", &format!("src/{n:02}.rs")))
        .collect();
    let editor = Session {
        file: "edits/editor.jsonl",
        id: "33333333-3333-4333-8333-333333333333",
        project: "edits",
        branch: Some("main-line"),
        day: "2026-08-14",
        prompt: "the prompt, about gaskets",
        reply: "the reply, about gaskets",
    };
    bench.archive_records(
        &editor,
        &[
            text_record("user", editor.prompt),
            tool_calls("Edit", &paths),
            text_record("assistant", editor.reply),
        ],
    );
    push_section(
        &mut out,
        "edited-files-more",
        &bench.render("edits", &config),
    );

    // 4. More than 64 main sessions, the newest all one-shots with no typed prompt.
    for n in 0..66usize {
        let file = format!("deep/shot-{n:02}.jsonl");
        let id = format!("deep-shot-{n:02}");
        let day = format!("2026-{:02}-{:02}", 9 + n / 28, 1 + n % 28);
        let shot = Session {
            file: &file,
            id: &id,
            project: "deep",
            branch: Some("main-line"),
            day: &day,
            prompt: "",
            reply: "a one-shot answer",
        };
        bench.archive_records(
            &shot,
            &[
                ai_title(&format!("One-shot {n:02}")),
                text_record("assistant", shot.reply),
            ],
        );
    }
    let main_typed = Session {
        file: "deep/typed-main.jsonl",
        id: "deep-typed-main",
        project: "deep",
        branch: Some("main-line"),
        day: "2026-07-02",
        prompt: "the typed prompt on the main line, about valves",
        reply: "the reply on the main line, about valves",
    };
    let side_typed = Session {
        file: "deep/typed-side.jsonl",
        id: "deep-typed-side",
        branch: Some("side-branch"),
        day: "2026-07-01",
        prompt: "the typed prompt on the side branch, about pistons",
        reply: "the reply on the side branch, about pistons",
        ..main_typed
    };
    bench.archive_titled(&main_typed, "Main line work");
    bench.archive_titled(&side_typed, "Side branch work");
    push_section(
        &mut out,
        "deep-no-repository",
        &bench.render("deep", &config),
    );
    bench.check_out("deep", &on_branch("side-branch"));
    push_section(
        &mut out,
        "deep-side-checked-out",
        &bench.render("deep", &config),
    );

    // 5. An evicted session keeps its head line, files and pointer, with no quotes.
    let evicted = Session {
        file: "evicted/gone.jsonl",
        id: "44444444-4444-4444-8444-444444444444",
        project: "evicted",
        branch: Some("main-line"),
        day: "2026-08-14",
        prompt: "the prompt that gets evicted, about bearings",
        reply: "the reply that gets evicted, about bearings",
    };
    let edited = vec![bench.inside("evicted", "src/lib.rs")];
    bench.archive_records(
        &evicted,
        &[
            text_record("user", evicted.prompt),
            tool_calls("Edit", &edited),
            text_record("assistant", evicted.reply),
        ],
    );
    let key = bench
        .work
        .join(evicted.file)
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    {
        let mut store = Store::open(&bench.data_dir).unwrap();
        let applied = retention::apply(
            &mut store,
            &Selection {
                evict: vec![key.clone()],
                ..Selection::default()
            },
        );
        assert_eq!(applied.evicted, vec![key]);
    }
    push_section(
        &mut out,
        "evicted-session",
        &bench.render("evicted", &config),
    );

    // 6. An excluded worktree: a repository session and a later session run from one of its worktrees.
    let repo = Session {
        file: "zeta-repo.jsonl",
        id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        project: "zeta",
        branch: Some("branch-of-the-repo-one"),
        day: "2026-08-10",
        prompt: "the repo prompt, about gaskets",
        reply: "the repo reply, about gaskets",
    };
    let forked = Session {
        file: "zeta-forked.jsonl",
        id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
        project: "zeta/.claude/worktrees/wt-a",
        branch: Some("branch-of-the-worktree-one"),
        day: "2026-08-14",
        prompt: "the worktree prompt, about flanges",
        reply: "the worktree reply, about flanges",
    };
    bench.archive(&repo);
    bench.archive(&forked);
    push_section(&mut out, "worktree-open", &bench.render("zeta", &config));
    let worktree = bench.project(forked.project).to_string_lossy().into_owned();
    let excluding = Config::from_parts(Vec::new(), vec![worktree]);
    push_section(
        &mut out,
        "worktree-excluded",
        &bench.render("zeta", &excluding),
    );

    // 7. A project of subagent sessions only.
    for (n, id) in [
        "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
        "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
    ]
    .into_iter()
    .enumerate()
    {
        let file = format!("{id}/subagents/agent-{n}.jsonl");
        let day = format!("2026-08-1{n}");
        bench.archive(&Session {
            file: &file,
            id,
            project: "subs",
            branch: Some("sub-branch"),
            day: &day,
            prompt: "the subagent prompt, about flanges",
            reply: "the subagent reply, about flanges",
        });
    }
    push_section(&mut out, "subagents-only", &bench.render("subs", &config));

    rooted(&out, bench.dir.path())
}

/// The captured text with the tempdir written back to [`testkit::FIXTURE_ROOT_TOKEN`].
fn rooted(text: &str, dir: &Path) -> String {
    let raw = dir.to_string_lossy().into_owned();
    // The escaped spelling first, so a doubled backslash leaves no stray one behind.
    let escaped = raw.replace('\\', "\\\\");
    let substituted = text
        .replace(&escaped, testkit::FIXTURE_ROOT_TOKEN)
        .replace(&raw, testkit::FIXTURE_ROOT_TOKEN);
    forward_slashes(&substituted)
}

/// Separators normalized to `/` inside every token-rooted path, and nowhere else.
fn forward_slashes(text: &str) -> String {
    let token = testkit::FIXTURE_ROOT_TOKEN;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(token) {
        out.push_str(&rest[..at + token.len()]);
        rest = &rest[at + token.len()..];

        let mut chars = rest.char_indices().peekable();
        let mut end = rest.len();
        while let Some((index, c)) = chars.next() {
            match c {
                '\\' => {
                    out.push('/');
                    // A doubled separator is one that was escaped for JSON.
                    if chars.peek().map(|(_, next)| *next) == Some('\\') {
                        chars.next();
                    }
                }
                c if c.is_whitespace() || c == '"' || c == ',' => {
                    end = index;
                    break;
                }
                c => out.push(c),
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(GOLDEN)
}

/// The first line that differs, with both sides, and how many lines each has.
fn diff(expected: &str, actual: &str) -> String {
    for (n, (want, got)) in expected.lines().zip(actual.lines()).enumerate() {
        if want != got {
            return format!("line {}:\n  golden: {want:?}\n  actual: {got:?}", n + 1);
        }
    }
    format!(
        "no line differs; the golden has {} lines and this build produced {}",
        expected.lines().count(),
        actual.lines().count()
    )
}

#[test]
fn the_briefs_are_what_the_golden_says() {
    let bench = bench();
    let actual = capture(&bench);
    let path = golden_path();

    if std::env::var_os(REGENERATE_ENV).is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        panic!(
            "regenerated {} - unset {REGENERATE_ENV} and run again to compare",
            path.display()
        );
    }

    let expected =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(
        expected == actual,
        "the briefs moved against {}\n{}",
        path.display(),
        diff(&expected, &actual)
    );
}

/// The text of one `== name` section, up to the next one.
fn section<'a>(doc: &'a str, name: &str) -> &'a str {
    let head = format!("\n== {name}\n");
    let start = doc
        .find(&head)
        .unwrap_or_else(|| panic!("the golden has no section {name}"))
        + head.len();
    let rest = &doc[start..];
    &rest[..rest.find("\n== ").unwrap_or(rest.len())]
}

/// A capture that lost one of the seven shapes would still match itself, so each is checked by a fixed substring.
#[test]
fn the_golden_holds_each_of_the_seven_shapes() {
    let doc = std::fs::read_to_string(golden_path()).unwrap();

    let others = section(&doc, "others-listed");
    assert!(
        others.contains("Other recent sessions, titled by how each began:\n2026-08-11 Middle work\n2026-08-10 Oldest work"),
        "{others}"
    );

    let later = section(&doc, "later-session");
    assert!(
        later.contains("A later session ended 2026-08-14, on branch branch-of-the-late-one.")
            && later.contains("the early prompt, about widgets"),
        "{later}"
    );

    let edits = section(&doc, "edited-files-more");
    assert!(
        edits.contains("It edited: ") && edits.contains(" and 3 more."),
        "{edits}"
    );

    let shallow = section(&doc, "deep-no-repository");
    assert!(
        !shallow.contains("It last asked:") && shallow.contains("68 sessions"),
        "{shallow}"
    );
    let side = section(&doc, "deep-side-checked-out");
    assert!(
        side.contains("on branch side-branch") && side.contains("about pistons"),
        "{side}"
    );

    let gone = section(&doc, "evicted-session");
    assert!(
        gone.contains("The last session in ")
            && gone.contains("It edited: src/lib.rs.")
            && gone.contains("Reach them with the recall_search")
            && !gone.contains("about bearings"),
        "{gone}"
    );

    let open = section(&doc, "worktree-open");
    let excluded = section(&doc, "worktree-excluded");
    assert!(
        open.contains("2 sessions and 4 turns") && open.contains("about flanges"),
        "{open}"
    );
    assert!(
        excluded.contains("1 session and 2 turns") && !excluded.contains("about flanges"),
        "{excluded}"
    );

    let subs = section(&doc, "subagents-only");
    assert!(
        subs.contains("2 sessions and 4 turns")
            && !subs.contains("The last session")
            && !subs.contains(NO_BRIEF),
        "{subs}"
    );
}
