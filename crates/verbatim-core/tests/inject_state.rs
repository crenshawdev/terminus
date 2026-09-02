//! The per-session injection state file (INJ-04, D-06).
//!
//! Real files in real temporary directories, never a mock: what is under test
//! is a write that survives a kill and a name that comes off an untrusted
//! payload, and neither property exists anywhere but on a filesystem.

use std::path::{Path, PathBuf};

use verbatim_core::inject::state::{self, Reason, State, Suppressed};

/// A `session_id` of the shape Claude Code sends, and a second one, so a test
/// can show that one session's file is not another's.
const SESSION: &str = "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55";
const OTHER: &str = "3b7c11ff-4a20-4d99-8e01-7c5da2b64e10";

fn data_dir(dir: &tempfile::TempDir) -> PathBuf {
    // Under the temp dir rather than at it: several cases below assert that
    // nothing created the data directory, which is only observable when the
    // helper did not create it either.
    dir.path().join("data")
}

fn injection_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(state::DIR_NAME)
}

fn file_of(data_dir: &Path, session_id: &str) -> PathBuf {
    injection_dir(data_dir).join(format!("{session_id}.json"))
}

/// Something in every field, so a round trip that silently dropped one is
/// visible.
fn populated() -> State {
    let mut state = State::default();
    state.injected.extend([12_582_913, 12_582_915]);
    state.brief.push(41_943_040);
    state.suppressed.push(Suppressed {
        turn_id: 12_582_913,
        reason: Reason::AlreadyInjected,
    });
    state.suppressed.push(Suppressed {
        turn_id: 41_943_040,
        reason: Reason::CarriedByBrief,
    });
    state.compaction_owed = true;
    state
}

/// Everything one session was given, written and read back - and AC4's
/// requirement that the reason is legible in the file itself.
#[test]
fn one_sessions_state_survives_a_round_trip_through_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = data_dir(&dir);

    // The falsifying half: it really is empty before the write, so what comes
    // back after it is what the write put there.
    assert_eq!(State::load(&data_dir, Some(SESSION)), State::default());

    let state = populated();
    assert!(state.save(&data_dir, Some(SESSION)));
    assert_eq!(State::load(&data_dir, Some(SESSION)), state);

    // AC4 asks for a suppressed turn and its reason to be readable here, so
    // the assertion is against the bytes rather than against the parse.
    let text = std::fs::read_to_string(file_of(&data_dir, SESSION)).unwrap();
    assert!(text.contains("already_injected"), "{text}");
    assert!(text.contains("carried_by_brief"), "{text}");
    assert!(text.contains("12582913"), "{text}");

    // Keyed on the session id: another session under the same data directory
    // is another file, and this one told it nothing.
    assert_eq!(State::load(&data_dir, Some(OTHER)), State::default());
}

