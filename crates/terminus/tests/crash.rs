//! AC2 / AC6 / STOR-02: killing ingest anywhere leaves a store that converges.
//!
//! The harness kills the child un-catchably - `Child::kill` is `SIGKILL` on
//! Unix and `TerminateProcess` on Windows - at more than forty points across
//! three scenarios, spread three ways. Some are *aimed at a moment*: the child
//! announces that it has reached a named point inside one file's pass and waits
//! there to be killed. Some are *aimed at a file*: the walk stalls once a given
//! number of transcripts have committed, so the kill lands in the middle of a
//! tree. The rest are timed, at pseudorandom delays.
//!
//! Aiming is not a convenience. A release ingest of a few-KB fixture finishes
//! in single-digit milliseconds, so a harness that only randomizes on elapsed
//! time lands nearly every kill after the commit and passes without ever
//! testing anything. All three spreads are here because each covers what the
//! others cannot: moment-aimed kills prove the transaction boundary, file-aimed
//! kills prove that a half-walked tree resumes, timed kills sample the places
//! nobody thought to name.
//!
//! Three scenarios, because they fail differently. One transcript killed on a
//! first pass is phase 1's. A *tree* killed on a first pass is what a real hook
//! spawn does, and a tree killed on an **append** pass is the one phase 1 never
//! covered and flagged: every kill it ran went into a fresh data directory, so
//! the resume path - a store that already holds bytes for the file being killed
//! - had never been interrupted at all.
//!
//! Everything rests on the invariant check being able to fail, so
//! `the_harness_catches_a_watermark_committed_outside_the_transaction` runs the
//! same procedure against an ingest whose watermark commits in a transaction of
//! its own, and against a watermark moved off a record boundary, and requires
//! the check to reject both. Those negatives are tests rather than one-off
//! edits someone reverted.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use terminus_core::ingest::fault;
use terminus_core::store::{Store, DB_FILE_NAME};
use terminus_core::testkit::{self, Rng};
use terminus_core::{blob, verify};

/// Aimed kills against one transcript: every point, this many times.
const ROUNDS: usize = 3;
/// Timed kills, at pseudorandom delays.
const TIMED: usize = 8;
/// Timed kills per tree scenario. Lower than [`TIMED`] because each tree
/// iteration pays for a whole convergence pass over four transcripts.
const TREE_TIMED: usize = 4;
/// How long to wait for a child to announce that it reached its point.
const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(20);

/// Every directory a spawned `terminus` may resolve, all of them temporary.
///
/// The config and Claude directories are as load-bearing as the data directory.
/// A bare `terminus ingest` walks the *configured* roots, so a spawn that set
/// only `TERMINUS_DATA_DIR` would resolve the developer's real config and walk
/// the live `~/.claude` tree - 2,000+ private transcripts ingested into a temp
/// dir on every `cargo test`. No test process may resolve a real root, and that
/// is true of the single-file spawns too: one wrong argument away is all it is.
struct Dirs {
    _dir: tempfile::TempDir,
    root: PathBuf,
    /// Scratch for transcripts a spawn is handed by path.
    work: PathBuf,
    /// Holds no `terminus.toml`, so the loader yields defaults.
    config_dir: PathBuf,
    /// `<claude_dir>/projects` is the only tree a bare pass can reach.
    claude_dir: PathBuf,
}

fn dirs() -> Dirs {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let work = root.join("work");
    let config_dir = root.join("config");
    let claude_dir = root.join("claude");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    Dirs {
        _dir: dir,
        root,
        work,
        config_dir,
        claude_dir,
    }
}

