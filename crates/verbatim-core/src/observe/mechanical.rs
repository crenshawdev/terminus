//! OBS-01: what one session did, read off the parser and never off a model.
//!
//! Every fact here is either a SQL query over tables ingest already wrote or a
//! value lifted straight out of the session's own bytes. **No model is called
//! and no network connection is opened on any path in this module** - that is
//! the whole point of splitting the mechanical half out: a machine with
//! judgment disabled, or with no network at all, still gets an auditable
//! account of every finalized session.
//!
//! **Where each fact comes from (D-04).** `paths` gives the files and
//! `turns.tool_name` says whether the turn that named one was writing it;
//! `entities` of kind `tool` and `error` give the tools and the failures;
//! `session_meta` gives the branch and the two timestamps the duration is the
//! difference of; `turns` gives the count; `compaction_boundaries` joined
//! through `turns` gives the compactions.
//!
//! **The two exceptions, and why they cost a blob read.** Commands-with-
//! arguments and commits cannot come out of SQL:
//! [`crate::index::entity::program_of`] reduces a Bash command to its basename,
//! so `entities` holds `git` with the subcommand and every argument discarded,
//! and [`crate::parse::Record`] parses `gitBranch` and no commit field at all.
//! The rejected alternative was widening the `entities` command normalization,
//! which is a reindex-visible change to a shipped table. So the session stream
//! is decompressed once, rescanned, and both facts are taken off the same walk
//! - one blob read per session, never two.
//!
//! **Everything is bounded.** One pathological session must not write a
//! megabyte of JSON into a row that `verbatim observations` then prints on
//! every run, so each list is capped at [`MAX_LIST`] entries and each value at
//! [`MAX_VALUE_BYTES`], and the stored document says which lists were cut.

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::error::Result;

/// How many entries one fact list may carry.
///
/// A bound on rows and never a judgement about which values matter: the cut is
/// taken in the list's own stable order (sorted for the SQL facts, byte order
/// for the two that come off the blob), so the same session computes the same
/// document twice. 64 leaves the ordinary session untouched and binds the tail
/// that would otherwise be one entry per file of a `git status --porcelain`.
pub const MAX_LIST: usize = 64;

/// How many bytes one fact value may carry.
///
/// The same number [`crate::index::entity::MAX_ERROR_BYTES`] uses, for the same
/// reason: the head of a value is the part worth reading, and a `git commit`
/// carrying a forty-line body in its `-m` argument is one value that will never
/// recur. The whole of it stays in the blob either way.
pub const MAX_VALUE_BYTES: usize = 512;

/// What a value cut at [`MAX_VALUE_BYTES`] carries in place of its tail.
///
/// Explicit rather than a silent slice: a reader has to be able to tell a
/// command that ended from a command that was cut.
pub const ELISION: &str = " [...]";

/// The tools that CHANGE a file rather than read one.
///
/// A closed list, and short on purpose. Everything not named here is treated as
/// a read, so a tool added upstream shows its paths under `files_read` - which
/// is the safe direction to be wrong in: it understates what a session altered
/// rather than inventing an edit that never happened.
const WRITING_TOOLS: [&str; 4] = ["Edit", "MultiEdit", "Write", "NotebookEdit"];

/// The OBS-01 fact set for one session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mechanical {
    pub files_read: Vec<String>,
    pub files_modified: Vec<String>,
    pub tools: Vec<String>,
    /// Whole command lines, arguments included - not the basenames `entities`
    /// holds.
    pub commands: Vec<String>,
    pub errors: Vec<String>,
    /// The subject of every commit this session's own commands made.
    pub commits: Vec<String>,
    pub branch: Option<String>,
    pub turns: i64,
    pub first_turn_at: Option<String>,
    pub last_turn_at: Option<String>,
    /// Wall-clock seconds between the first and the last turn. `None` when the
    /// session carries no timestamp to measure from.
    pub duration_seconds: Option<i64>,
    pub compactions: i64,
    /// The lists a cap cut, by field name, in declaration order.
    pub truncated: Vec<&'static str>,
}

