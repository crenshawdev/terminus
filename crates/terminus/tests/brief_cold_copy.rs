//! One cold session start for every project of a guarded copy of a real archive,
//! against the real 50 ms deadline.
//!
//! Skipped unless `TERMINUS_COLD_COPY_DIR` is set, so CI never runs it. It names
//! the copy's data directory, the one holding `terminus.db`, and is run by hand,
//! once, from a script that takes the copy under guard: SQLite's own backup from
//! a read-only source, into a mode 0700 scratch directory under `/var/tmp`,
//! removed when the run ends. The harness refuses any copy that is not there.
//!
//! Cold means a cold page cache for the copy's `terminus.db` and its `-wal`, and
//! nothing else: only those two files are evicted, and the hook binary and the
//! small files (the config, the state file and the `.git` files) are warm. Linux
//! only. Each file is evicted with `posix_fadvise` after an fsync, and shown by
//! `mincore` to have no page resident before its render, or the harness refuses
//! to report a time.
//!
//! The archive is private, so the only output is one line of numbers: renders,
//! briefs delivered, abandoned, slowest and median milliseconds, and the seconds
//! the fill took. Strings read from the copy, the pair paths and the briefs, live
//! in memory and on a hook's stdin and go nowhere else. A hook's stdout and stderr
//! are captured and only tested. No failure message carries anything but the
//! variable, the rule broken, or numbers.
//!
//! The run fills the copy's summary with this build's `ingest` under a scratch
//! config that turns snapshots off, takes the ingest lock so no hook's detached
//! ingest can warm or race the read, then renders every
//! `(project, project_pre_worktree)` pair of `project_summary` once. Every hook is
//! handed the copy as `TERMINUS_DATA_DIR`, which outranks the config directory's
//! pointer, and `TERMINUS_CONFIG_DIR` must be unset: the hook reads the real
//! config directory, as the installed hook does, and the harness never opens it.

