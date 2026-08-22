//! One `chat/completions` request, serving every OpenAI-compatible endpoint
//! (OBS-05, D-05).
//!
//! # One code path, three settings
//!
//! Base URL, model and key are the only things that differ between ollama on
//! `localhost:11434` and OpenRouter, Groq, DeepSeek or vLLM. There is no second
//! branch here: no Anthropic `x-api-key`, no `anthropic-version`, no second
//! body shape. Anthropic's own API and its subscription OAuth are out of this
//! phase entirely (D-05), and adding a shape for them is adding a branch to the
//! thing OBS-05 promises there is one of.
//!
//! # Two boundaries every request crosses
//!
//! The body goes through [`egress::for_destination`] on the DECLARED
//! destination before it is handed over (D-13), and the request itself goes
//! through [`net::post`] and through nothing else, so it lands in the attempt
//! log (PRIV-03).
//!
//! # Reading the answer: `choices[0].message.content`, specifically (D-08)
//!
//! Measured 2026-08-21 against ollama 0.32.15 running `qwen3:8b`: the `message`
//! object carried a third, non-standard key - `reasoning`, holding roughly a
//! kilobyte of chain-of-thought - beside `content`. A parser that stringified
//! the message object would feed that prose into the caller's JSON parse and
//! take OBS-04's retry-then-`parse_failed` path on every call against a
//! thinking model, burning two calls a session for nothing. So one path is read
//! and every other key on the message is ignored, whatever it turns out to be.
//!
//! # No caller can reach an unscrubbed error
//!
//! [`Error`]'s text is private and is written only by [`Error::new`], which
//! runs it through [`egress::scrub`] first (D-16). A 401 whose body echoes the
//! key back is the case that matters: `runs.error` is free text, `verbatim
//! status` prints it, and there is no log file, so an unscrubbed error is a
//! durable leak rather than one that scrolls past.

use std::fmt;

use serde_json::{json, Value};

use crate::config::{Config, ResponseFormat, Secret};
use crate::observe::{egress, net};

/// What is appended to the configured base URL.
pub const COMPLETIONS_PATH: &str = "chat/completions";

/// D-09's strict-JSON request, carrying the caller's schema.
///
/// `json_schema` mode is a two-part shape: `type` names the mode and a SIBLING
/// `json_schema` object carries the `name` and `schema` the answer is held to.
/// `strict` lives inside that object, not beside `type`.
///
/// It was a bare constant until AC4 was first run against a real remote
/// endpoint. The 2026-08-21 probe (ollama) accepted
/// `{"type":"json_schema","strict":true}` with no `json_schema` beside it, and
/// so does the loopback stub, because neither validates the field it is not
/// going to enforce. DeepSeek does: it answers 400 `missing field json_schema`.
/// A permissive local endpoint cannot tell you this request is malformed, which
/// is why the parameter arrived from a live remote run rather than from a test.
///
/// The schema is the CALLER's - `crate::observe::judgment::schema` already
/// returns the `{name, schema}` payload this wraps - so `provider` stays the one
/// request shape OBS-05 promises and learns nothing about observations.
fn response_format(mode: ResponseFormat, schema: Option<&Value>) -> Option<Value> {
    match mode {
        ResponseFormat::None => None,
        ResponseFormat::JsonObject => Some(json!({ "type": "json_object" })),
        ResponseFormat::JsonSchema => schema.map(|schema| {
            let mut payload = schema.clone();
            if let Some(object) = payload.as_object_mut() {
                object.insert("strict".to_owned(), Value::Bool(true));
            }
            json!({ "type": "json_schema", "json_schema": payload })
        }),
    }
}

/// One message in the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    role: &'static str,
    content: String,
}

impl Message {
    /// The instruction turn.
    pub fn system(content: impl Into<String>) -> Message {
        Message {
            role: "system",
            content: content.into(),
        }
    }

    /// The turn carrying what is to be judged.
    pub fn user(content: impl Into<String>) -> Message {
        Message {
            role: "user",
            content: content.into(),
        }
    }
}

/// What the provider reported it charged for.
///
/// Read back because PLAN-3's daily budget accumulates it (D-11, D-12) and the
/// probe returned one: `{"prompt_tokens":325,"completion_tokens":69,
/// "total_tokens":394}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// One answered call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// `choices[0].message.content`, exactly as it arrived. Not parsed here:
    /// what the content is supposed to be is the caller's schema, and OBS-04's
    /// failure arm needs the raw text to store.
    pub content: String,
    /// `None` when the provider reported no `usage` object at all.
    ///
    /// An `Option` and not a zeroed [`Usage`], because a budget that
    /// accumulated zeros would never be reached and the user would be paying an
    /// unbounded bill against a cap that reads as untouched.
    pub usage: Option<Usage>,
}

/// Why a call did not produce a [`Completion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `[provider] enabled` is unset or false, so no request was built.
    ///
    /// Its own kind rather than one of [`Kind::NotConfigured`]'s cases,
    /// because the two mean opposite things to a caller: this is the user
    /// having said no, which is the default and is silent, and that one is the
    /// user having said yes and left something out, which is worth a note.
    Disabled,
    /// There is nothing configured to call.
    NotConfigured,
    /// No bytes were exchanged: a connect failure, a timeout, a dead socket.
    Transport,
    /// The endpoint answered, and not with a 2xx.
    Status(u16),
    /// A 2xx whose body is not a chat completion this parser recognizes.
    Malformed,
}