impl Dirs {
    /// A data directory of its own, named for the iteration that owns it.
    fn data(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

/// A transcript big enough that a timed kill has somewhere to land: a release
/// pass over it takes long enough to sample.
fn big_transcript(dir: &Path, name: &str, repeats: usize) -> PathBuf {
    let mut bytes = Vec::new();
    for _ in 0..repeats {
        bytes.extend_from_slice(&testkit::fixture_bytes("session-basic.jsonl"));
        bytes.extend_from_slice(&testkit::fixture_bytes("session-large-record.jsonl"));
    }
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    path
}

/// Four transcripts of very different sizes, over two projects, including a
/// sidecar at the depth real ones sit at
/// (`<project>/<sessionId>/subagents/agent-*.jsonl`).
///
/// Returned in the order discovery yields them, which is sorted by path: the
/// file-aimed kills count commits in exactly that order.
fn build_tree(claude_dir: &Path) -> Vec<PathBuf> {
    let projects = claude_dir.join("projects");
    let one = projects.join("-data-code-verbatim");
    let two = projects.join("-data-projects-cadence");

    // ~1 MB, so a timed kill has a wide window inside a single file.
    let big = big_transcript(&one, "11111111-1111-4111-8111-111111111111.jsonl", 6);
    let small = place(
        &one,
        "22222222-2222-4222-8222-222222222222.jsonl",
        "session-basic.jsonl",
    );
    let sidecar = place(
        &one,
        "33333333-3333-4333-8333-333333333333/subagents/agent-a.jsonl",
        "subagents/agent-alpha.jsonl",
    );
    let other = place(
        &two,
        "44444444-4444-4444-8444-444444444444.jsonl",
        "session-continuation.jsonl",
    );

    let tree = vec![big, small, sidecar, other];
    let mut sorted = tree.clone();
    sorted.sort();
    assert_eq!(tree, sorted, "the tree is not in discovery order");
    tree
}

fn place(project: &Path, name: &str, fixture: &str) -> PathBuf {
    let dest = project.join(name);
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::copy(testkit::fixture_path(fixture), &dest).unwrap();
    dest
}

/// Append whole records to a transcript, the way a live session grows.
///
/// Distinct bytes per file and per record, so an append that landed on the
/// wrong session or was written twice shows up as a different blob rather than
/// as a coincidence.
fn append_records(path: &Path, count: usize) {
    let tag = path.file_stem().unwrap().to_str().unwrap().to_owned();
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    for i in 0..count {
        writeln!(
            file,
            r#"{{"type":"user","uuid":"appended-{tag}-{i}","timestamp":"2026-08-12T0{i}:00:00.000Z","cwd":"/data/code/verbatim","message":{{"role":"user","content":"crash append {tag} {i}"}}}}"#
        )
        .unwrap();
    }
}

/// Copy a whole data directory, so an iteration can start from a store that
/// already holds an earlier pass rather than from nothing.
///
/// This is what makes the append scenario possible at all: the tree's files
/// have already grown by then, so the pre-append store cannot be rebuilt by
/// ingesting them again - it has to be kept.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

/// A spawn of the binary with every directory it may resolve pointed at this
/// test's own temporaries, and every fault variable explicitly cleared.
///
/// `transcript: None` is a bare `terminus ingest`: the tree pass.
fn spawn(dirs: &Dirs, data_dir: &Path, transcript: Option<&Path>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_terminus"));
    command.arg("ingest");
    if let Some(path) = transcript {
        command.arg(path);
    }
    command
        .env("TERMINUS_DATA_DIR", data_dir)
        .env("TERMINUS_CONFIG_DIR", &dirs.config_dir)
        .env("CLAUDE_CONFIG_DIR", &dirs.claude_dir)
        .env_remove(fault::AT)
        .env_remove(fault::READY)
        .env_remove(fault::SPLIT)
        .env_remove(fault::AFTER_FILES_COUNT);
    command
}

/// Run ingest to completion, with no fault injection.
fn ingest_fully(dirs: &Dirs, data_dir: &Path, transcript: Option<&Path>) {
    let out = spawn(dirs, data_dir, transcript).output().unwrap();
    assert!(
        out.status.success(),
        "the completing pass failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Where a kill is aimed.
#[derive(Debug, Clone, Copy)]
enum Aim<'a> {
    /// A named moment inside one file's pass.
    Moment(&'a str),
    /// Between files: stall once this many transcripts have committed.
    AfterFiles(usize),
    /// The watermark-outside-the-transaction fault, for the negative test.
    SplitWatermark,
}

/// Kill the child once it reports that it reached the point it was aimed at.
fn kill_at(dirs: &Dirs, data_dir: &Path, transcript: Option<&Path>, aim: Aim<'_>) {
    std::fs::create_dir_all(data_dir).unwrap();
    let ready = data_dir.join("fault-ready");
    let _ = std::fs::remove_file(&ready);

    let mut command = spawn(dirs, data_dir, transcript);
    command.env(fault::READY, &ready);
    let point = match aim {
        Aim::Moment(point) => point,
        Aim::AfterFiles(n) => {
            command.env(fault::AFTER_FILES_COUNT, n.to_string());
            fault::AFTER_FILES
        }
        Aim::SplitWatermark => {
            command.env(fault::SPLIT, "1");
            fault::AFTER_SPLIT_WATERMARK
        }
    };
    command.env(fault::AT, point);
    let mut child = command.spawn().unwrap();

    let deadline = Instant::now() + ARRIVAL_TIMEOUT;
    while !ready.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "the child exited ({status}) before reaching `{point}`; \
                 build the binary with --features testkit or the fault points are inert"
            );
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the child to reach `{point}`"
        );
        std::thread::sleep(Duration::from_micros(200));
    }

    kill_and_reap(&mut child);
}

/// Kill the child after `delay`, wherever it happens to be.
fn kill_after(dirs: &Dirs, data_dir: &Path, transcript: Option<&Path>, delay: Duration) {
    std::fs::create_dir_all(data_dir).unwrap();
    let mut child = spawn(dirs, data_dir, transcript).spawn().unwrap();
    std::thread::sleep(delay);
    kill_and_reap(&mut child);
}

/// `SIGKILL` on Unix, `TerminateProcess` on Windows: un-catchable either way,
/// which is the point - a signal the process could handle would let it tidy up
/// and prove nothing.
fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    child.wait().unwrap();
}

