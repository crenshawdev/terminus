//! The resume brief, against stores built by the ordinary ingest path.
//!
//! Every session here is a transcript this file wrote and `ingest::run`
//! archived, never rows a helper inserted: what the brief is about is which
//! `session_meta` row a real pass produced, and a test that wrote the row
//! itself would be asserting against its own opinion of the schema.
//!
//! Each session's `cwd` is a directory beneath a root the test owns and
//! **nothing creates**, so project identity degrades to that path rather than
//! resolving through `git rev-parse` for a directory that happens to exist -
//! the same reason `testkit::FIXTURE_ROOT_TOKEN` exists.

#![cfg(feature = "testkit")]

use std::path::{Path, PathBuf};

use terminus_core::config::Config;
use terminus_core::ingest;
use terminus_core::inject::{self, brief, Payload};

struct Bench {
    _dir: tempfile::TempDir,
    data_dir: PathBuf,
    work: PathBuf,
    root: PathBuf,
}

fn bench() -> Bench {
    let dir = tempfile::tempdir_in(terminus_core::testkit::temp_root()).unwrap();
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

/// One record carrying a single `text` block, which is what a person typing a
/// prompt and a model answering in prose both produce.
fn text_record(kind: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": kind,
        "message": {
            "role": kind,
            "content": [{"type": "text", "text": text}],
        },
    })
}

impl Bench {
    fn project(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Archive one session through the ordinary ingest path: the prompt, then
    /// the reply, both plain text blocks.
    fn archive(&self, session: &Session<'_>) {
        self.archive_records(
            session,
            &[
                text_record("user", session.prompt),
                text_record("assistant", session.reply),
            ],
        );
    }

    /// Archive one session whose records are spelled out rather than being the
    /// prompt-and-reply pair [`Bench::archive`] writes.
    ///
    /// The brief must not quote a `user` record the person did not write - a
    /// `tool_result` block, an `isMeta` caveat, a `<task-notification>`
    /// envelope - and none of the three is a `text` block with a `prompt` in
    /// it. Each entry carries only what is particular to its record; this fills
    /// in the envelope every record of one session shares.
    ///
    /// At most nine records: the timestamps are `T1{n}:00:00`, which is the
    /// same one-digit hour [`Bench::archive`] has always used to keep them
    /// ascending without a date library.
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

    /// One record of a session, as the line its transcript holds.
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

    /// Add one record to an archived session's transcript and ingest the tail.
    fn append(&self, session: &Session<'_>, n: usize, particular: &serde_json::Value) {
        use std::io::Write as _;
        let path = self.work.join(session.file);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(self.line(session, n, particular).as_bytes())
            .unwrap();
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{}: {other:?}", session.file),
        }
    }

    /// Archive one rooted fixture verbatim, under this bench's own root.
    ///
    /// The fixtures carry `{{ROOT}}` where a real transcript carries an
    /// absolute `cwd`, so the project key is one this test built.
    fn archive_fixture(&self, fixture: &str) {
        let path =
            terminus_core::testkit::copy_rooted_fixture_into(fixture, &self.work, &self.root);
        match ingest::run(&self.data_dir, &path).unwrap() {
            ingest::Outcome::Committed(_) => {}
            other => panic!("{fixture}: {other:?}"),
        }
    }