/// Every way the file can be unreadable is the empty state, never an error.
///
/// The last entry is the control. A well-formed document of this build's shape
/// really does load, so the cases above it are the shapes being refused rather
/// than a reader that always answers `default`.
#[test]
fn a_file_this_build_cannot_read_is_empty_state_rather_than_a_failure() {
    let refused: &[(&str, &[u8])] = &[
        ("nothing at all", b""),
        ("prose", b"not json, a note about json"),
        ("a truncated document", br#"{"format":1,"injected":[125"#),
        ("a JSON value that is not an object", b"[1, 2, 3]"),
        // No `format` at all: whatever wrote it, it was not this build.
        ("an object of ours minus its format", br#"{"injected":[7]}"#),
        // A format this build does not know. There is no migration and the
        // file is disposable, so it is discarded rather than half-read.
        (
            "a format from another build",
            br#"{"format":9999,"injected":[7]}"#,
        ),
        // A reason this build has no variant for: phase 6 may add reasons, and
        // a file naming one must not become an error on the prompt path.
        (
            "a suppression reason this build has no name for",
            br#"{"format":1,"suppressed":[{"turn_id":7,"reason":"budget"}]}"#,
        ),
        ("a document of the wrong types", br#"{"format":"1"}"#),
    ];

    for (what, bytes) in refused {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = data_dir(&dir);
        std::fs::create_dir_all(injection_dir(&data_dir)).unwrap();
        std::fs::write(file_of(&data_dir, SESSION), bytes).unwrap();

        assert_eq!(
            State::load(&data_dir, Some(SESSION)),
            State::default(),
            "{what} did not read as empty state"
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let data_dir = data_dir(&dir);
    std::fs::create_dir_all(injection_dir(&data_dir)).unwrap();
    std::fs::write(
        file_of(&data_dir, SESSION),
        br#"{"format":1,"injected":[7],"compaction_owed":true}"#,
    )
    .unwrap();
    let read = State::load(&data_dir, Some(SESSION));
    assert_eq!(read.injected, vec![7]);
    assert!(read.compaction_owed);
    assert!(read.suppressed.is_empty(), "{read:?}");
}

/// The ordinary state of a machine that has installed verbatim and not yet
/// injected anything: no directory, no file, and no error.
///
/// Nothing is created by asking, either. A read that made the data directory
/// would be a `SessionEnd` hook leaving a tree behind as the side effect of a
/// question it had nothing to answer.
#[test]
fn a_data_directory_with_no_injection_state_reads_empty_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = data_dir(&dir);

    assert_eq!(State::load(&data_dir, Some(SESSION)), State::default());
    // And an event that carried no session id at all.
    assert_eq!(State::load(&data_dir, None), State::default());

    assert!(!data_dir.exists(), "the read created the data directory");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "the read left something behind"
    );
}

/// A `session_id` that could name a path reaches no filesystem call.
///
/// The `session_id` comes off an untrusted payload and becomes a file name, so
/// the refusal is an allow-list applied before a path is built - not a `join`
/// checked afterwards. Nothing here may create a file, and that includes the
/// data directory and the injection directory themselves: a refusal that still
/// made the tree would prove the check runs too late.
#[test]
fn a_session_id_that_could_name_a_path_creates_nothing_anywhere() {
    let refused: &[&str] = &[
        "../../../etc/passwd",
        "..",
        "0e5e6a1e-9f2b/../../4c7a",
        "0e5e6a1e/9f2b-4c7a-8d31-6b4f2a9c1d55",
        r"0e5e6a1e\9f2b-4c7a-8d31-6b4f2a9c1d55",
        "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55/../..",
        "c:0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55",
        "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55.json",
        "0e5e6a1e 9f2b 4c7a 8d31 6b4f2a9c1d55",
        "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55\u{0}",
        "0e5e6a1e-9f2b-4c7a-8d31-6b4f2a9c1d55.",
        "CON",
        "",
        "0e5e6a",
        // Hex and dashes, and far too long to be a session id.
        "0e5e6a1e9f2b4c7a8d316b4f2a9c1d550e5e6a1e9f2b4c7a8d316b4f2a9c1d55f",
    ];

    for session_id in refused {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = data_dir(&dir);

        assert_eq!(
            State::load(&data_dir, Some(session_id)),
            State::default(),
            "{session_id:?} was read from somewhere"
        );
        assert!(
            !populated().save(&data_dir, Some(session_id)),
            "{session_id:?} was accepted as a file name"
        );
        assert!(
            !data_dir.exists(),
            "{session_id:?} created the data directory"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "{session_id:?} created something in the temp root"
        );
    }

    // The control: a real session id under the same helper does write, so the
    // refusals above are about the names and not about a save that never works.
    let dir = tempfile::tempdir().unwrap();
    let data_dir = data_dir(&dir);
    assert!(populated().save(&data_dir, Some(SESSION)));
    assert!(file_of(&data_dir, SESSION).is_file());
}

/// The write lands atomically: the target is replaced, never rewritten in
/// place, so a kill mid-write leaves the previous document rather than half of
/// the next one.
///
/// The inode comparison is the direct observation of that. A rename gives the
/// name a new inode; an `open`-truncate-write keeps the old one, and that is
/// exactly the implementation where a kill between the truncate and the last
/// byte leaves a prefix where a read expects a document. It is unix-only
/// because the number is; the rest of the case holds on every platform.
#[test]
fn a_write_replaces_the_target_rather_than_rewriting_it_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = data_dir(&dir);
    let target = file_of(&data_dir, SESSION);

    let mut first = State::default();
    first.injected.push(1);
    assert!(first.save(&data_dir, Some(SESSION)));
    #[cfg(unix)]
    let before = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&target).unwrap().ino()
    };

    // Longer than the first, so a truncating writer that died part way would
    // leave a document that still parses - the failure this shape rules out.
    let mut second = State::default();
    second.injected.extend(0..200);
    second.compaction_owed = true;
    assert!(second.save(&data_dir, Some(SESSION)));

    assert_eq!(State::load(&data_dir, Some(SESSION)), second);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let after = std::fs::metadata(&target).unwrap().ino();
        assert_ne!(
            before, after,
            "the target was written in place, so a kill mid-write can leave a \
             prefix of it where the next prompt expects a document"
        );
    }

    // And the temporary went with it: one file in the directory, the target.
    let entries: Vec<String> = std::fs::read_dir(injection_dir(&data_dir))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec![target.file_name().unwrap().to_string_lossy().into_owned()],
        "a temporary was left beside the state file"
    );
}

/// PRIV-02, PRIV-04: the injection scratch is quoted prompt text keyed by
/// session id, and none of it - nor the directory holding it, nor the data
/// directory the hook had to create to get there - is readable by group or
/// world.
///
/// The data directory does not exist when this starts, which is the case that
/// matters: a `SessionStart` hook on a machine that has installed verbatim and
/// never ingested is the first thing to create it, so the mode it lands with is
/// this write's to get right. That the directories ABOVE it keep the user's own
/// default is D-04, asserted where it can be seen -
/// `tests/pass.rs::the_data_directory_and_the_lock_are_owner_only_from_the_first_acquire`
/// - since a `tempfile` root is already 0700 and would prove nothing here.
#[cfg(unix)]
#[test]
fn the_scratch_the_hook_writes_is_owner_only_and_so_is_what_holds_it() {
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    let dir = tempfile::tempdir().unwrap();
    let data_dir = data_dir(&dir);
    assert!(!data_dir.exists(), "the data directory exists already");

    assert!(populated().save(&data_dir, Some(SESSION)));

    assert_eq!(mode(&data_dir), 0o700, "the data directory is not owner-only");
    assert_eq!(
        mode(&injection_dir(&data_dir)),
        0o700,
        "the injection directory is not owner-only"
    );
    assert_eq!(
        mode(&file_of(&data_dir, SESSION)),
        0o600,
        "the session's state file is not owner-only"
    );

    // The temporary is 0600 for its whole life too - it holds the same text -
    // and it is not left behind for a mode to be asserted on afterwards, which
    // is why the shape that proves it is `create_new` on the primitive's
    // options rather than a stat here.
    let entries: Vec<String> = std::fs::read_dir(injection_dir(&data_dir))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec![format!("{SESSION}.json")]);
}
