//! `verbatim export <dir>`: the archive in a portable form, and a manifest that
//! says what that is (PRIV-04).
//!
//! **The portable form is the transcript.** One `.jsonl` file per session,
//! holding that session's uncompressed stream exactly as the archive stores it.
//! It is the form the data arrived in and the form any future importer would
//! read, and it needs no verbatim build to open. A copy of the store file is
//! deliberately NOT what this is - that is a snapshot, and it carries the
//! derived tables, the decision log and the observations along with it.
//!
//! **Nothing is redacted on the way out, and that is why the notice exists.**
//! Redaction is keyed on destination, never on operation (PRIV-01), and a
//! directory the user named is not egress - the bytes are already on this
//! machine and the user asked for them here. What that leaves is a directory of
//! plaintext transcripts somewhere a user may not have thought about, so
//! [`NOTICE`] is written into the manifest AND printed, in words, before the
//! counts. Saying it is the mitigation; filtering would be a different product.
//!
//! **It refuses rather than merges.** A destination that already holds anything
//! is left exactly as it is. An export interleaved with an older one is a
//! manifest that describes some of the files beside it, and overwriting a file
//! this command did not write is not a thing it may do at all.
//!
//! Sessions come from `config::visible::sessions`. Export is a read path and
//! ING-08 binds every read path alike: a project the user excluded is not
//! written out to a directory they may hand to someone else.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use verbatim_core::blob;
use verbatim_core::config::visible;
use verbatim_core::store::{ARCHIVE_FORMAT, META_ARCHIVE_FORMAT};

use super::json::Document;
use super::read::{self, Opened, Reader};
use super::{human, Failure};

/// The command name, which is also what the `--json` envelope reports.
const COMMAND: &str = "export";

/// The manifest's file name, which is also how a destination is recognized as
/// already holding an export.
const MANIFEST: &str = "manifest.json";

/// What this export is, said in the manifest and on the terminal both.
///
/// PRIV-04's "states what it contains" half. It is prose rather than a flag
/// because the reader who needs it is a person looking at a directory they just
/// created, and "unredacted: true" is not a sentence anyone reads as a warning.
const NOTICE: &str = "\
This export is the verbatim, unredacted transcripts: every prompt, tool call, tool result and \
assistant turn exactly as verbatim archived them, with nothing removed and nothing masked. \
Verbatim redacts at egress and keys that on the destination, and a directory you named is not \
egress - so nothing was filtered on the way out, and these files deserve the care you would give \
~/.claude/projects itself.
Not in this export: the derived tables and the search index, which rebuild from these bytes; the \
injection decision log; and the observations. A session marked evicted here has had its archived \
bytes reclaimed by retention, so its file is present and empty.";

pub struct Args {
    pub destination: PathBuf,
    pub json: bool,
}

pub fn run(args: Args) -> Result<(), Failure> {
    let reader = match read::open()? {
        Opened::Ready(reader) => reader,
        // Before the destination is touched: a machine that has never ingested
        // must not acquire an empty export directory by being asked a question.
        Opened::Nothing(reason) => {
            return read::empty(
                document(&args.destination, &Written::default()),
                &reason,
                args.json,
            )
        }
    };

    // Every refusal happens before one byte is written. A half-written export
    // beside a manifest that does not describe it is worse than no export.
    refuse_if_occupied(&args.destination)?;

    let written = write_export(&reader, &args.destination)?;

    if args.json {
        document(&args.destination, &written).emit();
        return Ok(());
    }

    // The notice first and the counts after it, so a user who reads one line
    // reads the one that matters.
    for line in NOTICE.lines() {
        println!("{line}");
    }
    println!();
    println!("wrote          {}", args.destination.display());
    println!("sessions       {}", written.sessions);
    println!(
        "  evicted      {} (written as empty files, no body to write)",
        written.evicted
    );
    println!("turns          {}", written.turns);
    println!(
        "bytes          {} byte(s) ({})",
        written.bytes,
        human(written.bytes)
    );
    println!(
        "manifest       {}",
        args.destination.join(MANIFEST).display()
    );
    Ok(())
}

/// What one export put on disk.
#[derive(Debug, Default)]
struct Written {
    sessions: i64,
    turns: i64,
    evicted: i64,
    bytes: u64,
}