    /// A config with `[privacy] redact_recall` written either way, through the
    /// file the binary loads one from.
    ///
    /// There is no in-memory setter for it, deliberately, so a `terminus.toml`
    /// is the only thing that can turn it on. `on = false` spells the key out
    /// rather than omitting it, so the two configs differ in exactly one token
    /// of one file and in nothing else about how they were built.
    fn redacting(&self, on: bool) -> Config {
        let dir = self.work.join(format!("privacy-{on}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("terminus.toml"),
            format!("[privacy]\nredact_recall = {on}\n"),
        )
        .unwrap();
        Config::load_from(&dir).unwrap()
    }

    /// A config whose `[injection] brief_chars` is what the test says, loaded
    /// through the file the binary loads one from - the budget has to reach the
    /// query, and a struct built in memory would not prove that it does.
    fn config(&self, brief_chars: usize) -> Config {
        let dir = self.work.join(format!("config-{brief_chars}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("terminus.toml"),
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
        self.brief_as("0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55", project, config)
    }

    /// The same, under a `session_id` the caller names, so that what the brief
    /// remembered can be read back from that session's state file alone.
    fn brief_as(&self, session: &str, project: &str, config: &Config) -> Option<String> {
        let cwd = self.project(project);
        let payload = Payload {
            session_id: Some(session),
            transcript_path: Some("/home/user/.claude/projects/-p/s.jsonl"),
            cwd: cwd.to_str(),
            prompt: None,
            source: Some("startup"),
        };
        brief::session_start(&self.data_dir, config, &payload)
    }
}

impl Bench {
    /// What the brief a `SessionStart` in one project would carry read from
    /// the archive, at the default config.
    fn brief_reads(&self, project: &str) -> terminus_core::recall::Reads {
        let cwd = self.project(project);
        let payload = Payload {
            session_id: Some("0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55"),
            transcript_path: Some("/home/user/.claude/projects/-p/s.jsonl"),
            cwd: cwd.to_str(),
            prompt: None,
            source: Some("startup"),
        };
        brief::session_start_reads(&self.data_dir, &Config::default(), &payload)
            .expect("a brief renders")
    }

    /// The store's turn ids for one session file, in `turn_seq` order, and how
    /// many block rows that session has.
    fn turns_and_rows(&self, file: &str) -> (Vec<i64>, usize) {
        let conn =
            rusqlite::Connection::open(self.data_dir.join(terminus_core::store::DB_FILE_NAME))
                .unwrap();
        let key = self
            .work
            .join(file)
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let ids = conn
            .prepare("SELECT id FROM turns WHERE session_key = ?1 ORDER BY turn_seq")
            .unwrap()
            .query_map([&key], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        (ids, terminus_core::testkit::block_count(&conn, &key))
    }

    fn covered(&self, ids: &[i64]) -> usize {
        let conn =
            rusqlite::Connection::open(self.data_dir.join(terminus_core::store::DB_FILE_NAME))
                .unwrap();
        terminus_core::testkit::covered_blocks(&conn, ids)
    }
}

/// A session for the block-read tests, in `project-blocks`.
const BLOCKS: Session<'static> = Session {
    file: "blocks.jsonl",
    id: "55555555-5555-4555-8555-555555555555",
    project: "project-blocks",
    branch: None,
    day: "2026-08-14",
    prompt: "the last prompt",
    reply: "the last reply",
};

/// The brief's block reads: when the two quoted turns sit in one block of a
/// session of three or more, the brief fetches that one row and decompresses
/// it once.
#[test]
fn the_brief_reads_only_the_row_its_two_quotes_sit_in() {
    let bench = bench();
    let filler = "filler ".repeat(30_000);
    bench.archive_records(
        &BLOCKS,
        &[
            text_record("assistant", &filler),
            text_record("user", BLOCKS.prompt),
            text_record("assistant", BLOCKS.reply),
        ],
    );
    let (ids, rows) = bench.turns_and_rows(BLOCKS.file);
    assert!(
        rows >= 3,
        "the session must be three or more blocks: {rows}"
    );
    assert_eq!(bench.covered(&ids[1..]), 1, "the quotes share a block");

    let text = bench.brief(BLOCKS.project).expect("a brief renders");
    assert!(
        text.contains(BLOCKS.prompt) && text.contains(BLOCKS.reply),
        "{text}"
    );
    let reads = bench.brief_reads(BLOCKS.project);
    assert_eq!(reads.block_rows, 1, "{reads:?}");
    assert_eq!(reads.blocks_decompressed, 1, "{reads:?}");
}

/// The brief's block reads across blocks: a quoted reply spanning k blocks
/// costs the union of the rows the two quotes cover, each fetched and
/// decompressed once.
#[test]
fn the_brief_reads_the_union_of_the_rows_its_quotes_cover() {
    let bench = bench();
    let reply = format!("{} {}", BLOCKS.reply, "filler ".repeat(30_000));
    bench.archive_records(
        &BLOCKS,
        &[
            text_record("user", BLOCKS.prompt),
            text_record("assistant", &reply),
        ],
    );
    let (ids, rows) = bench.turns_and_rows(BLOCKS.file);
    assert!(
        rows >= 3,
        "the session must be three or more blocks: {rows}"
    );
    let union = bench.covered(&ids);
    assert!(
        union >= 3,
        "the reply must span three or more blocks: {union}"
    );

    let reads = bench.brief_reads(BLOCKS.project);
    assert_eq!(reads.block_rows, union, "{reads:?}");
    assert_eq!(reads.blocks_decompressed, union, "{reads:?}");
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

/// A session that changes branch ended on the last branch it named, whether the
/// change arrives in one pass or in a later one. A later record naming no branch
/// changes nothing.
#[test]
fn the_brief_names_the_branch_the_session_ended_on() {
    let bench = bench();
    let session = Session {
        file: "branches.jsonl",
        id: "77777777-7777-4777-8777-777777777777",
        project: "project-alpha",
        branch: None,
        day: "2026-08-14",
        prompt: "",
        reply: "",
    };
    let on = |branch: &str, text: &str| {
        let mut record = text_record("assistant", text);
        record["gitBranch"] = serde_json::Value::String(branch.to_owned());
        record
    };

    bench.archive_records(
        &session,
        &[on("first-branch", "started"), on("second-branch", "moved")],
    );
    let brief = bench.brief("project-alpha").expect("a brief");
    assert!(brief.contains("on branch second-branch."), "{brief}");

    bench.append(&session, 2, &on("third-branch", "moved again"));
    let brief = bench.brief("project-alpha").expect("a brief");
    assert!(brief.contains("on branch third-branch."), "{brief}");

    bench.append(&session, 3, &text_record("assistant", "no branch named"));
    let brief = bench.brief("project-alpha").expect("a brief");
    assert!(brief.contains("on branch third-branch."), "{brief}");
}

/// The brief names the session with the greatest `last_turn_at`, at day
/// resolution, with the branch it ended on and the last thing said in each
/// direction.
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

/// A subagent sidecar is not a session the brief may name, however recent it
/// is. Its `parent_session_key` is what excludes it, and that column comes from
/// the sidecar's path.
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
    let conn = rusqlite::Connection::open(bench.data_dir.join(terminus_core::store::DB_FILE_NAME))
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

// ---------------------------------------------------------------------------
// The quoted prompt is a turn the person typed

/// The prompt every case below has to end up quoting, and the reply beside it.
const TYPED: &str = "the prompt somebody actually typed, about sprockets";
const ANSWER: &str = "the reply, about sprockets";

/// The `user` record Claude Code writes when a tool returns: a `tool_result`
/// block in `message.content`, and the result again at the top level.
///
/// Both halves, because the classifier reads only the first, and a record
/// carrying only the top-level key would let a rule that read the wrong one
/// pass.
fn tool_result_record(text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_inj07",
                "content": text,
            }],
        },
        "toolUseResult": {"stdout": text},
    })
}

