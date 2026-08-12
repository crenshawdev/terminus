//! AC2 / STOR-02: killing ingest anywhere leaves a store that makes sense.
//!
//! The harness kills the child un-catchably - `Child::kill` is `SIGKILL` on
//! Unix and `TerminateProcess` on Windows - at more than twenty points, spread
//! two ways. Five of them are *aimed*: the child announces that it has reached
//! a named point inside the pass and waits there to be killed. The rest are
//! timed, at pseudorandom delays across a pass over a multi-megabyte transcript.
//!
//! Aiming is not a convenience. A release ingest of a few-KB fixture finishes
//! in single-digit milliseconds, so a harness that only randomizes on elapsed
//! time lands nearly every kill after the commit and passes without ever
//! testing anything. Both spreads are here because each covers what the other
//! cannot: aimed kills prove the transaction boundary, timed kills sample the
//! places nobody thought to name.
//!
//! Everything rests on the invariant check being able to fail, so
//! `the_harness_catches_a_watermark_committed_outside_the_transaction` runs the
//! same procedure against an ingest whose watermark commits in a transaction of
//! its own and requires the check to reject it. That negative is a test rather
//! than a one-off edit someone reverted.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use verbatim_core::ingest::fault;
use verbatim_core::store::{Store, DB_FILE_NAME};
use verbatim_core::testkit::{self, Rng};
use verbatim_core::{blob, verify};

/// Aimed kills: every point, this many times.
const ROUNDS: usize = 3;
/// Timed kills, at pseudorandom delays.
const TIMED: usize = 8;
/// How long to wait for a child to announce that it reached its point.
const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(20);

/// A transcript big enough that a timed kill has somewhere to land: a release
/// pass over it takes long enough to sample.
fn big_transcript(dir: &Path) -> PathBuf {
    let mut bytes = Vec::new();
    for _ in 0..24 {
        bytes.extend_from_slice(&testkit::fixture_bytes("session-basic.jsonl"));
        bytes.extend_from_slice(&testkit::fixture_bytes("session-large-record.jsonl"));
    }
    let path = dir.join("crash-corpus.jsonl");
    std::fs::write(&path, &bytes).unwrap();
    assert!(
        bytes.len() > 4 * 1024 * 1024,
        "the corpus must be worth killing"
    );
    path
}

fn spawn(transcript: &Path, data_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_verbatim"));
    command
        .arg("ingest")
        .arg(transcript)
        .env("VERBATIM_DATA_DIR", data_dir)
        .env_remove(fault::AT)
        .env_remove(fault::READY)
        .env_remove(fault::SPLIT);
    command
}

/// Run ingest to completion, with no fault injection.
fn ingest_fully(transcript: &Path, data_dir: &Path) {
    let out = spawn(transcript, data_dir).output().unwrap();
    assert!(
        out.status.success(),
        "the completing pass failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Kill the child once it reports that it reached `point`.
fn kill_at(transcript: &Path, data_dir: &Path, point: &str, split: bool) {
    std::fs::create_dir_all(data_dir).unwrap();
    let ready = data_dir.join("fault-ready");
    let _ = std::fs::remove_file(&ready);

    let mut command = spawn(transcript, data_dir);
    command.env(fault::AT, point).env(fault::READY, &ready);
    if split {
        command.env(fault::SPLIT, "1");
    }
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
fn kill_after(transcript: &Path, data_dir: &Path, delay: Duration) {
    std::fs::create_dir_all(data_dir).unwrap();
    let mut child = spawn(transcript, data_dir).spawn().unwrap();
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
/// panicking: the negative test needs to assert that it *does* complain.
fn check_invariants(data_dir: &Path) -> Result<(), String> {
    if !data_dir.join(DB_FILE_NAME).exists() {
        // The kill landed before the store file existed. Nothing is claimed, so
        // nothing can be inconsistent.
        return Ok(());
    }
    let store = Store::open(data_dir).map_err(|e| format!("the store will not open: {e}"))?;
    let conn = store.conn();

    // Every blob decompresses and matches the BLAKE3 recorded for it.
    let report = verify::verify(&store).map_err(|e| format!("verify failed: {e}"))?;
    if !report.is_ok() {
        return Err(format!("blobs failed verification: {}", report.render()));
    }

    // No derived row without the row it derives from.
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
            Some(at) if at > committed => {
                return Err(format!(
                    "{session_key}: watermark at {at} is past the {committed} bytes the \
                     committed blob holds"
                ))
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

/// AC2. Twenty-three kills, aimed and timed, from a printed seed.
#[test]
fn killing_ingest_anywhere_leaves_a_store_that_converges() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    // One transcript for every iteration, so the session key - the canonical
    // path - is identical across them and the snapshots are comparable.
    let transcript = big_transcript(&work);

    // The reference: one uninterrupted pass, and how long it takes.
    let reference_dir = dir.path().join("reference");
    let started = Instant::now();
    ingest_fully(&transcript, &reference_dir);
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
        ingest_fully(&transcript, data_dir);
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
        let mut points = fault::POINTS.to_vec();
        for i in (1..points.len()).rev() {
            points.swap(i, rng.below(i as u64 + 1) as usize);
        }
        for point in points {
            let data_dir = dir.path().join(format!("aimed-{round}-{point}"));
            kill_at(&transcript, &data_dir, point, false);
            iteration(&data_dir, &mut kills, &mut incomplete);
        }
    }

    // Timed: anywhere in a window a little wider than the pass itself.
    let window = (pass_time.as_micros() as u64 * 3 / 2).max(1_000);
    for index in 0..TIMED {
        let data_dir = dir.path().join(format!("timed-{index}"));
        kill_after(
            &transcript,
            &data_dir,
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

/// The negative confirmation, as a test rather than a one-off edit.
///
/// With the watermark committed in a transaction of its own before the pass -
/// which is what "the single transaction is real" denies - a kill in between
/// leaves the store claiming bytes no blob holds, and `check_invariants` must
/// say so. If this ever passes silently, every assertion in the harness above
/// is worthless.
#[test]
fn the_harness_catches_a_watermark_committed_outside_the_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let transcript = testkit::copy_fixture_into("session-basic.jsonl", &work);

    // Same procedure as the harness, with the watermark moved out.
    let data_dir = dir.path().join("split");
    kill_at(&transcript, &data_dir, fault::AFTER_SPLIT_WATERMARK, true);

    let complaint = check_invariants(&data_dir)
        .expect_err("a watermark committed outside the pass must be caught");
    assert!(
        complaint.contains("watermark"),
        "the complaint must name the watermark: {complaint}"
    );

    // The control: the same kill point does not exist when the watermark stays
    // inside the transaction, and killing at the equivalent moment - everything
    // written, nothing committed - leaves a store the check accepts.
    let control = dir.path().join("control");
    kill_at(&transcript, &control, fault::IN_TX_BEFORE_COMMIT, false);
    check_invariants(&control).expect("an interrupted single-transaction pass is consistent");
}