/// What a store holds, minus everything a pass count changes.
///
/// `runs` is excluded on purpose: an interrupted store reaches the same
/// contents in more passes than an uninterrupted one, and one row per committed
/// pass is exactly the record it is supposed to keep.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    blobs: Vec<(String, Vec<u8>)>,
    turns: Vec<(i64, String, i64, String, i64, i64)>,
    fts: Vec<i64>,
    watermarks: Vec<(String, i64)>,
}

impl Snapshot {
    fn of(data_dir: &Path) -> Snapshot {
        let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
        let snapshot = Snapshot {
            blobs: conn
                .prepare("SELECT session_key, blob FROM sessions ORDER BY session_key")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect(),
            turns: conn
                .prepare(
                    "SELECT id, session_key, turn_seq, record_type, stream_offset, byte_len
                     FROM turns ORDER BY id",
                )
                .unwrap()
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect(),
            fts: conn
                .prepare("SELECT rowid FROM turns_fts ORDER BY rowid")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect(),
            watermarks: conn
                .prepare("SELECT transcript_path, byte_offset FROM watermarks ORDER BY 1")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect(),
        };
        snapshot
    }
}

/// AC2's invariants, as a check that returns its complaint instead of
/// panicking: the negative tests need to assert that it *does* complain.
///
/// Five properties, and it is worth being explicit about which is which,
/// because two of them look like one and are not:
///
/// 1. **Every blob verifies.** Decompresses, and hashes to the BLAKE3 recorded
///    for it (D-06). A killed pass must never leave a blob its own checksum
///    disowns.
/// 2. **No orphaned derived row.** Nothing in `turns`, `turns_fts`, `entities`
///    or `paths` may outlive the row it derives from - which is what "one
///    transaction" buys and the only thing that makes a rebuild meaningful.
/// 3. **Containment, for turn rows.** No turn row may address bytes past the
///    end of the committed blob. The *weaker* of the two addressing properties.
/// 4. **Containment, for the watermark.** The watermark may not sit past the
///    committed blob either. Also weak: it rejects a store that claims bytes
///    nothing holds, and accepts one that resumes from the middle of a record.
/// 5. **The watermark is a record boundary.** The *strong* property, and the
///    one that actually encodes D-14: the watermark is the byte just past a
///    `\n`, so the byte before it in the decompressed stream is a newline. A
///    watermark one byte short of a boundary passes 4 and fails 5, and a pass
///    resuming from it would archive half a record as a whole one.
///
/// Plus a store-wide one: a watermark past zero with no archived session claims
/// bytes nothing holds at all.
fn check_invariants(data_dir: &Path) -> Result<(), String> {
    if !data_dir.join(DB_FILE_NAME).exists() {
        // The kill landed before the store file existed. Nothing is claimed, so
        // nothing can be inconsistent.
        return Ok(());
    }
    let store = Store::open(data_dir).map_err(|e| format!("the store will not open: {e}"))?;
    let conn = store.conn();

    // (1) Every blob decompresses and matches the BLAKE3 recorded for it.
    let report = verify::verify(&store).map_err(|e| format!("verify failed: {e}"))?;
    if !report.is_ok() {
        return Err(format!("blobs failed verification: {}", report.render()));
    }

    // (2) No derived row without the row it derives from.
    for (what, sql) in [
        (
            "turns",
            "SELECT count(*) FROM turns
             WHERE session_key NOT IN (SELECT session_key FROM sessions)",
        ),
        (
            "turns_fts",
            "SELECT count(*) FROM turns_fts WHERE rowid NOT IN (SELECT id FROM turns)",
        ),
        (
            "entities",
            "SELECT count(*) FROM entities WHERE turn_id NOT IN (SELECT id FROM turns)",
        ),
        (
            "paths",
            "SELECT count(*) FROM paths WHERE turn_id NOT IN (SELECT id FROM turns)",
        ),
        (
            "compaction_boundaries",
            "SELECT count(*) FROM compaction_boundaries
             WHERE turn_id NOT IN (SELECT id FROM turns)",
        ),
    ] {
        let orphans: i64 = conn.query_row(sql, [], |r| r.get(0)).unwrap();
        if orphans != 0 {
            return Err(format!("{orphans} orphaned {what} row(s)"));
        }
    }

    // What each session's committed blob actually holds.
    let sessions: Vec<(String, Vec<u8>)> = conn
        .prepare("SELECT session_key, blob FROM sessions")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    for (session_key, bytes) in &sessions {
        let reader = blob::BlobReader::open(bytes)
            .map_err(|e| format!("{session_key}: blob will not parse: {e}"))?;
        let committed = reader.uncompressed_len() as i64;

        // (3) Containment, for turn rows.
        let past: i64 = conn
            .query_row(
                "SELECT coalesce(max(stream_offset + byte_len), 0) FROM turns WHERE session_key = ?1",
                [session_key],
                |r| r.get(0),
            )
            .unwrap();
        if past > committed {
            return Err(format!(
                "{session_key}: a turn row addresses byte {past} of a {committed}-byte blob"
            ));
        }

        let watermark: Option<i64> = conn
            .query_row(
                "SELECT byte_offset FROM watermarks WHERE transcript_path = ?1",
                [session_key],
                |r| r.get(0),
            )
            .ok();
        match watermark {
            // (4) Containment, for the watermark.
            Some(at) if at > committed => {
                return Err(format!(
                    "{session_key}: watermark at {at} is past the {committed} bytes the \
                     committed blob holds"
                ))
            }
            // (5) The watermark is a record boundary. Offset 0 is one
            // vacuously: nothing has been consumed, so there is no byte before
            // it to be a newline.
            Some(at) if at > 0 => {
                let before = reader
                    .read_range((at - 1) as u64, 1)
                    .map_err(|e| format!("{session_key}: reading byte {} failed: {e}", at - 1))?;
                if before != b"\n" {
                    return Err(format!(
                        "{session_key}: watermark at {at} is not a record boundary - the byte \
                         before it in the committed blob is {:?}, not a newline, so a resume \
                         would archive the tail of a record as a record",
                        before[0] as char
                    ));
                }
            }
            _ => {}
        }
    }

    // A watermark with no session claims bytes nothing archived.
    let dangling: Vec<String> = conn
        .prepare(
            "SELECT transcript_path FROM watermarks
             WHERE byte_offset > 0
               AND transcript_path NOT IN (SELECT session_key FROM sessions)",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    if !dangling.is_empty() {
        return Err(format!(
            "watermark(s) with no archived session: {}",
            dangling.join(", ")
        ));
    }

    Ok(())
}

