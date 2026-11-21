//! The brief of this build against the brief of an older build, on a copy of a
//! real archive.
//!
//! Skipped unless `TERMINUS_DIFF_COPY_DIR` is set, so CI never runs it. It is run
//! by hand, once, from a script that takes the copy under guard: the copy is made
//! with SQLite's own backup from a read-only source, lives in a scratch directory
//! outside the repository, and goes when the run ends.
//!
//! The archive is private, so this file prints three counts and nothing else out
//! of it: projects compared, equal and differing. No brief text, project name,
//! session key or path from the store reaches the output, a panic message or the
//! repository. On a difference it prints, for each differing render, its ordinal,
//! the line number of the brief's first differing line and that line's two byte
//! lengths, writes both renders under numbered names into the render directory,
//! and prints that directory's path once. Nobody opens it but the owner.
//!
//! Three environment variables drive it:
//!
//! - `TERMINUS_DIFF_COPY_DIR`: the copy's data directory, the one holding
//!   `terminus.db`. Its parent is the scratch directory, and everything the run
//!   makes besides the renders goes there.
//! - `TERMINUS_DIFF_OLD_BIN`: the `terminus` binary built from the older code,
//!   with the `testkit` feature so it honours the deadline variable.
//! - `TERMINUS_DIFF_RENDER_DIR`: an existing, empty directory outside the repository
//!   and outside the scratch directory, for the renders of a differing project.
//!
//! The run fills the copy's summary with this build's `ingest`, takes the ingest
//! lock on the copy so no hook's detached ingest can touch it, then renders every
//! `(project, project_pre_worktree)` pair of `project_summary` with both binaries
//! and compares the bytes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use terminus_core::ingest::{self, Attempt};
use terminus_core::store::DB_FILE_NAME;
use terminus_core::summary::META_SUMMARY_FILLED;
use terminus_core::testkit;

const COPY_DIR: &str = "TERMINUS_DIFF_COPY_DIR";
const OLD_BIN: &str = "TERMINUS_DIFF_OLD_BIN";
const RENDER_DIR: &str = "TERMINUS_DIFF_RENDER_DIR";

/// One fixed session for every render, so both binaries are asked the same thing.
const SESSION_ID: &str = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";

/// A variable the run needs, as a path. Fails naming only the variable.
fn required(name: &str) -> PathBuf {
    match std::env::var_os(name) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => panic!("{name} is not set"),
    }
}

/// `path`, resolved, and named in a failure by its variable alone.
fn resolved(name: &str, path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| panic!("{name} does not name an existing path"))
}