impl Mechanical {
    /// The stored document.
    ///
    /// One JSON object rather than a column per fact, the way `decisions`
    /// carries its list-shaped fields: nothing joins on any of this, and the
    /// row is read back whole by `verbatim observations`.
    pub fn to_json(&self) -> Value {
        json!({
            "files_read": self.files_read,
            "files_modified": self.files_modified,
            "tools": self.tools,
            "commands": self.commands,
            "errors": self.errors,
            "commits": self.commits,
            "branch": self.branch,
            "turns": self.turns,
            "first_turn_at": self.first_turn_at,
            "last_turn_at": self.last_turn_at,
            "duration_seconds": self.duration_seconds,
            "compactions": self.compactions,
            "truncated": self.truncated,
        })
    }
}

/// Compute the OBS-01 facts for one archived session.
///
/// The connection is the only input beside the key: this reads and never
/// writes, so a caller may run it inside its own transaction or outside one.
pub fn observe(conn: &Connection, session_key: &str) -> Result<Mechanical> {
    let (files_read, files_modified) = files(conn, session_key)?;
    let tools = entity_values(conn, session_key, crate::index::entity::TOOL)?;
    let errors = entity_values(conn, session_key, crate::index::entity::ERROR)?;
    let (commands, commits) = from_blob(conn, session_key)?;

    // One row, and every column of it read at once: `session_meta` is keyed on
    // the same value and the duration is a difference of two of its columns, so
    // computing it in SQL rather than in Rust keeps the one date format this
    // store uses in the one place that already parses it. A session with no
    // meta row - `verbatim verify`'s damage case - answers `None` on all four
    // rather than failing the whole observation.
    let meta = conn
        .query_row(
            "SELECT branch, first_turn_at, last_turn_at,
                    CAST(strftime('%s', last_turn_at) AS INTEGER)
                  - CAST(strftime('%s', first_turn_at) AS INTEGER)
               FROM session_meta WHERE session_key = ?1",
            [session_key],
            |r| {
                Ok(Meta {
                    branch: r.get(0)?,
                    first_turn_at: r.get(1)?,
                    last_turn_at: r.get(2)?,
                    duration_seconds: r.get(3)?,
                })
            },
        )
        .unwrap_or_default();

    let turns = conn.query_row(
        "SELECT count(*) FROM turns WHERE session_key = ?1",
        [session_key],
        |r| r.get(0),
    )?;
    let compactions = conn.query_row(
        "SELECT count(*) FROM compaction_boundaries c
           JOIN turns t ON t.id = c.turn_id
          WHERE t.session_key = ?1",
        [session_key],
        |r| r.get(0),
    )?;

    let mut truncated = Vec::new();
    Ok(Mechanical {
        files_read: cap("files_read", files_read, &mut truncated),
        files_modified: cap("files_modified", files_modified, &mut truncated),
        tools: cap("tools", tools, &mut truncated),
        commands: cap("commands", commands, &mut truncated),
        errors: cap("errors", errors, &mut truncated),
        commits: cap("commits", commits, &mut truncated),
        branch: meta.branch,
        turns,
        first_turn_at: meta.first_turn_at,
        last_turn_at: meta.last_turn_at,
        duration_seconds: meta.duration_seconds,
        compactions,
        truncated,
    })
}

/// The four facts one `session_meta` row answers for.
///
/// A named struct rather than a tuple because the read is four `Option`s of
/// three different types and a tuple of those is unreadable at both ends.
#[derive(Debug, Default)]
struct Meta {
    branch: Option<String>,
    first_turn_at: Option<String>,
    last_turn_at: Option<String>,
    duration_seconds: Option<i64>,
}

/// Cut one list to [`MAX_LIST`] and record that it was cut.
///
/// The note is not optional, which is the failure this shape exists to prevent:
/// a document quietly holding 64 of 900 files would read as a complete account
/// of the session.
fn cap(
    field: &'static str,
    mut values: Vec<String>,
    truncated: &mut Vec<&'static str>,
) -> Vec<String> {
    if values.len() > MAX_LIST {
        values.truncate(MAX_LIST);
        truncated.push(field);
    }
    values
}

/// The files this session read and the files it changed.
///
/// A file both read and written appears in both lists. Two lists of paths is
/// what OBS-01 asks for, and "modified" has to mean that a tool which writes
/// actually named it - so a path an `Edit` and a `Read` both touched is true on
/// both counts rather than sorted into one.
fn files(conn: &Connection, session_key: &str) -> Result<(Vec<String>, Vec<String>)> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT p.path, coalesce(t.tool_name, '')
           FROM paths p JOIN turns t ON t.id = p.turn_id
          WHERE t.session_key = ?1
          ORDER BY p.path",
    )?;
    let rows = statement.query_map([session_key], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;

    let (mut read, mut modified) = (Vec::new(), Vec::new());
    for row in rows {
        let (path, tool) = row?;
        let path = clip(&path);
        let into = if WRITING_TOOLS.contains(&tool.as_str()) {
            &mut modified
        } else {
            &mut read
        };
        if !into.contains(&path) {
            into.push(path);
        }
    }
    Ok((read, modified))
}