/// A destination that already holds anything is refused, not merged into.
///
/// Stricter than "already holds an export" on purpose. Recognizing an export by
/// its manifest alone would let a second run overwrite same-named files in a
/// directory this command did not create - and a `.jsonl` a user put there
/// themselves is exactly the file that must not be silently replaced. Naming
/// the manifest when it is there is what tells the two cases apart for the
/// reader.
fn refuse_if_occupied(destination: &Path) -> Result<(), Failure> {
    let Ok(mut entries) = std::fs::read_dir(destination) else {
        // Missing is the ordinary case, and anything else - a file where the
        // directory should be, a permission error - surfaces from the create
        // below with the operating system's own words.
        return Ok(());
    };
    if entries.next().is_none() {
        return Ok(());
    }
    let already = destination.join(MANIFEST).exists();
    Err(Failure::Operational(format!(
        "{} {}; nothing was written. export writes a whole directory and never merges into one, \
         so name a directory that does not exist yet",
        destination.display(),
        match already {
            true => format!("already holds an export ({MANIFEST} is there)"),
            false => "is not empty".to_owned(),
        }
    )))
}

fn write_export(reader: &Reader, destination: &Path) -> Result<Written, Failure> {
    let conn = reader.store().conn();
    let sessions = visible::sessions(conn, reader.config()).map_err(op)?;

    // Owner-only, and the leaf alone (D-04, D-11). An export is a directory of
    // plaintext transcripts, so it is verbatim's own output even though the
    // user named the path - the mode rides the `mkdir` rather than a `chmod`
    // after it, and there is no instant at which the directory about to hold
    // every prompt anyone typed is readable by the rest of the machine. A
    // destination that already exists and is empty is accepted by
    // `refuse_if_occupied` above and keeps the mode its creator gave it: this
    // command narrows what it creates and nothing it merely found.
    verbatim_core::owner_only::create_dir_all(destination).map_err(|e| {
        Failure::Operational(format!(
            "{} could not be created: {e}",
            destination.display()
        ))
    })?;

    let mut written = Written::default();
    let mut files: Vec<Value> = Vec::new();
    let mut projects: BTreeSet<Option<String>> = BTreeSet::new();
    let mut first: Option<String> = None;
    let mut last: Option<String> = None;

    for session in &sessions {
        let row: (Vec<u8>, i64, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT s.blob, coalesce(m.is_evicted, 0) <> 0, m.session_id,
                        m.first_turn_at, m.last_turn_at
                   FROM sessions s LEFT JOIN session_meta m USING (session_key)
                  WHERE s.session_key = ?1",
                [&session.session_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(|e| Failure::Operational(format!("{}: {e}", session.session_key)))?;
        let (bytes, evicted, session_id, session_first, session_last) = row;

        // An evicted session's blob is `x''` and `blob::read_all` cannot parse
        // a header out of nothing (RET-02, D-03). Its file is written empty
        // rather than skipped: the manifest names it either way, and a missing
        // file would read as an export that lost something.
        let stream = match evicted != 0 {
            true => Vec::new(),
            false => blob::read_all(&bytes).map_err(|e| {
                Failure::Operational(format!(
                    "{} could not be read out of the archive: {e}",
                    session.session_key
                ))
            })?,
        };
        let name = file_name(session.session_no, &session.session_key);
        write_owner_only(&destination.join(&name), &stream)
            .map_err(|e| Failure::Operational(format!("{name} could not be written: {e}")))?;

        written.sessions += 1;
        written.turns += session.turns;
        written.evicted += i64::from(evicted != 0);
        written.bytes += stream.len() as u64;
        projects.insert(session.project.clone());
        extend(&mut first, &session_first, |a, b| b < a);
        extend(&mut last, &session_last, |a, b| b > a);
        files.push(json!({
            "file": name,
            "session_key": session.session_key,
            "session_id": session_id,
            "project": session.project,
            "turns": session.turns,
            "bytes": stream.len(),
            "evicted": evicted != 0,
            "first_turn_at": session_first,
            "last_turn_at": session_last,
        }));
    }

    // The archive format the STORE reports, not the constant this build was
    // compiled with. They are the same on any store this binary can open, and
    // the manifest is read by whatever opens it later, for which the store's
    // own answer is the honest one.
    let archive_format = reader
        .store()
        .meta_int(META_ARCHIVE_FORMAT)
        .map_err(op)?
        .unwrap_or(ARCHIVE_FORMAT);

    let manifest = json!({
        "notice": NOTICE,
        "archive_format": archive_format,
        "sessions": written.sessions,
        "turns": written.turns,
        "evicted": written.evicted,
        "bytes": written.bytes,
        "projects": projects.into_iter().collect::<Vec<_>>(),
        "first_turn_at": first,
        "last_turn_at": last,
        "files": files,
    });
    let path = destination.join(MANIFEST);
    // Pretty-printed, unlike every other JSON this binary writes. The `--json`
    // envelope is one line because it is piped; a manifest is opened and read.
    let text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Failure::Operational(format!("the manifest could not be built: {e}")))?;
    write_owner_only(&path, format!("{text}\n").as_bytes()).map_err(|e| {
        Failure::Operational(format!("{} could not be written: {e}", path.display()))
    })?;
    Ok(written)
}

/// One export file, created 0600 and never overwritten.
///
/// `create_new` rather than the truncating create `std::fs::write` performs:
/// [`refuse_if_occupied`] has already established that the destination was
/// empty or absent, so a name that is already taken by the time this runs is
/// something else writing into the directory mid-export, and truncating that
/// file would be the one thing this command says it never does. The mode is
/// carried by the open itself (D-01), so the file holding a whole session's
/// unredacted transcript is never on disk at the umask's width, not even for
/// the moment between the create and a chmod.
fn write_owner_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = verbatim_core::owner_only::options()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)
}