/// A failed call, with its text already scrubbed (D-16).
///
/// The text is private and there is one constructor, so there is no way to
/// build one of these carrying an unscrubbed provider message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: Kind,
    detail: String,
}

impl Error {
    /// The only constructor, and the only place [`egress::scrub`] has to be
    /// remembered.
    fn new(kind: Kind, credential: Option<&Secret>, detail: impl AsRef<str>) -> Error {
        Error {
            kind,
            detail: egress::scrub(credential, detail.as_ref()),
        }
    }

    /// What sort of failure this is, for a caller that has to branch.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The scrubbed explanation.
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// The HTTP status, when there was one.
    pub fn status(&self) -> Option<u16> {
        match self.kind {
            Kind::Status(status) => Some(status),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            Kind::Disabled => write!(f, "provider judgment is off: {}", self.detail),
            Kind::NotConfigured => write!(f, "no provider is configured: {}", self.detail),
            Kind::Transport => write!(f, "the provider could not be reached: {}", self.detail),
            Kind::Status(status) => write!(f, "the provider answered {status}: {}", self.detail),
            Kind::Malformed => write!(
                f,
                "the provider's answer is not a chat completion: {}",
                self.detail
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Ask the configured provider to complete `messages`.
///
/// `credential` is whatever `crate::credentials::resolve` produced - `None` for
/// a local endpoint that wants no key. Resolution is the caller's, because
/// PRIV-02's refusal is a different failure from a provider failure and the two
/// must not be reported as one.
///
/// `schema` is the `{name, schema}` payload the answer is held to. `None` sends
/// no `response_format` at all rather than a half of one: an endpoint that
/// enforces the field refuses a request naming the mode without the schema, so
/// there is no shape to fall back to that is better than asking for nothing.
///
/// Makes exactly one request. Retries are OBS-04's and belong to the caller
/// that knows whether the content parsed.
pub fn complete(
    config: &Config,
    credential: Option<&Secret>,
    messages: &[Message],
    schema: Option<&Value>,
) -> Result<Completion, Error> {
    // OBS-02: judgment is opt-in and off by default, and this is where that is
    // enforced rather than assumed of the caller. Nothing below runs - no URL
    // is built, no header is assembled and `net::post` is never reached, so the
    // attempt log stays empty and a test can read that as a number.
    if !config.provider_enabled() {
        return Err(Error::new(
            Kind::Disabled,
            credential,
            "the [provider] table does not set enabled = true",
        ));
    }
    let Some(base_url) = config.provider_base_url() else {
        return Err(Error::new(
            Kind::NotConfigured,
            credential,
            "the [provider] table names no base_url",
        ));
    };
    let Some(model) = config.provider_model() else {
        return Err(Error::new(
            Kind::NotConfigured,
            credential,
            "the [provider] table names no model",
        ));
    };

    let url = endpoint(base_url);
    let mut request = json!({
        "model": model,
        "messages": messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect::<Vec<_>>(),
    });
    if let (Some(format), Some(object)) = (
        response_format(config.provider_response_format(), schema),
        request.as_object_mut(),
    ) {
        object.insert("response_format".to_owned(), format);
    }
    let body = request.to_string();
    // D-13: the declaration decides, not the address. `local` absent or false
    // filters, so a user who forgot the key pays the filter rather than sending
    // unfiltered session text offsite.
    let body = egress::for_destination(config.provider_local(), credential, &body);

    let mut headers: Vec<(&str, String)> = vec![("content-type", "application/json".to_owned())];
    if let Some(secret) = credential {
        // The one site in this workspace where a credential leaves its wrapper
        // to go on the wire. `net::post` moves header values into the request
        // and copies them nowhere else.
        headers.push(("authorization", format!("Bearer {}", secret.expose())));
    }
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();

    let response = net::post(&url, &headers, body.as_bytes())
        .map_err(|e| Error::new(Kind::Transport, credential, e.to_string()))?;
    if !response.is_success() {
        return Err(Error::new(
            Kind::Status(response.status),
            credential,
            &response.body,
        ));
    }
    parse(&response.body, credential)
}

/// The base URL with [`COMPLETIONS_PATH`] on the end.
///
/// String concatenation and not URL parsing (D-13): there is no `url` crate in
/// this workspace on purpose, and the only thing that has to be got right here
/// is not doubling or dropping the separator.
fn endpoint(base_url: &str) -> String {
    if base_url.ends_with('/') {
        format!("{base_url}{COMPLETIONS_PATH}")
    } else {
        format!("{base_url}/{COMPLETIONS_PATH}")
    }
}

/// Read `choices[0].message.content` and the `usage` object, and nothing else
/// (D-08).
fn parse(body: &str, credential: Option<&Secret>) -> Result<Completion, Error> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| Error::new(Kind::Malformed, credential, format!("{e}: {body}")))?;

    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::new(
                Kind::Malformed,
                credential,
                format!("choices[0].message.content is missing or is not a string: {body}"),
            )
        })?;

    Ok(Completion {
        content: content.to_owned(),
        usage: value.get("usage").map(|usage| Usage {
            prompt_tokens: count(usage, "prompt_tokens"),
            completion_tokens: count(usage, "completion_tokens"),
            total_tokens: count(usage, "total_tokens"),
        }),
    })
}

/// One token count, or zero when the provider left it out of an object it did
/// send. Zero for one field is a gap in a report; a missing `usage` object
/// entirely is the different thing [`Completion::usage`] keeps as `None`.
fn count(usage: &Value, field: &str) -> u64 {
    usage.get(field).and_then(Value::as_u64).unwrap_or(0)
}