const COPY_DIR: &str = "TERMINUS_COLD_COPY_DIR";

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use rusqlite::{Connection, OpenFlags};
    use serde_json::Value;
    use terminus_core::config::{CONFIG_DIR_ENV, CONFIG_FILE_NAME};
    use terminus_core::ingest::{self, Attempt};
    use terminus_core::store::snapshot::DIR_NAME as SNAPSHOT_DIR;
    use terminus_core::store::DB_FILE_NAME;
    use terminus_core::summary::META_SUMMARY_FILLED;
    use terminus_core::testkit;

    use super::COPY_DIR;

    /// One fixed session for every render.
    const SESSION_ID: &str = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";

    /// The three indexes the brief's reads depend on.
    const INDEXES: [&str; 3] = [
        "idx_session_summary_project_last",
        "idx_session_summary_project_branch_last",
        "idx_session_summary_project_last_titled",
    ];

    /// What a hook prints when it gives up on its injection.
    const ABANDONED: &[u8] = b"abandoned its injection";

    /// The store a hook may be handed: the copy, resolved, inside the scratch
    /// directory. Only [`check_hook_store`] makes one, and [`render`] takes nothing else.
    struct HookStore(PathBuf);

    /// The copy refused unless it is the store this run may render against.
    /// The messages name the rule and never a path.
    fn check_hook_store(copy: &Path, scratch: &Path) -> HookStore {
        assert!(
            !copy.as_os_str().is_empty() && copy.is_absolute(),
            "the store a hook would open is empty or not absolute"
        );
        let resolved = copy
            .canonicalize()
            .unwrap_or_else(|_| panic!("the store a hook would open does not resolve"));
        assert!(
            resolved == copy && resolved.parent() == Some(scratch) && resolved.starts_with(scratch),
            "the store a hook would open is not the copy inside the scratch directory"
        );
        HookStore(resolved)
    }

    /// This build's `terminus hook SessionStart` for `cwd`, timed from spawn to
    /// exit. Returns the elapsed time, whether it delivered a brief and whether it
    /// abandoned. Neither stream is echoed.
    fn render(
        store: &HookStore,
        scratch: &Path,
        claude: &Path,
        cwd: Option<&str>,
    ) -> (Duration, bool, bool) {
        let mut payload = serde_json::json!({
            "session_id": SESSION_ID,
            "hook_event_name": "SessionStart",
            "source": "startup",
        });
        if let Some(cwd) = cwd {
            payload["cwd"] = Value::String(cwd.to_owned());
        }
        let started = Instant::now();
        let mut child = Command::new(env!("CARGO_BIN_EXE_terminus"))
            .args(["hook", "SessionStart"])
            .current_dir(scratch)
            .env("TERMINUS_DATA_DIR", &store.0)
            .env("CLAUDE_CONFIG_DIR", claude)
            .env_remove(testkit::INJECT_DEADLINE_ENV)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|_| panic!("spawn a hook"));
        // A hook that exits before reading its stdin shows as no brief below.
        let _ = child
            .stdin
            .take()
            .unwrap_or_else(|| panic!("the hook's stdin"))
            .write_all(format!("{payload}\n").as_bytes());
        let output = child
            .wait_with_output()
            .unwrap_or_else(|_| panic!("wait for a hook"));
        let elapsed = started.elapsed();

        let delivered = serde_json::from_slice::<Value>(&output.stdout)
            .ok()
            .and_then(|doc| {
                doc["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .map(|text| !text.is_empty())
            })
            .unwrap_or(false);
        let abandoned = output
            .stderr
            .windows(ABANDONED.len())
            .any(|w| w == ABANDONED);
        (elapsed, delivered, abandoned)
    }

    /// A count from the copy, failing without anything the query returned.
    fn count(conn: &Connection, sql: &str, what: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0))
            .unwrap_or_else(|_| panic!("read {what}"))
    }

    pub fn run(copy: &Path) {
        // The copy: resolved, under /var/tmp, outside the repository, in a 0700
        // scratch directory, and holding a store.
        let copy = copy
            .canonicalize()
            .unwrap_or_else(|_| panic!("{COPY_DIR} does not name an existing path"));
        let var_tmp = Path::new("/var/tmp")
            .canonicalize()
            .unwrap_or_else(|_| panic!("/var/tmp does not resolve"));
        assert!(
            copy.starts_with(&var_tmp),
            "{COPY_DIR} is not under /var/tmp"
        );
        // The repository is two levels above this crate's manifest.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap_or_else(|| panic!("the repository root"))
            .canonicalize()
            .unwrap_or_else(|_| panic!("the repository root exists"));
        assert!(
            !copy.starts_with(&repo),
            "{COPY_DIR} is inside the repository"
        );
        let scratch = copy
            .parent()
            .unwrap_or_else(|| panic!("{COPY_DIR} has no scratch parent"))
            .to_path_buf();
        let mode = std::fs::metadata(&scratch)
            .unwrap_or_else(|_| panic!("the scratch directory is unreadable"))
            .permissions()
            .mode()
            & 0o777;
        assert!(mode == 0o700, "the scratch directory is not mode 0700");
        assert!(
            copy.join(DB_FILE_NAME).is_file(),
            "{COPY_DIR} holds no store"
        );
        // The hook reads the real config directory. It must not be redirected,
        // so his exclusions and redaction apply to the renders.
        assert!(
            std::env::var_os(CONFIG_DIR_ENV).is_none(),
            "{CONFIG_DIR_ENV} is set"
        );

        // The fill, under a scratch config with snapshots off and nothing else.
        let config = scratch.join("config");
        let claude = scratch.join("claude");
        std::fs::create_dir_all(&config).unwrap_or_else(|_| panic!("make the scratch config"));
        std::fs::create_dir_all(claude.join("projects"))
            .unwrap_or_else(|_| panic!("make the scratch claude directory"));
        std::fs::write(
            config.join(CONFIG_FILE_NAME),
            "[snapshot]\nenabled = false\n",
        )
        .unwrap_or_else(|_| panic!("write the scratch config"));
        let started = Instant::now();
        let filled = Command::new(env!("CARGO_BIN_EXE_terminus"))
            .arg("ingest")
            .current_dir(&scratch)
            .env("TERMINUS_DATA_DIR", &copy)
            .env(CONFIG_DIR_ENV, &config)
            .env("CLAUDE_CONFIG_DIR", &claude)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap_or_else(|_| panic!("spawn the fill"));
        let fill_seconds = started.elapsed().as_secs_f64();
        let code = filled.code().unwrap_or(-1);
        assert!(filled.success(), "the fill exited {code}");
        let snapshots = std::fs::read_dir(copy.join(SNAPSHOT_DIR)).map_or(0, Iterator::count);
        assert!(
            snapshots == 0,
            "the fill left {snapshots} files under the copy's snapshots directory"
        );

        // Checks on one read-only connection. The pairs stay in memory.
        let pairs: Vec<(Option<String>, Option<String>)> = {
            let conn = Connection::open_with_flags(
                copy.join(DB_FILE_NAME),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap_or_else(|_| panic!("open the copy read-only"));
            let filled: i64 = conn
                .query_row(
                    "SELECT count(*) FROM meta WHERE key = ?1",
                    [META_SUMMARY_FILLED],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| panic!("read the fill key"));
            assert!(filled == 1, "the copy's summary is not marked filled");
            let indexes = count(
                &conn,
                &format!(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'index' \
                     AND tbl_name = 'session_summary' AND name IN ('{}', '{}', '{}')",
                    INDEXES[0], INDEXES[1], INDEXES[2]
                ),
                "the index count",
            );
            assert!(
                indexes == INDEXES.len() as i64,
                "the copy's session_summary lacks the index"
            );
            let mut read = conn
                .prepare(
                    "SELECT project, project_pre_worktree FROM project_summary ORDER BY pair_key",
                )
                .unwrap_or_else(|_| panic!("prepare the pair read"));
            let rows = read
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap_or_else(|_| panic!("read the pairs"))
                .collect::<Result<_, _>>()
                .unwrap_or_else(|_| panic!("read a pair"));
            rows
        };

        // Held until every render is done: no hook's detached ingest may touch the copy.
        let _lock =
            match ingest::lock::try_acquire(&copy).unwrap_or_else(|_| panic!("take the lock")) {
                Attempt::Acquired(lock) => lock,
                Attempt::Held => panic!("the copy's ingest lock is held elsewhere"),
            };

        let store = check_hook_store(&copy, &scratch);
        let files = [
            copy.join(DB_FILE_NAME),
            copy.join(format!("{DB_FILE_NAME}-wal")),
        ];
        let (mut times, mut delivered, mut abandoned) = (Vec::new(), 0usize, 0usize);
        for (ordinal, (project, pre_worktree)) in pairs.iter().enumerate() {
            let cwd = project.as_deref().or(pre_worktree.as_deref());
            // A store with no write since the last checkpoint has no -wal yet.
            let mut resident = [0usize; 2];
            for (file, pages) in files.iter().zip(&mut resident) {
                if file.is_file() {
                    testkit::evict_from_page_cache(file);
                    *pages = testkit::resident_pages(file);
                }
            }
            assert!(
                resident == [0, 0],
                "render {}: {} pages of the db and {} of the wal are resident after eviction",
                ordinal + 1,
                resident[0],
                resident[1]
            );
            let (elapsed, brief, gave_up) = render(&store, &scratch, &claude, cwd);
            times.push(elapsed);
            delivered += usize::from(brief);
            abandoned += usize::from(gave_up);
        }

        times.sort();
        let ms = |d: Option<&Duration>| d.map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let renders = times.len();
        println!(
            "cold copy: renders {renders}, briefs delivered {delivered}, abandoned {abandoned}, \
             slowest {:.1} ms, median {:.1} ms, fill {fill_seconds:.0} s; the summary pages \
             were freshly written by the fill, and the copy was laid out fresh on disk",
            ms(times.last()),
            ms(times.get(renders.saturating_sub(1) / 2)),
        );
        assert!(
            renders == pairs.len(),
            "{renders} renders for {} pairs",
            pairs.len()
        );
        assert!(abandoned == 0, "{abandoned} of {renders} renders abandoned");
        assert!(
            delivered >= 1,
            "{delivered} of {renders} renders delivered a brief"
        );
    }
}

#[test]
fn every_project_starts_inside_the_deadline_from_a_cold_page_cache_on_a_copy_of_the_real_archive() {
    let Some(copy) = std::env::var_os(COPY_DIR).filter(|v| !v.is_empty()) else {
        println!("brief_cold_copy skipped: {COPY_DIR} is not set");
        return;
    };
    #[cfg(target_os = "linux")]
    linux::run(std::path::Path::new(&copy));
    #[cfg(not(target_os = "linux"))]
    {
        let _ = copy;
        panic!("a cold page cache needs posix_fadvise and mincore, which are Linux only");
    }
}