/// AC2. Twenty-three kills against one transcript, aimed and timed, from a
/// printed seed.
#[test]
fn killing_ingest_anywhere_leaves_a_store_that_converges() {
    let dirs = dirs();
    // One transcript for every iteration, so the session key - the canonical
    // path - is identical across them and the snapshots are comparable.
    let transcript = big_transcript(&dirs.work, "crash-corpus.jsonl", 24);
    assert!(
        std::fs::metadata(&transcript).unwrap().len() > 4 * 1024 * 1024,
        "the corpus must be worth killing"
    );
    let transcript = Some(transcript.as_path());

    // The reference: one uninterrupted pass, and how long it takes.
    let reference_dir = dirs.data("reference");
    let started = Instant::now();
    ingest_fully(&dirs, &reference_dir, transcript);
    let pass_time = started.elapsed();
    let reference = Snapshot::of(&reference_dir);
    assert!(!reference.turns.is_empty());
    println!(
        "uninterrupted pass: {pass_time:?}, {} turns",
        reference.turns.len()
    );

    let mut rng = Rng(testkit::seed(
        "killing_ingest_anywhere_leaves_a_store_that_converges",
    ));
    let mut kills = 0usize;
    let mut incomplete = 0usize;

    let iteration = |data_dir: &Path, kills: &mut usize, incomplete: &mut usize| {
        *kills += 1;
        check_invariants(data_dir)
            .unwrap_or_else(|why| panic!("kill {} left an inconsistent store: {why}", *kills));
        if data_dir.join(DB_FILE_NAME).exists() && Snapshot::of(data_dir) != reference {
            *incomplete += 1;
        }
        // Convergence: finish the job and land exactly where one pass lands.
        ingest_fully(&dirs, data_dir, transcript);
        check_invariants(data_dir)
            .unwrap_or_else(|why| panic!("kill {} did not converge: {why}", *kills));
        assert_eq!(
            Snapshot::of(data_dir),
            reference,
            "kill {} converged on a different store",
            *kills
        );
    };

    // Aimed: every point in the pass, in a seed-shuffled order.
    for round in 0..ROUNDS {
        for point in shuffled_points(&mut rng) {
            let data_dir = dirs.data(&format!("aimed-{round}-{point}"));
            kill_at(&dirs, &data_dir, transcript, Aim::Moment(point));
            iteration(&data_dir, &mut kills, &mut incomplete);
        }
    }

    // Timed: anywhere in a window a little wider than the pass itself.
    let window = (pass_time.as_micros() as u64 * 3 / 2).max(1_000);
    for index in 0..TIMED {
        let data_dir = dirs.data(&format!("timed-{index}"));
        kill_after(
            &dirs,
            &data_dir,
            transcript,
            Duration::from_micros(rng.below(window)),
        );
        iteration(&data_dir, &mut kills, &mut incomplete);
    }

    assert!(
        kills >= 20,
        "AC2 asks for 20 or more kill points, ran {kills}"
    );
    assert!(
        incomplete >= ROUNDS * 3,
        "only {incomplete} of {kills} kills interrupted anything; the harness is testing \
         a race it always loses"
    );
}

