//! RCL-02 and RCL-03: the exact-match entities a structured tool record leaves
//! behind.
//!
//! **Where they come from (D-02).** One record's own bytes and nothing else:
//! the `tool_use` and `tool_result` blocks of `message.content`, and the
//! top-level `toolUseResult` object. There is no join between a call and its
//! result - that would need a `tool_use_id` index and a second pass, and would
//! break the one-record-in / one-row-out shape that gives ingest and rebuild a
//! single seam. Nothing is read from prose, and nothing from `attachment`, even
//! though the text projection reads that one: D-02 names three subtrees.
//!
//! **What that buys.** An assistant sentence mentioning `src/main.rs` is a
//! guess about what happened; a `Read` whose `file_path` is `src/main.rs` is a
//! record of it. Exact-match recall over the second is worth something and over
//! the first is a keyword search that already exists in `turns_fts`.
//!
//! Nothing here rejects a value for being common (RCL-04). "Too common" changes
//! as the corpus grows and an index-time rejection is irreversible; commonness
//! is a query-time weighting problem.

use serde_json::Value;

use super::expand::case_components;

/// A file or directory named by a structured tool field.
pub const PATH: &str = "path";
/// The program a `Bash` call ran.
pub const COMMAND: &str = "command";
/// A failure's normalized text (D-03, D-04).
pub const ERROR: &str = "error";
/// An identifier-shaped token out of a search pattern or an edit.
pub const SYMBOL: &str = "symbol";
/// The tool a record called.
pub const TOOL: &str = "tool";

/// Every kind, for a test or a report that needs the closed set.
pub const KINDS: [&str; 5] = [PATH, COMMAND, ERROR, SYMBOL, TOOL];

/// One extracted entity, before it becomes an `entities` row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entity {
    pub kind: &'static str,
    pub value: String,
}

/// The `tool_use` input keys that name a file outright.
const PATH_INPUTS: [&str; 3] = ["file_path", "path", "notebook_path"];

/// Every entity one turn record carries, deduplicated on `(kind, value)` and in
/// the order the record walk produces.
///
/// The order is the whole reason a rebuild reproduces the same rows: it depends
/// on the record's bytes and on nothing about the store, so re-deriving the same
/// record twice writes the same list twice.
pub fn extract(record: &Value) -> Vec<Entity> {
    let mut out = Collector::default();

    if let Some(blocks) = record
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => out.tool_use(block),
                Some("tool_result") => out.tool_result(block),
                _ => {}
            }
        }
    }

    if let Some(result) = record.get("toolUseResult") {
        out.tool_use_result(result);
    }

    out.entities
}

#[derive(Default)]
struct Collector {
    entities: Vec<Entity>,
}

impl Collector {
    fn push(&mut self, kind: &'static str, value: impl Into<String>) {
        let value = value.into();
        if value.is_empty() {
            return;
        }
        let entity = Entity { kind, value };
        if !self.entities.contains(&entity) {
            self.entities.push(entity);
        }
    }

    /// One `tool_use` block: the tool itself, then whatever its inputs name.
    fn tool_use(&mut self, block: &Value) {
        let Some(name) = block.get("name").and_then(Value::as_str) else {
            return;
        };
        // The same value `parse::record::tool_name` lifts into `turns.tool_name`
        // (D-16 measured at most one `tool_use` block per record), so a
        // `kind = 'tool'` filter and the column cannot disagree.
        self.push(TOOL, name);

        let Some(input) = block.get("input") else {
            return;
        };

        for key in PATH_INPUTS {
            if let Some(raw) = input.get(key).and_then(Value::as_str) {
                if let Some(path) = normalize_path(raw) {
                    self.push(PATH, path);
                }
            }
        }

        match name {
            "Bash" => {
                if let Some(command) = input.get("command").and_then(Value::as_str) {
                    if let Some(program) = program_of(command) {
                        self.push(COMMAND, program);
                    }
                    for word in path_words(command) {
                        self.push(PATH, word);
                    }
                }
            }
            // A search pattern is what the user was looking FOR, which is the
            // one free-text field whose contents are reliably identifiers.
            "Grep" => self.symbols(input.get("pattern")),
            // Both sides of an edit: the symbol that went and the one that came.
            "Edit" => {
                self.symbols(input.get("old_string"));
                self.symbols(input.get("new_string"));
            }
            _ => {}
        }
    }