/// Every distinct entity value of one kind this session's turns left behind.
fn entity_values(conn: &Connection, session_key: &str, kind: &str) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT e.value_norm
           FROM entities e JOIN turns t ON t.id = e.turn_id
          WHERE t.session_key = ?1 AND e.kind = ?2
          ORDER BY e.value_norm",
    )?;
    let rows = statement.query_map(rusqlite::params![session_key, kind], |r| {
        r.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(clip(&row?));
    }
    Ok(out)
}

/// The two facts SQL cannot answer, off one decompression of the session.
///
/// Byte order, deduplicated, and nothing is read from prose: only the `input`
/// of a `Bash` `tool_use` block, which is a record of what ran rather than a
/// sentence about it.
fn from_blob(conn: &Connection, session_key: &str) -> Result<(Vec<String>, Vec<String>)> {
    let bytes: Vec<u8> = conn.query_row(
        "SELECT blob FROM sessions WHERE session_key = ?1",
        [session_key],
        |r| r.get(0),
    )?;
    let stream = crate::blob::read_all(&bytes)?;
    let scan = crate::parse::scan(&stream);

    let mut commands: Vec<String> = Vec::new();
    let mut commits: Vec<String> = Vec::new();
    for (record, _) in scan.turns() {
        let from = record.offset as usize;
        let line = &stream[from..from + record.len as usize];
        // A line that will not parse contributes nothing, exactly as it
        // contributes nothing to the derived tables. The blob keeps it either
        // way, which is what "the transcript is the record" is worth here.
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        for command in bash_commands(&value) {
            if let Some(subject) = commit_subject(command) {
                let subject = clip(&subject);
                if !commits.contains(&subject) {
                    commits.push(subject);
                }
            }
            let command = clip(command);
            if !commands.contains(&command) {
                commands.push(command);
            }
        }
    }
    Ok((commands, commits))
}

/// The command line of every `Bash` call in one record.
///
/// `message.content` and nothing else - the same subtree
/// [`crate::index::entity::extract`] reads, so a command reaching this list and
/// the `command` entity of the same turn cannot have come from different
/// places.
fn bash_commands(record: &Value) -> Vec<&str> {
    record
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter(|block| block.get("name").and_then(Value::as_str) == Some("Bash"))
        .filter_map(|block| {
            block
                .get("input")
                .and_then(|i| i.get("command"))
                .and_then(Value::as_str)
        })
        .collect()
}

/// The subject of the commit a command line made, when it made one.
///
/// Shell-aware only as far as it has to be: words are split on whitespace with
/// quoted runs held together, because `git commit -m "wire the drain up"` is
/// four words to a shell and six to `split_whitespace`.
///
/// The heredoc spelling gets an arm of its own because it is the one Claude
/// Code itself writes: `git commit -m "$(cat <<'EOF'\n<subject>\n...\nEOF\n)"`.
/// The `-m` argument there is the substitution rather than the message, so the
/// subject is the first non-empty line of the body - which is exactly what git
/// takes as the subject too.
///
/// `None` for every command that is not a commit, which is nearly all of them.
pub fn commit_subject(command: &str) -> Option<String> {
    let words = shell_words(command);
    let program = words.first()?;
    if program.rsplit(['/', '\\']).next().unwrap_or(program) != "git" {
        return None;
    }
    // Scanned rather than tested at word 1: `git -C /repo commit` is ordinary.
    let at = words.iter().position(|word| word == "commit")?;

    let mut rest = words[at + 1..].iter();
    let mut message: Option<&str> = None;
    while let Some(word) = rest.next() {
        if let Some(value) = word.strip_prefix("--message=") {
            message = Some(value);
            break;
        }
        if word == "-m" || word == "--message" {
            message = rest.next().map(String::as_str);
            break;
        }
    }
    let message = message.filter(|m| !m.is_empty())?;

    if message.starts_with("$(") {
        return first_body_line(command);
    }
    Some(message.trim().to_owned())
}