/// AC6. A tree pass killed on a first pass and again on an append pass, ten or
/// more kills each, every one resumed to completion and required to land on the
/// same store an uninterrupted pass reaches.
///
/// The append half is the one phase 1 could not do. Every kill it ran went into
/// an empty data directory, so `blob::append` - copying committed blocks
/// forward, verifying the stored checksum before it writes, resuming a session
/// at a non-zero watermark - had never been interrupted once.
#[test]
fn killing_a_tree_pass_converges_on_a_first_pass_and_on_an_append() {
    let dirs = dirs();
    let tree = build_tree(&dirs.claude_dir);
    let mut rng = Rng(testkit::seed(
        "killing_a_tree_pass_converges_on_a_first_pass_and_on_an_append",
    ));

    // --- Scenario one: a first pass over the whole tree. ---
    let first_reference_dir = dirs.data("tree-reference");
    let started = Instant::now();
    ingest_fully(&dirs, &first_reference_dir, None);
    let first_time = started.elapsed();
    let first_reference = Snapshot::of(&first_reference_dir);
    assert_eq!(
        first_reference.blobs.len(),
        tree.len(),
        "the reference pass did not archive the whole tree"
    );
    println!(
        "uninterrupted tree pass: {first_time:?}, {} sessions, {} turns",
        first_reference.blobs.len(),
        first_reference.turns.len()
    );

    let first_kills = kill_spread(
        &dirs,
        "first",
        None,
        &first_reference,
        first_time,
        tree.len(),
        &mut rng,
    );
    assert!(
        first_kills >= 10,
        "AC6 asks for ten or more kills against a first pass, ran {first_kills}"
    );

    // --- Scenario two: the same spread against an append pass. ---
    // Every transcript grows, so a kill between files always leaves work.
    for path in &tree {
        append_records(path, 3);
    }

    let append_reference_dir = dirs.data("append-reference");
    copy_dir(&first_reference_dir, &append_reference_dir);
    let started = Instant::now();
    ingest_fully(&dirs, &append_reference_dir, None);
    let append_time = started.elapsed();
    let append_reference = Snapshot::of(&append_reference_dir);
    assert_ne!(
        append_reference, first_reference,
        "the appends changed nothing, so the append scenario would test the first one twice"
    );
    assert_eq!(
        append_reference.turns.len(),
        first_reference.turns.len() + 3 * tree.len(),
        "the append pass did not add exactly the appended records"
    );
    println!(
        "uninterrupted append pass: {append_time:?}, {} turns",
        append_reference.turns.len()
    );

    let append_kills = kill_spread(
        &dirs,
        "append",
        Some(&first_reference_dir),
        &append_reference,
        append_time,
        tree.len(),
        &mut rng,
    );
    assert!(
        append_kills >= 10,
        "AC6 asks for ten or more kills against an append pass, ran {append_kills}"
    );
}