    /// The identifier-shaped tokens of a string field, and nothing else.
    ///
    /// `old_string` and a `Grep` `pattern` can both hold prose, so the shape
    /// test is what keeps RCL-02's "not from prose" true for a field that is not
    /// structurally guaranteed to hold a symbol. An internal case or underscore
    /// boundary admits `SearchManager` and `search_manager` and refuses every
    /// ordinary English word.
    fn symbols(&mut self, field: Option<&Value>) {
        let Some(text) = field.and_then(Value::as_str) else {
            return;
        };
        for token in identifier_tokens(text) {
            if is_identifier_shaped(token) {
                self.push(SYMBOL, token);
            }
        }
    }

    /// One `tool_result` block. `error` is D-03's, and lands in the next task.
    fn tool_result(&mut self, _block: &Value) {}

    /// The top-level `toolUseResult`. `filePath` is the one path key it carries
    /// (679 of the measured sample); `stderr` is D-03's and lands next.
    fn tool_use_result(&mut self, result: &Value) {
        if let Some(raw) = result.get("filePath").and_then(Value::as_str) {
            if let Some(path) = normalize_path(raw) {
                self.push(PATH, path);
            }
        }
    }
}

/// A path as the record wrote it, minus surrounding quotes and a trailing
/// `:line:col`.
///
/// Never canonicalized against the filesystem. 52% of the real corpus's `cwd`
/// directories no longer exist (phase 2 D-05), so a path that resolved today
/// would normalize differently tomorrow and the same file would key two ways
/// across one archive.
pub fn normalize_path(raw: &str) -> Option<String> {
    let trimmed = trim_quotes(raw.trim());
    let stripped = strip_line_col(trimmed);
    (!stripped.is_empty()).then(|| stripped.to_owned())
}

/// The program a shell command ran: the first argv word that is not a
/// `NAME=value` assignment, reduced to its basename.
///
/// The basename is what makes the entity worth having. `cargo`, `/usr/bin/cargo`
/// and `~/.cargo/bin/cargo` are one command to anybody searching for what they
/// ran, and three values to anybody who kept the spelling.
pub fn program_of(command: &str) -> Option<String> {
    for word in command.split_whitespace() {
        let word = trim_quotes(word);
        if word.is_empty() {
            continue;
        }
        // `FOO=bar cmd`: an assignment prefix, not the program. Tested before
        // the path split, because the value may itself contain a separator.
        if is_assignment(word) {
            continue;
        }
        let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
        return (!base.is_empty()).then(|| base.to_owned());
    }
    None
}

/// The path-shaped words of a shell command line.
///
/// Path-shaped means "carries a separator": a bare word in argv is as likely to
/// be a subcommand or a flag value as a file, and guessing wrong fills the
/// exact-match table with words that are not paths, which is the one thing that
/// table is for. A leading `-` is a flag whatever follows it.
pub fn path_words(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(trim_quotes)
        .filter(|word| !word.starts_with('-') && !is_assignment(word))
        .filter(|word| word.contains('/') || word.contains('\\'))
        .filter_map(normalize_path)
        .collect()
}

/// Is this token shaped like an identifier rather than like a word?
///
/// An internal case boundary (`searchManager`, `SearchManager`) or an internal
/// underscore with content on both sides (`search_manager`). `Search` alone is
/// not one, and neither is `the`.
pub fn is_identifier_shaped(token: &str) -> bool {
    if case_components(token).len() > 1 {
        return true;
    }
    let mut parts = token.split('_');
    let first = parts.next().unwrap_or_default();
    !first.is_empty() && parts.clone().count() > 0 && parts.all(|part| !part.is_empty())
}

/// Maximal runs of identifier characters. Distinct from
/// [`super::expand::separator_components`] on exactly one point: `_` is kept, so
/// `search_manager` survives to be judged as one token.
pub fn identifier_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
}

fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

fn trim_quotes(word: &str) -> &str {
    let bytes = word.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        return &word[1..word.len() - 1];
    }
    word
}

/// Drop a trailing `:line` or `:line:col`, which names a position inside a file
/// rather than a different file.
fn strip_line_col(path: &str) -> &str {
    let once = strip_trailing_number(path);
    strip_trailing_number(once)
}

fn strip_trailing_number(path: &str) -> &str {
    match path.rsplit_once(':') {
        Some((head, tail))
            if !head.is_empty() && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => path,
    }
}