/// The assistant turn that asked for it.
fn tool_use_record() -> serde_json::Value {
    serde_json::json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_inj07",
                "name": "Glob",
                "input": {"pattern": "*.csv"},
            }],
        },
    })
}

impl Bench {
    /// `is_typed` of the last `user` turn in the store, by `turn_seq`.
    ///
    /// The falsifying half of the three cases below: each seeds ONE session,
    /// and each is only about anything if that session's last `user` record
    /// really did archive as one nobody typed. Without this a test would pass
    /// just as happily against a transcript whose last record was the prompt.
    fn last_user_is_typed(&self) -> Option<i64> {
        let conn =
            rusqlite::Connection::open(self.data_dir.join(terminus_core::store::DB_FILE_NAME))
                .unwrap();
        conn.query_row(
            "SELECT is_typed FROM turns WHERE record_type = 'user' \
             ORDER BY turn_seq DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }
}

/// A session whose last `user` record is a tool result: the brief quotes the
/// prompt before it, and carries none of the result.
#[test]
fn a_session_ending_in_a_tool_result_quotes_the_last_typed_prompt() {
    const LISTING: &str = "flange-inventory.csv freight-manifest.csv";

    let bench = bench();
    bench.archive_records(
        &Session {
            file: "tool-result.jsonl",
            id: "55555555-5555-4555-8555-555555555555",
            project: "project-tools",
            ..LATE
        },
        &[
            text_record("user", TYPED),
            tool_use_record(),
            tool_result_record(LISTING),
            text_record("assistant", ANSWER),
        ],
    );
    assert_eq!(
        bench.last_user_is_typed(),
        Some(0),
        "the last user record archived as one the person typed"
    );

    let brief = bench.brief("project-tools").expect("a brief");
    assert!(
        brief.contains(TYPED) && brief.contains(ANSWER),
        "the brief lost a side of the exchange: {brief}"
    );
    for token in ["flange-inventory", "freight-manifest", "toolUseResult"] {
        assert!(
            !brief.contains(token),
            "the brief carries the tool result's {token:?}: {brief}"
        );
    }
}

/// The other two untyped shapes: a `<task-notification>` envelope and an
/// `isMeta: true` caveat are `user` records nobody typed either, and 12.3% of
/// measured sessions end on one of them.
///
/// A bench each, because [`Bench::last_user_is_typed`] reads the one session in
/// the store and two sessions in one store would order across sessions.
#[test]
fn a_session_ending_in_a_harness_record_quotes_the_prompt_before_it() {
    const ENVELOPE: &str = "<task-notification>the flange audit finished</task-notification>";
    const CAVEAT: &str = "Caveat: the messages below were generated while running /flanges";

    for (shape, last) in [
        ("envelope", text_record("user", ENVELOPE)),
        ("isMeta", {
            let mut record = text_record("user", CAVEAT);
            record["isMeta"] = serde_json::Value::Bool(true);
            record
        }),
    ] {
        let bench = bench();
        bench.archive_records(
            &Session {
                file: "harness.jsonl",
                id: "66666666-6666-4666-8666-666666666666",
                project: "project-harness",
                ..LATE
            },
            &[
                text_record("user", TYPED),
                text_record("assistant", ANSWER),
                last,
            ],
        );
        assert_eq!(
            bench.last_user_is_typed(),
            Some(0),
            "{shape}: the last user record archived as one the person typed"
        );

        let brief = bench.brief("project-harness").expect("a brief");
        assert!(
            brief.contains(TYPED) && brief.contains(ANSWER),
            "{shape}: the brief lost a side of the exchange: {brief}"
        );
        for token in ["task-notification", "flange audit", "Caveat"] {
            assert!(
                !brief.contains(token),
                "{shape}: the brief carries the harness record's {token:?}: {brief}"
            );
        }
    }
}

/// A session with no typed `user` record at all: no "It last asked" line, and
/// every other block still rendered.
///
/// Fixture-only, and deliberately so: 0 of 700 sampled real transcripts have
/// zero person-authored `user` records. `session-errors-a.jsonl` is
/// three `Bash` calls and their three `tool_result` records and nothing else,
/// which is exactly that session.
#[test]
fn a_session_with_no_typed_prompt_renders_no_asked_line() {
    let bench = bench();
    bench.archive_fixture("session-errors-a.jsonl");
    assert_eq!(
        bench.last_user_is_typed(),
        Some(0),
        "the fixture archived a typed user record after all"
    );

    let brief = bench.brief("project-beta").expect("a brief");
    assert!(
        !brief.contains("It last asked"),
        "the brief quotes a prompt in a session that has none: {brief}"
    );
    // The rest of the brief is untouched: this is a missing quotation, not a
    // missing block.
    for kept in ["It last answered", "2026-08-12", "phase-3", "recall_search"] {
        assert!(
            brief.contains(kept),
            "the brief dropped {kept:?} along with the prompt line: {brief}"
        );
    }
}

/// The injection path spawns no process and reads no `observations`, asserted
/// against the source rather than against behaviour.
///
/// Neither is observable from the outside: a `git` subprocess that costs 10-30
/// ms still returns the right answer, and a query against `observations`
/// succeeds on every store, because every store has the table. The comments
/// are stripped first because this module's own doc explains at length why
/// neither appears - a raw grep would match the explanation and never the
/// thing.
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
// The budget

/// A turn long enough that the brief cannot carry both of them whole.
fn long_turn(subject: &str) -> String {
    format!("{subject} ").repeat(400)
}

/// The header line, the opening tag on the next line, and the closing tag once,
/// as the last characters.
fn assert_framed(brief: &str) {
    let mut lines = brief.lines();
    let header = lines.next().unwrap_or_default();
    assert!(
        header.starts_with("Terminus resume brief - ") && header.ends_with(inject::QUOTED_HISTORY),
        "the first line is not the header: {brief}"
    );
    assert_eq!(lines.next(), Some(inject::OPEN_TAG), "{brief}");
    assert_eq!(brief.matches(inject::OPEN_TAG).count(), 1, "{brief}");
    assert_eq!(brief.matches(inject::CLOSE_TAG).count(), 1, "{brief}");
    assert!(
        brief.ends_with(&format!("\n{}", inject::CLOSE_TAG)),
        "the closing tag is not last, on its own line: {brief}"
    );
}

/// What sits between the two tags. Panics if they are not both there.
fn between_tags(brief: &str) -> &str {
    let open = brief.find(inject::OPEN_TAG).expect("no opening tag") + inject::OPEN_TAG.len();
    let close = brief.rfind(inject::CLOSE_TAG).expect("no closing tag");
    &brief[open..close]
}

/// What the frame of a rendered brief costs: everything that is not the body.
///
/// The body sits between `<open>\n` and `\n<close>`, so those two line breaks
/// are the frame's, not the body's.
fn frame_cost(brief: &str) -> usize {
    brief.chars().count() - between_tags(brief).chars().count() + 2
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

    // The frame, the body's fixed text and 200 characters for the two quoted
    // turns. The fixed text is measured off the uncut brief because the head
    // line and the pointer name the project path, which sits under the temp
    // directory. 200 gives each turn a cut that ends in the mark, and is well
    // short of the two windowed quotations whole.
    let uncut = bench.brief("project-alpha").expect("a brief");
    let quotations = quoted_after(&uncut, "It last asked: ").chars().count()
        + quoted_after(&uncut, "It last answered: ").chars().count();
    let fixed = between_tags(&uncut).chars().count() - 2 - quotations;
    let budget = frame_cost(&uncut) + fixed + 200;
    let brief = bench
        .brief_with("project-alpha", &bench.config(budget))
        .expect("a brief");

    // The falsifying half: an uncut brief is longer than the budget, so what
    // follows is a cut and not a brief that happened to fit.
    assert!(
        uncut.chars().count() > budget,
        "the unbudgeted brief is only {} characters, so this test proves \
         nothing about cutting: {uncut}",
        uncut.chars().count()
    );
    assert_framed(&brief);
    assert!(
        between_tags(&brief).contains("\nIt last asked: "),
        "the budget dropped the quoted prompt rather than cutting it: {brief}"
    );

    assert!(
        brief.chars().count() <= budget,
        "the brief is {} characters against a budget of {budget}: {brief}",
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
        brief.contains(terminus_core::recall::excerpt::ELISION),
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
    // Cut inside the frame: the elision ends the body, and the closing tag
    // still ends the brief.
    assert!(
        brief.ends_with(&format!(
            "{}\n{}",
            terminus_core::recall::excerpt::ELISION,
            inject::CLOSE_TAG
        )),
        "the brief was cut at the ceiling without saying so, or cut the frame"
    );
    assert_framed(&brief);
}

/// At every budget from nothing to the whole brief, the arm either
/// renders nothing and remembers nothing, or renders a brief that fits and
/// ends with the closing tag.
///
/// One `session_id` per budget, so "remembers nothing" is read off that
/// session's state file alone: a turn recorded as briefed and never shown would
/// be refused to every prompt of the session.
#[test]
fn every_brief_budget_gives_nothing_or_a_framed_brief_that_fits() {
    let bench = bench();
    let prompt = long_turn("a very long prompt about sprockets");
    let reply = long_turn("a very long reply about sprockets");
    bench.archive(&Session {
        prompt: &prompt,
        reply: &reply,
        ..LATE
    });

    let length = bench
        .brief("project-alpha")
        .expect("a brief")
        .chars()
        .count();
    let (mut silent, mut spoke) = (0, 0);
    for budget in 0..=length {
        let session = format!("55555555-0000-4000-8000-{budget:012}");
        match bench.brief_as(&session, "project-alpha", &bench.config(budget)) {
            None => {
                silent += 1;
                let state = inject::state::State::load(&bench.data_dir, Some(&session));
                assert!(
                    state.brief.is_empty(),
                    "budget {budget} rendered nothing and remembered {:?} as briefed",
                    state.brief
                );
            }
            Some(brief) => {
                spoke += 1;
                assert!(
                    brief.chars().count() <= budget,
                    "{} characters against a budget of {budget}: {brief}",
                    brief.chars().count()
                );
                assert_framed(&brief);
            }
        }
    }
    assert!(silent > 0, "no budget was too small for the frame");
    assert!(spoke > 0, "no budget was large enough to render");
}

/// The text after `label` on the line between the tags that starts with it.
fn quoted_after<'a>(brief: &'a str, label: &str) -> &'a str {
    between_tags(brief)
        .lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_else(|| panic!("no {label:?} line between the tags:\n{brief}"))
}

/// #46: a long project path and a small budget leave the quoted turns no room,
/// and the brief has to say it cut them rather than drop them without a word.
///
/// The project is never created, so its key is the long path itself, as in
/// `no_brief_exceeds_the_ceiling_whatever_the_config_says`. The budget that
/// leaves the quotations no room is measured off the rendered uncut brief, so it
/// follows whatever the temp path makes the project label.
#[test]
fn a_budget_that_leaves_a_quoted_turn_no_room_marks_it_and_does_not_record_it() {
    let bench = bench();
    let long = format!("project-with-a-long-name-{}", "0123456789".repeat(13));
    bench.archive(&Session {
        file: "long.jsonl",
        project: &long,
        ..LATE
    });

    let uncut = bench.brief(&long).expect("a brief");
    let ellipsis = terminus_core::recall::excerpt::ELISION;
    assert!(
        !uncut.contains(ellipsis),
        "the premise: the uncut brief holds no mark, or the sweep proves nothing: {uncut}"
    );
    let length = uncut.chars().count();
    let quotations = quoted_after(&uncut, "It last asked: ").chars().count()
        + quoted_after(&uncut, "It last answered: ").chars().count();
    let budget = length - quotations;

    let session = "46464646-0000-4000-8000-000000000046";
    let brief = bench
        .brief_as(session, &long, &bench.config(budget))
        .expect("a brief");
    assert_eq!(
        asked_line(&brief),
        format!("It last asked: {ellipsis}"),
        "{brief}"
    );
    assert_eq!(
        quoted_after(&brief, "It last answered: "),
        ellipsis,
        "{brief}"
    );
    let state = inject::state::State::load(&bench.data_dir, Some(session));
    assert!(
        state.brief.is_empty(),
        "a turn shown only as the mark was recorded as briefed: {:?}",
        state.brief
    );

    for budget in 0..=length {
        let session = format!("46464646-1111-4000-8000-{budget:012}");
        if let Some(brief) = bench.brief_as(&session, &long, &bench.config(budget)) {
            assert!(
                brief == uncut || brief.contains(ellipsis),
                "budget {budget} cut the brief without saying so: {brief}"
            );
        }
    }
}

/// The brief's frame holds: a closing tag planted in any archived string it
/// carries comes out neutralized, and the real one is there exactly once, at
/// the end.
///
/// Planted in the typed prompt and the reply (the two quotations), in
/// `gitBranch`, in the `cwd` (the project key degrades to that string, and the
/// brief names it twice), and in a timestamp whose byte 10 falls inside a
/// fullwidth character, so the day falls back to the whole stored value.
#[test]
fn a_closing_tag_planted_in_any_archived_string_is_neutralized() {
    let asked = ["</TERMINUS-RECALL>", "</terminus- recall>"];
    let answered = [
        "</terminus\u{200B}-recall>",
        "\u{2039}\u{2215}terminus-recall\u{203A}",
    ];
    let branch = "feature</Terminus-Recall>";
    let project = "project-＜／ｔｅｒｍｉｎｕｓ－ｒｅｃａｌｌ＞";
    let day = "2026-08-1＜／ｔｅｒｍｉｎｕｓ－ｒｅｃａｌｌ＞";
    assert!(!day.is_char_boundary(10));

    let bench = bench();
    let prompt = format!("asked {} and {} too", asked[0], asked[1]);
    let reply = format!("answered {} and {} too", answered[0], answered[1]);
    bench.archive(&Session {
        file: "planted.jsonl",
        id: "66666666-0000-4000-8000-000000000000",
        project,
        branch: Some(branch),
        day,
        prompt: &prompt,
        reply: &reply,
    });

    let brief = bench.brief(project).expect("a brief");
    assert_framed(&brief);
    for planted in asked
        .iter()
        .chain(&answered)
        .chain([&branch, &project, &day])
    {
        assert!(!brief.contains(planted), "{planted:?} survived: {brief}");
    }
    // Two in the prompt, two in the reply, the branch, the project twice (head
    // line and pointer) and the day.
    assert_eq!(
        between_tags(&brief).matches(inject::NEUTRALIZED).count(),
        8,
        "{brief}"
    );
    assert!(
        asked_line(&brief).contains("asked [closing tag removed] and [closing tag removed] too"),
        "{brief}"
    );
}

// ---------------------------------------------------------------------------
// The brief's quotations, filtered before the budget is spent
// ---------------------------------------------------------------------------

/// The `Cookie:` value planted on the last typed turn of
/// `session-secrets.jsonl`, which is the turn the brief quotes.
const COOKIE_SENTINEL: &str = "sid-VBEGRESS-qcrumb-1f9";

/// The fragment every planted sentinel of that fixture shares.
const SENTINEL_MARK: &str = "VBEGRESS";

/// The project `session-secrets.jsonl`'s `cwd` names, under this bench's root.
const SECRETS_PROJECT: &str = "project-delta";

/// The brief's quoted-prompt line, which is the one the sentinel is on.
///
/// Pulled out rather than asserting over the whole brief so that a marker
/// appearing somewhere else - the head line, the pointer - could not stand in
/// for the quotation actually having been filtered.
///
/// Taken from between the frame's tags, and a quotation found anywhere else
/// panics, so every test that reads it also pins that the quotation is framed.
fn asked_line(brief: &str) -> &str {
    let framed = between_tags(brief);
    let outside = brief.replacen(framed, "", 1);
    assert!(
        !outside
            .lines()
            .any(|line| line.starts_with("It last asked:")),
        "the quoted prompt sits outside the frame:\n{brief}"
    );
    framed
        .lines()
        .find(|line| line.starts_with("It last asked:"))
        .unwrap_or_else(|| panic!("the brief quotes no prompt between the tags:\n{brief}"))
}

/// The default, stated as an assertion: with the knob absent the brief quotes
/// the turn exactly as the archive holds it.
#[test]
fn an_unfiltered_brief_quotes_the_planted_credential_whole() {
    let bench = bench();
    bench.archive_fixture("session-secrets.jsonl");

    let brief = bench
        .brief_with(SECRETS_PROJECT, &bench.redacting(false))
        .expect("a brief");

    assert!(
        asked_line(&brief).contains(COOKIE_SENTINEL),
        "the default must not filter the quotation:\n{brief}"
    );
}

/// `redact_recall` over the `SessionStart` brief: hook stdout is the model's
/// first context of a session, so the knob filters what it quotes.
///
/// The filter runs before the budget is measured, never after `clip`: a marker
/// cut in half by the budget would leave a nameless partial value no shape rule
/// can catch, and the brief would be the one output where a long turn still
/// leaks.
#[test]
fn the_knob_takes_the_planted_credential_out_of_the_briefs_quotation() {
    let bench = bench();
    bench.archive_fixture("session-secrets.jsonl");

    let raw = bench
        .brief_with(SECRETS_PROJECT, &bench.redacting(false))
        .expect("a brief");
    let filtered = bench
        .brief_with(SECRETS_PROJECT, &bench.redacting(true))
        .expect("a brief");

    // The premise: the quoted turn really does carry the credential, so what
    // follows is about the filter and not about a brief that quoted elsewhere.
    assert!(asked_line(&raw).contains(COOKIE_SENTINEL), "{raw}");

    assert!(
        asked_line(&filtered).contains(terminus_core::config::REDACTED),
        "a filtered quotation names what went:\n{filtered}"
    );
    for absent in [COOKIE_SENTINEL, SENTINEL_MARK, "qcrumb"] {
        assert!(
            !filtered.contains(absent),
            "{absent:?} survived into the brief:\n{filtered}"
        );
    }

    // The blocks this file builds itself are not archived text and are not
    // filtered: the brief still names the session it is resuming and still
    // points at the index.
    for present in ["The last session in", "phase-2-egress", "recall_search"] {
        assert!(
            filtered.contains(present),
            "the filter took a block it does not own, {present:?}:\n{filtered}"
        );
    }
}

/// The brief's byte identity, restated with the knob on: two renders against an
/// unchanged store are the same brief.
///
/// The filter is a pure function of the projection, so it cannot be what makes
/// a brief vary - but that is the claim, and this is what falsifies it.
#[test]
fn two_filtered_briefs_against_an_unchanged_store_are_byte_identical() {
    let bench = bench();
    bench.archive_fixture("session-secrets.jsonl");
    let config = bench.redacting(true);

    let first = bench.brief_with(SECRETS_PROJECT, &config).expect("a brief");
    let second = bench.brief_with(SECRETS_PROJECT, &config).expect("a brief");

    assert_eq!(first, second);
    assert!(first.contains(terminus_core::config::REDACTED), "{first}");
    assert_framed(&first);
}

/// The control: a project with nothing credential-shaped in it renders the same
/// brief in both settings.
#[test]
fn the_knob_leaves_an_ordinary_brief_exactly_as_it_was() {
    let bench = bench();
    bench.archive(&LATE);

    assert_eq!(
        bench.brief_with("project-alpha", &bench.redacting(true)),
        bench.brief_with("project-alpha", &bench.redacting(false)),
    );
}

/// The earlier session of `project-zeta`, run from the repository itself.
const REPO: Session<'static> = Session {
    file: "zeta-repo.jsonl",
    id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    project: "project-zeta",
    branch: Some("branch-of-the-repo-one"),
    day: "2026-08-10",
    prompt: "the repo prompt, about gaskets",
    reply: "the repo reply, about gaskets",
};