/// One scenario's worth of kills: file-aimed, moment-aimed and timed, each
/// resumed to completion and compared against `reference`. Returns the count.
///
/// `seed_dir` is the store every iteration starts from, copied per iteration -
/// `None` for a first pass, which starts from nothing.
fn kill_spread(
    dirs: &Dirs,
    label: &str,
    seed_dir: Option<&Path>,
    reference: &Snapshot,
    pass_time: Duration,
    files: usize,
    rng: &mut Rng,
) -> usize {
    let mut kills = 0usize;
    let mut interrupted = 0usize;

    let iteration = |data_dir: &Path, kills: &mut usize, interrupted: &mut usize| {
        *kills += 1;
        check_invariants(data_dir).unwrap_or_else(|why| {
            panic!("{label} kill {} left an inconsistent store: {why}", *kills)
        });
        if data_dir.join(DB_FILE_NAME).exists() && &Snapshot::of(data_dir) != reference {
            *interrupted += 1;
        }
        ingest_fully(dirs, data_dir, None);
        check_invariants(data_dir)
            .unwrap_or_else(|why| panic!("{label} kill {} did not converge: {why}", *kills));
        assert_eq!(
            &Snapshot::of(data_dir),
            reference,
            "{label} kill {} converged on a different store",
            *kills
        );
    };

    let start = |name: &str| -> PathBuf {
        let data_dir = dirs.data(name);
        if let Some(seed) = seed_dir {
            copy_dir(seed, &data_dir);
        }
        data_dir
    };

    // Aimed at a file: the walk stalls between transcripts, with the tree half
    // archived. Every count below `files` leaves at least one file to do, which
    // is what makes these the kills that always interrupt something.
    for after in 1..files {
        let data_dir = start(&format!("{label}-after-{after}"));
        kill_at(dirs, &data_dir, None, Aim::AfterFiles(after));
        iteration(&data_dir, &mut kills, &mut interrupted);
    }

    // Aimed at a moment: the per-file points, firing inside whichever file the
    // walk is in when it reaches them.
    for point in shuffled_points(rng) {
        let data_dir = start(&format!("{label}-at-{point}"));
        kill_at(dirs, &data_dir, None, Aim::Moment(point));
        iteration(&data_dir, &mut kills, &mut interrupted);
    }

    // Timed: anywhere in a window a little wider than the pass itself.
    let window = (pass_time.as_micros() as u64 * 3 / 2).max(1_000);
    for index in 0..TREE_TIMED {
        let data_dir = start(&format!("{label}-timed-{index}"));
        kill_after(
            dirs,
            &data_dir,
            None,
            Duration::from_micros(rng.below(window)),
        );
        iteration(&data_dir, &mut kills, &mut interrupted);
    }

    println!("{label}: {kills} kills, {interrupted} of them interrupted the pass");
    assert!(
        interrupted >= files - 1,
        "{label}: only {interrupted} of {kills} kills interrupted anything; the file-aimed \
         kills alone should have managed {}",
        files - 1
    );
    kills
}

