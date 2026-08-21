//! The one door to the network (PRIV-03, D-21).
//!
//! **Every outbound connection this binary can make is built here.** That is
//! the whole reason the module exists, and it is a stronger claim than "the
//! provider call lives here": the privacy contract in `.planning/PROJECT.md` is
//! "no network connections except the configured model provider", and a claim
//! like that is only as good as the number of places that could falsify it.
//! One constructor makes the number one.
//!
//! **The HTTP client is named here and nowhere else.** `ureq` appears in this
//! file and in no other `.rs` file in either crate, which
//! `tests/provider.rs::the_http_client_is_named_in_exactly_one_source_file`
//! asserts by walking the source. Requests go out through [`post`] and come
//! back as this module's own [`Response`], so the client is replaceable - and,
//! far more importantly, countable.
//!
//! **The attempt log is the count.** [`attempts`] records the destination of
//! every call *before* the connection is made, so "zero connections" is a
//! number a test reads rather than a claim about the code. It is `testkit`-
//! gated in the exact shape of [`crate::discover::opened`] and compiles to
//! nothing in a shipped build.
//!
//! **No redirects, ever.** A followed redirect is a second connection to a
//! destination the attempt log never saw, which would make the log a lie; it is
//! also a bearer credential replayed at somewhere the user did not configure.
//! [`post`] therefore follows none and hands the 3xx back as an ordinary
//! response.

use std::fmt;
use std::time::Duration;

/// How long one exchange may take, end to end - DNS, connect, TLS handshake,
/// request, response and body.
///
/// Generous, because a local model on a cold GPU can think for a minute and the
/// caller asked for that; bounded, because D-07 puts this call outside the
/// ingest lock precisely so a hang cannot wedge the archive, and an unbounded
/// socket would still leave a process sitting on the machine forever. A
/// compile-time constant and not a config key: D-11 promoted exactly one
/// money-facing knob to configuration and left the quality and safety
/// thresholds spelled in source.
const TIMEOUT: Duration = Duration::from_secs(120);

/// One completed exchange.
///
/// A non-2xx status is a [`Response`] and not an [`Error`]: the body of a 401
/// or a 429 is the only thing that says *why*, and a client that threw it away
/// would leave the caller reporting a bare number. The caller decides what a
/// status means; this module only decides whether bytes were exchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status line's code.
    pub status: u16,
    /// The response body as text, invalid UTF-8 replaced rather than refused: a
    /// garbled body is something the caller must be able to store and report
    /// (OBS-04's parse-failure arm), not a transport failure.
    pub body: String,
}

impl Response {
    /// Did the server answer 2xx?
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// An exchange that did not complete: no status, no body.
///
/// It carries the destination and the client's own explanation, and it
/// deliberately carries nothing else. Header VALUES never reach it - the bearer
/// credential is a header value - so this type cannot be the path a key takes
/// to a stream. The caller scrubs it again on the way to an error message
/// (D-16); this is the half that makes that scrub a second line of defence
/// rather than the only one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    url: String,
    detail: String,
}

impl Error {
    /// The destination that was attempted.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The client's own account of what went wrong.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "POST {}: {}", self.url, self.detail)
    }
}

impl std::error::Error for Error {}

/// POST `body` to `url` with `headers`, and read the answer.
///
/// The attempt is recorded before anything is opened, including for a call that
/// fails to connect: PRIV-03 is about what this binary reaches for, not about
/// what it succeeded in reaching.
///
/// `headers` are appended in the order given. Their values are moved into the
/// request and are never copied into an error, a log line or the attempt
/// record - the authorization header is one of them.
pub fn post(url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Response, Error> {
    attempts::record(url);

    let mut request = agent().post(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }

    let mut response = request.send(body).map_err(|e| Error {
        url: url.to_owned(),
        detail: e.to_string(),
    })?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|e| Error {
        url: url.to_owned(),
        detail: e.to_string(),
    })?;
    Ok(Response { status, body })
}

/// The client, configured once, in the one place it is allowed to be named.
///
/// Built per call rather than kept in a static: this binary makes at most one
/// provider call per invocation (OBS-02) and then exits, so a pooled agent
/// would be a connection pool with nothing to pool and a `static` holding a
/// live socket across the rest of the process.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // A 4xx or 5xx is an answer with a body, and that body is the only
        // thing that says why. Turning it into an error would discard it.
        .http_status_as_error(false)
        // See the module comment: a redirect is a connection the attempt log
        // never recorded. Zero means the 3xx is returned as-is rather than
        // raising `TooManyRedirects`.
        .max_redirects(0)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into()
}

/// The counted-attempt log (D-21).
///
/// **Destinations and not a count**, for the reason [`crate::discover::opened`]
/// records paths: "zero connections to anywhere" and "zero connections to this
/// host" are different assertions, and only the first is answerable from a
/// scalar. A count is derivable from the list wherever one is wanted.
///
/// Recording happens *before* the connection, so a destination this binary
/// reached for and failed to reach is still in the log.
#[cfg(feature = "testkit")]
pub mod attempts {
    use std::sync::Mutex;

    static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn log() -> std::sync::MutexGuard<'static, Vec<String>> {
        // A test that panicked mid-assertion poisons the lock, and a poisoned
        // lock here would turn one failing test into every following test
        // failing for an unrelated reason.
        LOG.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn record(url: &str) {
        log().push(url.to_owned());
    }

    /// Forget every recorded attempt. Call it at the start of a test that
    /// asserts on the log, never at the end: a test that failed leaves its
    /// evidence.
    pub fn reset() {
        log().clear();
    }

    /// Every destination attempted since the last [`reset`], in order.
    pub fn destinations() -> Vec<String> {
        log().clone()
    }

    /// How many attempts were recorded in total.
    pub fn count() -> usize {
        log().len()
    }
}

#[cfg(not(feature = "testkit"))]
pub mod attempts {
    #[inline(always)]
    pub(super) fn record(_url: &str) {}
}