/// Widen a date range by one session's bound.
fn extend(bound: &mut Option<String>, candidate: &Option<String>, wider: fn(&str, &str) -> bool) {
    let Some(candidate) = candidate else { return };
    match bound {
        Some(current) if !wider(current, candidate) => {}
        _ => *bound = Some(candidate.clone()),
    }
}

/// The file one session is written to.
///
/// `session_no` first, so two sidecars that share the name `agent-alpha.jsonl`
/// under different parents cannot collide - the number is unique per store by
/// declaration. The transcript's own name follows it because that is what makes
/// a directory listing recognizable, reduced to the characters a filename may
/// safely carry on all three platforms: the source is a path off the user's
/// filesystem, and nothing about a name that arrived from outside gets to
/// decide where a byte lands.
fn file_name(session_no: i64, session_key: &str) -> String {
    let stem: String = Path::new(session_key)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(
            |c| match c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                true => c,
                false => '_',
            },
        )
        .collect();
    let stem = stem.trim_start_matches('.').trim_end_matches(".jsonl");
    match stem.is_empty() {
        true => format!("{session_no:06}.jsonl"),
        false => format!("{session_no:06}-{stem}.jsonl"),
    }
}

/// The envelope for this command.
fn document(destination: &Path, written: &Written) -> Document {
    Document::new(COMMAND)
        .field("destination", destination.display().to_string())
        .field("manifest", destination.join(MANIFEST).display().to_string())
        .field("sessions", written.sessions)
        .field("turns", written.turns)
        .field("evicted", written.evicted)
        .field("bytes", written.bytes)
        // In the document as well as in the manifest, because a caller that
        // scripts this never sees the terminal output and is exactly the caller
        // most likely to be writing somewhere shared.
        .field("notice", NOTICE)
}

fn op(error: verbatim_core::Error) -> Failure {
    Failure::Operational(error.to_string())
}

/// One positional destination and `--json`, so the loop rather than
/// `super::json_flag`.
///
/// A missing destination is misuse and not a default. There is no sensible
/// place to put an unredacted copy of every transcript that a user did not
/// name.
pub fn parse(parser: &mut lexopt::Parser) -> Result<Args, Failure> {
    use lexopt::prelude::*;

    let mut destination: Option<PathBuf> = None;
    let mut json = false;
    while let Some(arg) = parser.next().map_err(|e| Failure::Misuse(e.to_string()))? {
        match arg {
            Long(super::JSON_FLAG) => json = true,
            Value(value) if destination.is_none() => destination = Some(PathBuf::from(value)),
            other => return Err(Failure::Misuse(crate::unexpected(other))),
        }
    }
    let Some(destination) = destination else {
        return Err(Failure::Misuse(
            "export needs a destination directory: verbatim export <dir>".to_owned(),
        ));
    };
    Ok(Args { destination, json })
}