/// [`fault::POINTS`] in a seed-shuffled order.
fn shuffled_points(rng: &mut Rng) -> Vec<&'static str> {
    let mut points = fault::POINTS.to_vec();
    for i in (1..points.len()).rev() {
        points.swap(i, rng.below(i as u64 + 1) as usize);
    }
    points
}

/// The negative confirmations, as tests rather than one-off edits.
///
/// Both halves attack the watermark, because the watermark is where the two
/// properties of `check_invariants` differ. If either of these ever passes
/// silently, every assertion in the harness above is worthless.
#[test]
fn the_harness_catches_a_watermark_committed_outside_the_transaction() {
    let dirs = dirs();
    let transcript = testkit::copy_fixture_into("session-basic.jsonl", &dirs.work);
    let transcript = Some(transcript.as_path());

    // Half one: containment. With the watermark committed in a transaction of
    // its own before the pass - which is what "the single transaction is real"
    // denies - a kill in between leaves the store claiming bytes no blob holds.
    let data_dir = dirs.data("split");
    kill_at(&dirs, &data_dir, transcript, Aim::SplitWatermark);

    let complaint = check_invariants(&data_dir)
        .expect_err("a watermark committed outside the pass must be caught");
    assert!(
        complaint.contains("watermark"),
        "the complaint must name the watermark: {complaint}"
    );

    // The control: the same kill point does not exist when the watermark stays
    // inside the transaction, and killing at the equivalent moment - everything
    // written, nothing committed - leaves a store the check accepts.
    let control = dirs.data("control");
    kill_at(
        &dirs,
        &control,
        transcript,
        Aim::Moment(fault::IN_TX_BEFORE_COMMIT),
    );
    check_invariants(&control).expect("an interrupted single-transaction pass is consistent");
}

/// Half two: the record boundary, which is the property containment cannot see.
///
/// A watermark moved back one byte is still well inside the committed blob, so
/// check (4) accepts it and only check (5) can object - and it must, because a
/// pass resuming from there would read the last byte of an archived record as
/// the first byte of a new one and archive the fragment.
#[test]
fn the_harness_catches_a_watermark_off_a_record_boundary() {
    let dirs = dirs();
    let transcript = testkit::copy_fixture_into("session-basic.jsonl", &dirs.work);
    let data_dir = dirs.data("boundary");
    ingest_fully(&dirs, &data_dir, Some(&transcript));

    // The premise: this store is clean before the edit, so the complaint below
    // is caused by the edit and by nothing else.
    check_invariants(&data_dir).expect("a completed pass is consistent");

    let moved = {
        let conn = Connection::open(data_dir.join(DB_FILE_NAME)).unwrap();
        conn.execute("UPDATE watermarks SET byte_offset = byte_offset - 1", [])
            .unwrap()
    };
    assert_eq!(moved, 1, "this test needs exactly one watermark to move");

    let complaint =
        check_invariants(&data_dir).expect_err("a watermark off a record boundary must be caught");
    assert!(
        complaint.contains("watermark") && complaint.contains("record boundary"),
        "the complaint must name the watermark and the property: {complaint}"
    );
    assert!(
        !complaint.contains("is past the"),
        "this must fail the boundary check, not the containment one: {complaint}"
    );
}
