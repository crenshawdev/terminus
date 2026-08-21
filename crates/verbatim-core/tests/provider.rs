//! The one door to the network: where the HTTP client is allowed to be named,
//! and the log that makes "zero connections" a number instead of a claim
//! (PRIV-03, D-21).
//!
//! The attempt log is process-global, so every test that reads it holds `NET`:
//! two tests resetting and counting in parallel would each see the other's
//! attempts and both would be measuring nothing.
//!
//! The stub is a real `std::net::TcpListener` on a loopback port speaking one
//! canned HTTP response (D-20). The project bars mocks the way it bars them for
//! SQLite: real sockets, real bytes, real parsing on the way back.

use std::path::{Path, PathBuf};

/// Held by every test that resets or reads the attempt log.
#[cfg(feature = "testkit")]
static NET: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ---------------------------------------------------------------------------
// One client, named in one file

/// The workspace has exactly one HTTP client and exactly one file may name it.
///
/// A source-level assertion rather than a design note, for the reason D-21
/// gives: the strongest privacy claim in the product would otherwise be proven
/// only by a seam that a second client, added anywhere, would walk straight
/// past. This is the `inject_brief.rs` precedent - comment lines stripped the
/// same way, so a doc comment explaining the rule is not itself a violation of
/// it.
#[test]
fn the_http_client_is_named_in_exactly_one_source_file() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let binary = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crates directory")
        .join("verbatim")
        .join("src");
    let allowed = core.join("observe").join("net.rs");

    let mut checked = 0;
    let mut naming = Vec::new();
    for root in [&core, &binary] {
        for path in rust_files(root) {
            let source = std::fs::read_to_string(&path).unwrap();
            let code: String = source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            if code.contains("ureq") && path != allowed {
                naming.push(path.display().to_string());
            }
            checked += 1;
        }
    }

    assert!(
        naming.is_empty(),
        "the HTTP client is named outside {}: {naming:?}",
        allowed.display()
    );
    // The falsifying half: a walk that found nothing would pass the assertion
    // above for the wrong reason, and so would one that never reached the
    // binary crate.
    assert!(checked > 30, "only {checked} source files were walked");
    let allowed_source = std::fs::read_to_string(&allowed).unwrap();
    let allowed_code: String = allowed_source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        allowed_code.contains("ureq"),
        "{} does not name the client, so this test is asserting over nothing",
        allowed.display()
    );
}

/// Every `.rs` file at or beneath `root`, sorted.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// The attempt log

/// One request against a real socket, and exactly one entry in the log.
#[test]
#[cfg(feature = "testkit")]
fn one_request_records_exactly_one_attempt() {
    use verbatim_core::observe::net;

    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    let stub = Stub::serving("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
    let url = stub.url("chat/completions");

    net::attempts::reset();
    let response =
        net::post(&url, &[("content-type", "application/json")], b"{}").expect("the stub answered");

    assert_eq!(response.status, 200);
    assert_eq!(response.body, "hi");
    assert_eq!(
        net::attempts::destinations(),
        vec![url.clone()],
        "the attempt log did not record exactly this one destination"
    );
    assert_eq!(net::attempts::count(), 1);

    let request = stub.finish();
    assert!(
        request.starts_with("POST /chat/completions "),
        "the stub received: {request}"
    );
}

/// A destination that refuses the connection is still an attempt: PRIV-03 is
/// about what this binary reaches for, not about what it reached.
#[test]
#[cfg(feature = "testkit")]
fn a_connection_that_fails_is_still_a_recorded_attempt() {
    use verbatim_core::observe::net;

    let _guard = NET.lock().unwrap_or_else(|e| e.into_inner());
    // Bound, its port read, then dropped: nothing is listening there, and the
    // port was free a moment ago so nothing else is either.
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let url = format!("http://{dead}/chat/completions");

    net::attempts::reset();
    let error = net::post(&url, &[], b"{}").expect_err("nothing is listening there");

    assert_eq!(error.url(), url);
    assert_eq!(net::attempts::destinations(), vec![url]);
}

// ---------------------------------------------------------------------------
// The stub

/// A `std::net::TcpListener` on a loopback port serving one canned response.
#[cfg(feature = "testkit")]
struct Stub {
    base: String,
    served: std::thread::JoinHandle<String>,
}

#[cfg(feature = "testkit")]
impl Stub {
    fn serving(response: &'static str) -> Stub {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let served = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("one connection");
            let request = read_request(&mut stream);
            stream.write_all(response.as_bytes()).expect("respond");
            stream.flush().ok();
            request
        });
        Stub { base, served }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The request the stub received, once it has finished serving it.
    fn finish(self) -> String {
        self.served.join().expect("the stub thread")
    }
}

/// Read one HTTP request: the head, then exactly the declared body.
///
/// Read to the declared length rather than to EOF, because the client keeps the
/// socket open for a response it has not been given yet.
#[cfg(feature = "testkit")]
fn read_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    while !buffer.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).expect("read the head") == 0 {
            break;
        }
        buffer.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buffer).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 {
        stream.read_exact(&mut body).expect("read the body");
    }
    format!("{head}{}", String::from_utf8_lossy(&body))
}