/// The first non-empty line of a heredoc body in a command line.
fn first_body_line(command: &str) -> Option<String> {
    // Past `<<`, past the delimiter word, and onto the body.
    let (_, body) = command.split_once("<<")?;
    let (_, body) = body.split_once('\n')?;
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

/// A command line split into words, with single- and double-quoted runs held
/// together and their quotes removed.
///
/// Not a shell parser and never one: no expansion, no escaping rules, no
/// operator handling. It exists so `-m "a subject"` yields one word, which is
/// the only thing [`commit_subject`] needs of it.
fn shell_words(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut quoted = false;

    for ch in command.chars() {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => word.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                quoted = true;
            }
            None if ch.is_whitespace() => {
                if quoted || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                    quoted = false;
                }
            }
            None => word.push(ch),
        }
    }
    if quoted || !word.is_empty() {
        out.push(word);
    }
    out
}

/// One value cut to [`MAX_VALUE_BYTES`], on a character boundary, with the cut
/// said out loud.
///
/// The boundary is not a nicety: slicing a multi-byte character in half panics,
/// and one command line carrying one is enough to take the observation of a
/// whole session with it.
fn clip(value: &str) -> String {
    if value.len() <= MAX_VALUE_BYTES {
        return value.to_owned();
    }
    let mut at = MAX_VALUE_BYTES;
    while at > 0 && !value.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}{ELISION}", &value[..at])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_argument_stays_one_word() {
        assert_eq!(
            shell_words("git commit -m \"wire the drain up\""),
            vec!["git", "commit", "-m", "wire the drain up"]
        );
        assert_eq!(shell_words("  "), Vec::<String>::new());
        // An empty quoted string is a word, and it is one `split_whitespace`
        // loses outright.
        assert_eq!(shell_words("echo ''"), vec!["echo", ""]);
    }

    #[test]
    fn a_commit_subject_comes_off_every_spelling_that_carries_one() {
        assert_eq!(
            commit_subject("git commit -m \"wire the drain up\"").as_deref(),
            Some("wire the drain up")
        );
        assert_eq!(
            commit_subject("git commit --message='fix the gate'").as_deref(),
            Some("fix the gate")
        );
        assert_eq!(
            commit_subject("git -C /repo commit -a -m 'and again'").as_deref(),
            Some("and again")
        );
        assert_eq!(
            commit_subject("/usr/bin/git commit -m subject").as_deref(),
            Some("subject")
        );
    }

    /// The spelling Claude Code itself writes. Without this arm the stored
    /// subject would be `$(cat <<'EOF'`, which names nothing.
    #[test]
    fn the_heredoc_spelling_yields_the_subject_and_not_the_substitution() {
        let command = "git commit -m \"$(cat <<'EOF'\nfeat(7-1): the subject\n\nthe body\nEOF\n)\"";
        assert_eq!(
            commit_subject(command).as_deref(),
            Some("feat(7-1): the subject")
        );
    }

    #[test]
    fn a_command_that_is_not_a_commit_yields_nothing() {
        for command in [
            "cargo test -p verbatim-core",
            "git status --porcelain",
            "git log -m --oneline",
            "echo git commit -m nope",
            "",
        ] {
            assert_eq!(commit_subject(command), None, "{command}");
        }
    }

    /// A cut value says it was cut, and a multi-byte character across the cut
    /// is a shorter string rather than a panic.
    #[test]
    fn a_long_value_is_clipped_on_a_character_boundary_and_says_so() {
        let short = "cargo test";
        assert_eq!(clip(short), short);

        let long = "é".repeat(MAX_VALUE_BYTES);
        assert!(long.len() > MAX_VALUE_BYTES, "the premise");
        let clipped = clip(&long);
        assert!(clipped.ends_with(ELISION), "{clipped}");
        assert!(clipped.len() <= MAX_VALUE_BYTES + ELISION.len());
    }

    /// A capped list says which field was cut, and an uncapped one adds
    /// nothing.
    #[test]
    fn a_capped_list_names_itself_in_the_document() {
        let mut truncated = Vec::new();
        let short = cap("tools", vec!["Read".into()], &mut truncated);
        assert_eq!(short.len(), 1);
        assert!(truncated.is_empty());

        let many: Vec<String> = (0..MAX_LIST + 5).map(|n| n.to_string()).collect();
        let cut = cap("commands", many, &mut truncated);
        assert_eq!(cut.len(), MAX_LIST);
        assert_eq!(truncated, vec!["commands"]);
    }
}