/// This build's `terminus hook SessionStart` or the older build's, the payload on
/// stdin then EOF, and whatever it wrote to stdout. Its stderr is dropped, never
/// echoed.
fn render(bin: &Path, copy: &Path, claude: &Path, cwd: Option<&str>) -> Vec<u8> {
    let mut payload = serde_json::json!({
        "session_id": SESSION_ID,
        "hook_event_name": "SessionStart",
        "source": "startup",
    });
    if let Some(cwd) = cwd {
        payload["cwd"] = Value::String(cwd.to_owned());
    }
    let mut child = Command::new(bin)
        .args(["hook", "SessionStart"])
        .current_dir(copy.parent().expect("the copy has a scratch parent"))
        .env("TERMINUS_DATA_DIR", copy)
        .env("CLAUDE_CONFIG_DIR", claude)
        .env(
            testkit::INJECT_DEADLINE_ENV,
            testkit::CONTENT_INJECT_DEADLINE_MS,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn a hook");
    // A hook that exits before reading its stdin shows as a difference below.
    let _ = child
        .stdin
        .take()
        .expect("the hook's stdin")
        .write_all(format!("{payload}\n").as_bytes());
    child.wait_with_output().expect("wait for a hook").stdout
}

/// The brief inside a hook's document, or the raw bytes when it is not one.
fn brief(stdout: &[u8]) -> Vec<u8> {
    serde_json::from_slice::<Value>(stdout)
        .ok()
        .and_then(|doc| {
            doc["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .map(|text| text.as_bytes().to_vec())
        })
        .unwrap_or_else(|| stdout.to_vec())
}

/// Whether a hook's stdout is a document carrying a non-empty brief.
fn has_brief(stdout: &[u8]) -> bool {
    serde_json::from_slice::<Value>(stdout)
        .ok()
        .and_then(|doc| {
            doc["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .map(|text| !text.is_empty())
        })
        .unwrap_or(false)
}

/// The first line the two texts disagree on: its 1-based number and both lengths.
fn first_difference(old: &[u8], new: &[u8]) -> (usize, usize, usize) {
    let mut old_lines = old.split(|b| *b == b'\n');
    let mut new_lines = new.split(|b| *b == b'\n');
    for number in 1.. {
        match (old_lines.next(), new_lines.next()) {
            (Some(a), Some(b)) if a == b => {}
            (a, b) => return (number, a.map_or(0, <[u8]>::len), b.map_or(0, <[u8]>::len)),
        }
    }
    unreachable!()
}

#[test]
fn the_brief_matches_the_older_build_on_a_copy_of_the_real_archive() {
    let Some(copy) = std::env::var_os(COPY_DIR).filter(|v| !v.is_empty()) else {
        println!("brief_differential skipped: {COPY_DIR} is not set");
        return;
    };
    let copy = resolved(COPY_DIR, Path::new(&copy));
    let old_bin = required(OLD_BIN);
    let render_dir = resolved(RENDER_DIR, &required(RENDER_DIR));
    let new_bin = Path::new(env!("CARGO_BIN_EXE_terminus"));

    // The repository is two levels above this crate's manifest.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the repository root")
        .canonicalize()
        .expect("the repository root exists");
    let scratch = copy.parent().expect("the copy has a scratch parent");
    assert!(
        !copy.starts_with(&repo),
        "{COPY_DIR} is inside the repository"
    );
    assert!(
        !render_dir.starts_with(&repo),
        "{RENDER_DIR} is inside the repository"
    );
    assert!(
        !render_dir.starts_with(scratch),
        "{RENDER_DIR} is inside the scratch directory"
    );
    assert!(render_dir.is_dir(), "{RENDER_DIR} is not a directory");
    assert!(
        copy.join(DB_FILE_NAME).is_file(),
        "{COPY_DIR} holds no store"
    );

    let config = scratch.join("config");
    let claude = scratch.join("claude");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::create_dir_all(claude.join("projects")).unwrap();

    // The fill: a pass over no transcripts, which fills the summary and writes
    // its key.
    let filled = Command::new(new_bin)
        .arg("ingest")
        .current_dir(scratch)
        .env("TERMINUS_DATA_DIR", &copy)
        .env("TERMINUS_CONFIG_DIR", &config)
        .env("CLAUDE_CONFIG_DIR", &claude)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn the fill");
    assert!(filled.success(), "the fill exited {:?}", filled.code());

    let pairs: Vec<(Option<String>, Option<String>)> = {
        let conn = Connection::open_with_flags(
            copy.join(DB_FILE_NAME),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("open the copy read-only");
        let key: i64 = conn
            .query_row(
                "SELECT count(*) FROM meta WHERE key = ?1",
                [META_SUMMARY_FILLED],
                |r| r.get(0),
            )
            .expect("read the fill key");
        assert_eq!(key, 1, "the copy's summary is not marked filled");
        let mut read = conn
            .prepare("SELECT project, project_pre_worktree FROM project_summary ORDER BY pair_key")
            .expect("prepare the pair read");
        let rows = read
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("read the pairs")
            .collect::<Result<_, _>>()
            .expect("read a pair");
        rows
    };

    // Held until every render is done: no hook's detached ingest may touch the copy.
    let _lock = match ingest::lock::try_acquire(&copy).unwrap_or_else(|_| panic!("take the lock")) {
        Attempt::Acquired(lock) => lock,
        Attempt::Held => panic!("the copy's ingest lock is held elsewhere"),
    };

    let (mut compared, mut equal, mut differing) = (0usize, 0usize, 0usize);
    let mut differing_renders = 0usize;
    let mut carried_a_brief = false;
    let mut reports = Vec::new();
    for (project, pre_worktree) in &pairs {
        let mut cwds: Vec<Option<&str>> = vec![project.as_deref().or(pre_worktree.as_deref())];
        if let Some(key) = pre_worktree
            .as_deref()
            .filter(|k| Some(*k) != project.as_deref())
        {
            cwds.push(Some(key));
        }

        let mut pair_equal = true;
        for cwd in cwds {
            let old = render(&old_bin, &copy, &claude, cwd);
            let new = render(new_bin, &copy, &claude, cwd);
            carried_a_brief |= has_brief(&new);
            if old == new {
                continue;
            }
            pair_equal = false;
            differing_renders += 1;
            let (mut a, mut b) = (brief(&old), brief(&new));
            if a == b {
                (a, b) = (old.clone(), new.clone());
            }
            let (line, old_len, new_len) = first_difference(&a, &b);
            reports.push(format!(
                "render {differing_renders}: first differing line {line}, {old_len} bytes old, {new_len} bytes new"
            ));
            std::fs::write(
                render_dir.join(format!("{differing_renders}-old.txt")),
                &old,
            )
            .expect("write a render");
            std::fs::write(
                render_dir.join(format!("{differing_renders}-new.txt")),
                &new,
            )
            .expect("write a render");
        }
        compared += 1;
        if pair_equal {
            equal += 1;
        } else {
            differing += 1;
        }
    }

    println!("projects compared {compared}, equal {equal}, differing {differing}");
    for line in &reports {
        println!("{line}");
    }
    if differing > 0 {
        println!("the renders are in {}", render_dir.display());
    }

    assert_eq!(compared, pairs.len(), "a pair was not compared");
    assert!(carried_a_brief, "no render of this build carried a brief");
    assert_eq!(differing, 0, "the two builds rendered differently");
}