/// The later session of `project-zeta`, run from one of its worktrees. Ingest
/// folds it into the repository's `project` and keeps the worktree path as its
/// `project_pre_worktree`.
const FORKED: Session<'static> = Session {
    file: "zeta-forked.jsonl",
    id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    project: "project-zeta/.claude/worktrees/wt-a",
    branch: Some("branch-of-the-worktree-one"),
    day: "2026-08-14",
    prompt: "the worktree prompt, about flanges",
    reply: "the worktree reply, about flanges",
};

/// A session excluded under its worktree key is neither the one the brief
/// quotes nor one it counts.
///
/// `scope::resolve` refuses only a scope whose own project is excluded, so a
/// worktree session folded into a visible repository is still in the brief's
/// scope, and only the `project_pre_worktree` clause keeps it out of the last
/// session and out of both counts. Every other exclusion test reads search or
/// the session listing, never the brief.
#[test]
fn a_session_excluded_under_its_worktree_key_is_neither_quoted_nor_counted() {
    let bench = bench();
    bench.archive(&REPO);
    bench.archive(&FORKED);

    let repo = bench.project(REPO.project).to_string_lossy().into_owned();
    let worktree = bench.project(FORKED.project).to_string_lossy().into_owned();

    // The premise: ingest folded the worktree session into the repository.
    let conn = rusqlite::Connection::open(bench.data_dir.join(terminus_core::store::DB_FILE_NAME))
        .unwrap();
    let (project, pre): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT project, project_pre_worktree FROM session_meta WHERE session_id = ?1",
            [FORKED.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        project.as_deref(),
        Some(repo.as_str()),
        "it folded to the repo"
    );
    assert_eq!(pre.as_deref(), Some(worktree.as_str()));

    // The control: with nothing excluded the later, worktree session is the
    // one quoted, and both sessions are counted.
    let open = bench.brief(REPO.project).expect("a brief");
    for present in [
        FORKED.day,
        FORKED.branch.unwrap(),
        FORKED.prompt,
        FORKED.reply,
        "2 sessions and 4 turns",
    ] {
        assert!(
            open.contains(present),
            "the brief omits {present:?}: {open}"
        );
    }

    let config = Config::from_parts(Vec::new(), vec![worktree]);
    let brief = bench.brief_with(REPO.project, &config).expect("a brief");
    for present in [
        REPO.day,
        REPO.branch.unwrap(),
        REPO.prompt,
        REPO.reply,
        "1 session and 2 turns",
    ] {
        assert!(
            brief.contains(present),
            "the brief omits {present:?}: {brief}"
        );
    }
    for absent in [FORKED.day, FORKED.branch.unwrap(), "flanges"] {
        assert!(
            !brief.contains(absent),
            "the brief carries the excluded session's {absent:?}: {brief}"
        );
    }
}
