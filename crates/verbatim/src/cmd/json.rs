//! The one document every `--json` data command writes to stdout (RCL-06).
//!
//! **One envelope, six commands.** `search`, `show`, `sessions`, `status`,
//! `verify` and `reindex` all emit `{command, ok, reason, data}`, so a caller
//! can tell success from an empty result without parsing prose and without a
//! per-command decoder for the outcome. `ok` is exactly the exit code's answer -
//! true for 0, false for 1 - and `reason` is why a result is empty or a command
//! failed, present only when there is one. Everything the command actually
//! found sits under `data`, which is the only part whose shape differs between
//! commands.
//!
//! `reason` is a rendered string rather than a code. The library-side reasons
//! ([`verbatim_core::recall::Reason`]) are already values a caller can match on
//! in-process; across the CLI boundary the consumer is a human reading a
//! terminal or a script checking `ok`, and a second vocabulary of reason codes
//! would be a contract to keep stable for nobody.
//!
//! **Never hand-formatted (D-25).** Every value is built as a `serde_json`
//! value and serialized by `serde_json`. An excerpt is arbitrary text cut out of
//! a transcript - it carries quotes, backslashes, and control bytes - and a
//! `format!`-assembled document breaks on exactly the turns most worth reading,
//! while validating fine on the ones that are not.

use serde_json::{Map, Value};

/// One command's `--json` output.
///
/// Built by chaining, emitted once. Nothing is written until [`Document::emit`]
/// is called, so a command that fails half way through composing its answer has
/// not already put a partial document on stdout.
#[derive(Debug, Clone)]
pub struct Document {
    command: &'static str,
    ok: bool,
    reason: Option<String>,
    data: Map<String, Value>,
}

impl Document {
    /// A successful document for `command`, with no data in it yet.
    pub fn new(command: &'static str) -> Document {
        Document {
            command,
            ok: true,
            reason: None,
            data: Map::new(),
        }
    }

    /// Mark the command as having failed: `ok` is false and the process exits 1.
    ///
    /// The document is still written. A caller that parses it and a caller that
    /// checks the exit code must not disagree about whether the store is whole,
    /// which is why `verify` emits its failures *and* exits non-zero.
    pub fn failed(mut self) -> Document {
        self.ok = false;
        self
    }

    /// Why this result is empty, or why the command failed.
    pub fn because(mut self, reason: impl std::fmt::Display) -> Document {
        self.reason = Some(reason.to_string());
        self
    }

    /// One field of this command's own data.
    pub fn field(mut self, name: &str, value: impl Into<Value>) -> Document {
        self.data.insert(name.to_owned(), value.into());
        self
    }

    /// The whole document as a value.
    pub fn to_value(&self) -> Value {
        let mut out = Map::new();
        out.insert("command".into(), Value::from(self.command));
        out.insert("ok".into(), Value::from(self.ok));
        // Present and null rather than absent: a consumer reads one shape
        // whichever way the command went, and `doc["reason"]` is never a missing
        // key it has to handle separately from a null one.
        out.insert(
            "reason".into(),
            match &self.reason {
                Some(reason) => Value::from(reason.as_str()),
                None => Value::Null,
            },
        );
        out.insert("data".into(), Value::Object(self.data.clone()));
        Value::Object(out)
    }

    /// The exact bytes [`Document::emit`] writes, without the trailing newline.
    ///
    /// Split out so a test asserts on the same string the process prints rather
    /// than on a second serialization of the same value.
    pub fn render(&self) -> String {
        // Cannot fail: every value in the map came from `serde_json` and none of
        // them is a map with non-string keys or a non-finite float. Rendered
        // through the error arm anyway, because the alternative is an `unwrap`
        // in the one code path whose whole job is not to corrupt its output.
        serde_json::to_string(&self.to_value())
            .unwrap_or_else(|e| format!(r#"{{"command":"{}","ok":false,"reason":"the response could not be serialized: {e}","data":{{}}}}"#, self.command))
    }

    /// Write the document to stdout, and nothing else, ever.
    ///
    /// One line. Diagnostics go to stderr on every path, which is what lets
    /// `verbatim search --json | jq` work while a warning is still printed.
    pub fn emit(&self) {
        println!("{}", self.render());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-25's whole point: a value carrying the characters JSON has to escape
    /// survives the round trip byte for byte.
    ///
    /// A transcript excerpt is arbitrary text - a shell command with quoted
    /// arguments, a Windows path, a terminal control byte out of a tool result -
    /// and a `format!`-assembled document would produce output that fails its own
    /// shape test for exactly those turns.
    #[test]
    fn a_hostile_string_round_trips_through_the_emitter() {
        let hostile = "a \"quoted\" \\ backslash \u{7} bell \u{0}nul \n newline \u{2028} sep é";

        let rendered = Document::new("search")
            .field("excerpt", hostile)
            .because(hostile)
            .render();

        // The wire form carries no raw control byte at all: that is what makes
        // it one line on a terminal and one string to a parser.
        assert!(
            !rendered.contains('\n') && !rendered.contains('\u{7}') && !rendered.contains('\u{0}'),
            "the rendered document carries a raw control byte: {rendered:?}"
        );

        let back: Value = serde_json::from_str(&rendered).expect("the emitter writes valid JSON");
        assert_eq!(back["data"]["excerpt"], Value::from(hostile));
        assert_eq!(back["reason"], Value::from(hostile));
        assert_eq!(back["command"], Value::from("search"));
        assert_eq!(back["ok"], Value::from(true));
    }

    /// The envelope is the same four keys whichever way the command went.
    #[test]
    fn the_envelope_is_the_same_shape_on_success_and_on_failure() {
        for document in [
            Document::new("verify"),
            Document::new("verify")
                .failed()
                .because("one blob is corrupt"),
        ] {
            let value: Value = serde_json::from_str(&document.render()).unwrap();
            let object = value.as_object().expect("a document is a JSON object");
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["command", "data", "ok", "reason"]);
            assert!(object["data"].is_object());
        }
    }

    /// A document with no reason carries the key with a null, never no key.
    #[test]
    fn a_reasonless_document_still_carries_the_key() {
        let value: Value = serde_json::from_str(&Document::new("status").render()).unwrap();
        assert_eq!(value["reason"], Value::Null);
        assert!(value.as_object().unwrap().contains_key("reason"));
    }
}
