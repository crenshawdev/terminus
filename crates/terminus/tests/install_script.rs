//! `install.sh` run end to end: real `sh`, `curl`, `sha256sum` and `mktemp`
//! against a local HTTPS server, ending in a real `terminus install`.
//!
//! The server is `openssl s_server -WWW` with a certificate made for each test
//! and named by `CURL_CA_BUNDLE`, so the script stays https-only. The served
//! binary is a copy of this build, with bytes appended when a test needs a
//! different digest; it still runs and still carries the marker.
//!
//! `HOME`, `CLAUDE_CONFIG_DIR`, `TERMINUS_DATA_DIR` and `TERMINUS_CONFIG_DIR`
//! all point into a temporary root, so no run touches this machine's install.
//! The environment is cleared first, which also drops `TERMINUS_BIN_DIR`, the
//! traffic variable and any proxy curl would otherwise honour.
//!
//! `PATH` is a folder holding the fake `claude`, then a folder of links to the
//! system tools the script calls, and nothing else. The system folders cannot
//! be on it: `/usr/bin/claude` is a real Claude Code, and no test may start
//! one. The tools folder is also how a test leaves `sha256sum` out or swaps in
//! a fake `uname`.
//!
//! `s_server -WWW` answers a missing file with `200 ok` and an error text, so
//! every test serves both the manifest and the binary.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use terminus_core::testkit::{self, FakeCall};

/// Every program `install.sh` runs that is not a shell builtin.
const TOOLS: [&str; 11] = [
    "uname",
    "mktemp",
    "rm",
    "curl",
    "sha256sum",
    "mkdir",
    "chmod",
    "mv",
    "ln",
    "readlink",
    "grep",
];

/// The events install writes an entry for, as `tests/install.rs` spells them.
const EVENTS: [&str; 4] = [
    "SessionStart",
    "UserPromptSubmit",
    "SessionEnd",
    "PostCompact",
];

/// The arguments of `claude mcp add-json --scope user terminus`, up to the JSON.
const ADD: [&str; 5] = ["mcp", "add-json", "--scope", "user", "terminus"];

/// The same `settings.json` `tests/install.rs` seeds.
const SETTINGS: &str = r#"{
  "cleanupPeriodDays": 7,
  "env": {
    "CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS": "50000"
  },
  "hooks": {
    "SessionStart": [],
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "$HOME/.claude/hooks/terse-answers.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "PreToolUse": []
  },
  "autoCompactEnabled": true,
  "theme": "dark"
}
"#;

/// The same `.claude.json` `tests/install.rs` seeds.
const CLAUDE_JSON: &str = r#"{
  "numStartups": 412,
  "mcpServers": {
    "context7": {
      "type": "http",
      "url": "https://example.invalid/mcp"
    }
  },
  "projects": {
    "/data/code/verbatim": {
      "allowedTools": [],
      "history": []
    },
    "/data/code/other": {
      "allowedTools": []
    }
  },
  "oauthAccount": {
    "emailAddress": "someone@example.invalid"
  }
}"#;

/// `openssl s_server -WWW` serving one folder, killed on drop.
struct Server {
    child: Child,
    port: u16,
    cert: PathBuf,
}

impl Server {
    fn start(root: &Path, served: &Path) -> Server {
        let cert = root.join("cert.pem");
        let key = root.join("key.pem");
        let made = Command::new("openssl")
            .args(["req", "-x509", "-newkey", "ec"])
            .args(["-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"])
            .arg("-keyout")
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .args(["-days", "1", "-subj", "/CN=127.0.0.1"])
            .args(["-addext", "subjectAltName=IP:127.0.0.1"])
            .stdin(Stdio::null())
            .output()
            .expect("openssl runs");
        assert!(made.status.success(), "openssl req: {}", text(&made));

        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let log = root.join("server.log");
        let child = Command::new("openssl")
            .args(["s_server", "-WWW", "-accept"])
            .arg(format!("127.0.0.1:{port}"))
            .arg("-cert")
            .arg(&cert)
            .arg("-key")
            .arg(&key)
            .current_dir(served)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .expect("openssl s_server starts");
        let mut server = Server { child, port, cert };

        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err() {
            let exited = server.child.try_wait().unwrap();
            if exited.is_some() || Instant::now() > deadline {
                panic!(
                    "s_server on port {port} never accepted ({exited:?}): {}",
                    std::fs::read_to_string(&log).unwrap_or_default()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        server
    }

    fn url(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    claude_dir: PathBuf,
    /// The fake `claude`'s folder, first on `PATH`.
    fake_dir: PathBuf,
    /// Links to every tool in [`TOOLS`], second on `PATH`.
    tools: PathBuf,
    /// The served folder: `<os>-<arch>/manifest` and `<os>-<arch>/terminus`.
    served: PathBuf,
    server: Server,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let home = root.join("home");
    let claude_dir = root.join("claude");
    let fake_dir = root.join("fake");
    let served = root.join("served");
    for folder in [&home, &claude_dir, &fake_dir, &served.join(platform())] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(claude_dir.join("settings.json"), SETTINGS).unwrap();
    std::fs::write(claude_dir.join(".claude.json"), CLAUDE_JSON).unwrap();
    let server = Server::start(&root, &served);
    let mut fixture = Fixture {
        _dir: dir,
        tools: PathBuf::new(),
        root,
        home,
        claude_dir,
        fake_dir,
        served,
        server,
    };
    fixture.tools = fixture.tools_folder("tools", &[], &[]);
    testkit::fake_claude(&fixture.fake_dir, &fixture.claude_log(), "", 0);
    fixture
}

/// This machine as the script names it: `linux` or `macos`, then the
/// architecture.
fn platform() -> String {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        other => panic!("install.sh does not support {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => panic!("install.sh does not support {other}"),
    };
    format!("{os}-{arch}")
}

/// Where `name` lives on this test process's `PATH`.
fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|folder| folder.join(name))
        .find(|path| path.is_file())
}

/// The system `shasum`, which Arch keeps off the usual folders.
fn shasum() -> PathBuf {
    which("shasum")
        .or_else(|| Some(PathBuf::from("/usr/bin/core_perl/shasum")).filter(|p| p.is_file()))
        .expect("shasum is not on PATH or at /usr/bin/core_perl/shasum; the fallback test needs it")
}

/// The SHA-256 of `path` from the system `sha256sum`, as lowercase hex.
fn sha256(path: &Path) -> String {
    let output = Command::new("sha256sum").arg(path).output().unwrap();
    assert!(output.status.success(), "sha256sum: {}", text(&output));
    String::from_utf8(output.stdout).unwrap()[..64].to_owned()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Copy this build to `to` with `extra` appended, executable.
///
/// Written by a child shell, not by this process: a file this process had open
/// for writing could be inherited by another test's fork, and running it would
/// then fail with ETXTBSY.
fn copy_build(to: &Path, extra: &str) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    let status = Command::new("/bin/sh")
        .args([
            "-c",
            "cat \"$1\" > \"$2\" && printf %s \"$3\" >> \"$2\" && chmod 755 \"$2\"",
            "sh",
            env!("CARGO_BIN_EXE_terminus"),
            &to.display().to_string(),
            extra,
        ])
        .env("PATH", "/usr/bin:/bin")
        .status()
        .unwrap();
    assert!(status.success(), "the copy of this build failed");
}

impl Fixture {
    /// A folder of links to [`TOOLS`], leaving out `without` and pointing each
    /// `(name, path)` in `swap` at `path` instead. Never holds `claude`.
    fn tools_folder(&self, name: &str, without: &[&str], swap: &[(&str, &Path)]) -> PathBuf {
        let folder = self.root.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        for tool in TOOLS.iter().filter(|tool| !without.contains(tool)) {
            let target = which(tool).unwrap_or_else(|| panic!("{tool} is not on PATH"));
            std::os::unix::fs::symlink(target, folder.join(tool)).unwrap();
        }
        for (tool, target) in swap {
            let link = folder.join(tool);
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(target, link).unwrap();
        }
        assert!(!folder.join("claude").exists());
        folder
    }

    fn stable(&self) -> PathBuf {
        self.home.join(".local/bin/terminus")
    }

    fn versions(&self) -> PathBuf {
        self.home.join(".local/lib/crenshawdev/terminus/versions")
    }

    fn version_file(&self, version: &str) -> PathBuf {
        self.versions().join(version).join("terminus")
    }

    fn settings(&self) -> PathBuf {
        self.claude_dir.join("settings.json")
    }

    fn claude_log(&self) -> PathBuf {
        self.root.join("claude.log")
    }

    fn calls(&self) -> Vec<FakeCall> {
        testkit::fake_claude_calls(&self.claude_log())
    }

    /// Serve `version`: this build with `extra` appended, and a manifest
    /// naming its digest. Returns the served bytes.
    fn serve(&self, version: &str, extra: &str) -> Vec<u8> {
        let binary = self.served.join(platform()).join("terminus");
        copy_build(&binary, extra);
        self.serve_manifest(version, &sha256(&binary));
        std::fs::read(binary).unwrap()
    }

    /// Replace the served manifest with one naming `version` and `digest`.
    fn serve_manifest(&self, version: &str, digest: &str) {
        let manifest = self.served.join(platform()).join("manifest");
        std::fs::write(manifest, format!("version {version}\nsha256 {digest}\n")).unwrap();
    }

    /// `/bin/sh install.sh <source>` with the default tools folder.
    fn command(&self, source: &str) -> Command {
        self.command_with(source, &self.tools)
    }

    fn command_with(&self, source: &str, tools: &Path) -> Command {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
        let path = std::env::join_paths([&self.fake_dir, tools]).unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .arg(script)
            .arg(source)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("TERMINUS_DATA_DIR", self.root.join("data"))
            .env("TERMINUS_CONFIG_DIR", self.root.join("config"))
            .env("CURL_CA_BUNDLE", &self.server.cert)
            .stdin(Stdio::null());
        command
    }

    fn run(&self) -> Output {
        self.command(&self.server.url()).output().unwrap()
    }

    fn run_with(&self, tools: &Path) -> Output {
        self.command_with(&self.server.url(), tools)
            .output()
            .unwrap()
    }

    /// The stable path is a link to `version`'s file, which holds `bytes`.
    fn assert_linked(&self, version: &str, bytes: &[u8]) {
        assert_eq!(
            std::fs::read_link(self.stable()).unwrap(),
            self.version_file(version)
        );
        assert!(
            std::fs::read(self.version_file(version)).unwrap() == bytes,
            "{} does not hold the served bytes",
            self.version_file(version).display()
        );
    }

    /// Serve `version` with `extra` appended and install it; the run must
    /// succeed. Returns the served bytes.
    fn install(&self, version: &str, extra: &str) -> Vec<u8> {
        let bytes = self.serve(version, extra);
        let output = self.run();
        assert!(
            output.status.success(),
            "install {version}: {}",
            text(&output)
        );
        bytes
    }

    /// A tools folder whose `uname` answers `-s` with `system` and `-m` with
    /// `machine`.
    fn uname_tools(&self, system: &str, machine: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let fake = self.root.join(format!("uname-{system}-{machine}"));
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\ncase $1 in -s) printf '%s\\n' {system} ;; -m) printf '%s\\n' {machine} ;; esac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        self.tools_folder(
            &format!("tools-{system}-{machine}"),
            &[],
            &[("uname", &fake)],
        )
    }

    /// The stable path, everything under the versions folder, and both
    /// Claude Code files, as a refusal must leave them.
    fn snapshot(&self) -> Snapshot {
        let mut versions = Vec::new();
        walk(&self.versions(), Path::new("."), &mut versions);
        Snapshot {
            stable: entry(&self.stable()),
            versions,
            settings: Bytes(std::fs::read(self.settings()).unwrap()),
            claude_json: Bytes(std::fs::read(self.claude_dir.join(".claude.json")).unwrap()),
        }
    }

    /// Run `command` and assert it was refused: non-zero, a reason on stderr,
    /// nothing on disk changed and no `claude` started.
    fn assert_refused(&self, mut command: Command) {
        let before = self.snapshot();
        let calls = self.calls().len();
        let output = command.output().unwrap();
        assert!(!output.status.success(), "{}", text(&output));
        assert!(!output.stderr.is_empty(), "{}", text(&output));
        assert_eq!(self.snapshot(), before, "{}", text(&output));
        assert_eq!(self.calls().len(), calls, "{:?}", self.calls());
    }
}

/// File contents, compared in full and printed as a length.
#[derive(PartialEq)]
struct Bytes(Vec<u8>);

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} bytes>", self.0.len())
    }
}

#[derive(Debug, PartialEq)]
enum Entry {
    Missing,
    Folder,
    Link(PathBuf),
    File { bytes: Bytes, inode: u64 },
    Other,
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    stable: Entry,
    versions: Vec<(PathBuf, Entry)>,
    settings: Bytes,
    claude_json: Bytes,
}

fn entry(path: &Path) -> Entry {
    use std::os::unix::fs::MetadataExt;

    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Entry::Missing,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        Entry::Link(std::fs::read_link(path).unwrap())
    } else if kind.is_dir() {
        Entry::Folder
    } else if kind.is_file() {
        Entry::File {
            bytes: Bytes(std::fs::read(path).unwrap()),
            inode: meta.ino(),
        }
    } else {
        Entry::Other
    }
}

/// `path` and everything under it, sorted, named relative to the start.
fn walk(path: &Path, name: &Path, into: &mut Vec<(PathBuf, Entry)>) {
    let found = entry(path);
    let folder = found == Entry::Folder;
    into.push((name.to_path_buf(), found));
    if folder {
        let mut children: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|child| child.unwrap().file_name())
            .collect();
        children.sort();
        for child in children {
            walk(&path.join(&child), &name.join(&child), into);
        }
    }
}

/// A fake `claude` that exits 0: the run exits 0, the link points at the
/// manifest's version, the hooks and the MCP entry name the stable path.
#[test]
fn one_run_installs_and_wires_claude_code() {
    let fx = fixture();
    let bytes = fx.serve("1.0.0", "");
    let output = fx.run();
    assert!(output.status.success(), "{}", text(&output));
    fx.assert_linked("1.0.0", &bytes);

    let stable = fx.stable().display().to_string();
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fx.settings()).unwrap()).unwrap();
    for event in EVENTS {
        let ours: Vec<&serde_json::Value> = settings["hooks"][event]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|group| group["hooks"].as_array())
            .flatten()
            .filter(|hook| hook["command"] == stable.as_str())
            .collect();
        assert_eq!(ours.len(), 1, "{event}: {settings}");
    }

    let adds: Vec<FakeCall> = fx
        .calls()
        .into_iter()
        .filter(|call| call.args.len() == 6 && call.args[..5] == ADD)
        .collect();
    assert_eq!(adds.len(), 1, "{:?}", fx.calls());
    let entry: serde_json::Value = serde_json::from_str(&adds[0].args[5]).unwrap();
    assert_eq!(entry["command"], stable.as_str(), "{entry}");
    assert_eq!(entry["args"], serde_json::json!(["mcp"]), "{entry}");
}

/// No `claude` in either folder: the link is placed, and the receipt's
/// "install incomplete" line is the script's non-zero exit.
#[test]
fn no_claude_exits_non_zero_after_the_link_is_placed() {
    let fx = fixture();
    std::fs::remove_file(fx.fake_dir.join("claude")).unwrap();
    let bytes = fx.serve("1.0.0", "");
    let output = fx.run();
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("install incomplete: the mcp server is not registered"),
        "{}",
        text(&output)
    );
    fx.assert_linked("1.0.0", &bytes);
}

/// Without `sha256sum`, `shasum -a 256` computes the digest: the install
/// succeeds, and a wrong manifest digest is refused with the real one named.
#[test]
fn shasum_computes_the_digest_when_sha256sum_is_missing() {
    let fx = fixture();
    let shasum = shasum();
    let tools = fx.tools_folder("tools-shasum", &["sha256sum"], &[("shasum", &shasum)]);

    let bytes = fx.serve("1.0.0", "");
    let output = fx.run_with(&tools);
    assert!(output.status.success(), "{}", text(&output));
    fx.assert_linked("1.0.0", &bytes);

    let binary = fx.served.join(platform()).join("terminus");
    copy_build(&binary, "different");
    let real = sha256(&binary);
    fx.serve_manifest("2.0.0", &"0".repeat(64));
    let output = fx.run_with(&tools);
    assert!(!output.status.success(), "{}", text(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!("download SHA-256 is {real}")),
        "{}",
        text(&output)
    );
    fx.assert_linked("1.0.0", &bytes);
}

/// The manifest names version A's digest, but the binary has other bytes.
#[test]
fn a_digest_that_does_not_match_is_refused() {
    let fx = fixture();
    fx.install("1.0.0", "");
    let binary = fx.served.join(platform()).join("terminus");
    let digest_a = sha256(&binary);
    copy_build(&binary, "tampered");
    fx.serve_manifest("2.0.0", &digest_a);
    fx.assert_refused(fx.command(&fx.server.url()));
}

#[test]
fn an_unsupported_system_is_refused() {
    let fx = fixture();
    fx.install("1.0.0", "");
    fx.serve("2.0.0", "b");
    let tools = fx.uname_tools("FreeBSD", "x86_64");
    fx.assert_refused(fx.command_with(&fx.server.url(), &tools));
}

#[test]
fn an_unsupported_architecture_is_refused() {
    let fx = fixture();
    fx.install("1.0.0", "");
    fx.serve("2.0.0", "b");
    let tools = fx.uname_tools("Linux", "riscv64");
    fx.assert_refused(fx.command_with(&fx.server.url(), &tools));
}

#[test]
fn an_http_source_is_refused() {
    let fx = fixture();
    fx.install("1.0.0", "");
    fx.serve("2.0.0", "b");
    let source = fx.server.url().replacen("https://", "http://", 1);
    fx.assert_refused(fx.command(&source));
}

/// `terminus install` would point the hooks and the MCP entry into that
/// folder while the script linked `~/.local/bin/terminus`.
#[test]
fn terminus_bin_dir_set_is_refused() {
    let fx = fixture();
    fx.install("1.0.0", "");
    fx.serve("2.0.0", "b");
    let mut command = fx.command(&fx.server.url());
    command.env("TERMINUS_BIN_DIR", fx.root.join("elsewhere"));
    fx.assert_refused(command);
}

/// Somebody else's program at the stable path.
#[test]
fn a_file_without_the_marker_is_refused() {
    let fx = fixture();
    std::fs::create_dir_all(fx.stable().parent().unwrap()).unwrap();
    std::fs::write(fx.stable(), "#!/bin/sh\nexit 0\n").unwrap();
    fx.serve("1.0.0", "");
    fx.assert_refused(fx.command(&fx.server.url()));
}

/// A link to nothing is the script's to replace only when it points into the
/// versions folder.
#[test]
fn a_dangling_link_outside_the_versions_folder_is_refused() {
    let fx = fixture();
    std::fs::create_dir_all(fx.stable().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(fx.root.join("elsewhere/terminus"), fx.stable()).unwrap();
    fx.serve("1.0.0", "");
    fx.assert_refused(fx.command(&fx.server.url()));
    assert_eq!(
        std::fs::read_link(fx.stable()).unwrap(),
        fx.root.join("elsewhere/terminus")
    );
}

fn inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).unwrap().ino()
}

impl Fixture {
    /// `<stable path> mcp`, started through the link, its stdin held open.
    fn mcp(&self) -> Child {
        Command::new(self.stable())
            .arg("mcp")
            .env_clear()
            .env("PATH", &self.fake_dir)
            .env("HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude_dir)
            .env("TERMINUS_DATA_DIR", self.root.join("data"))
            .env("TERMINUS_CONFIG_DIR", self.root.join("config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }
}

/// Installing B moves the link and leaves A's file exactly as it was, so a
/// session started on A keeps running from it.
#[test]
fn successive_installs_keep_every_version() {
    let fx = fixture();
    let bytes_a = fx.install("1.0.0", "a");
    let file_a = fx.version_file("1.0.0");
    let inode_a = inode(&file_a);
    let mut mcp = fx.mcp();

    let bytes_b = fx.install("2.0.0", "b");
    fx.assert_linked("2.0.0", &bytes_b);
    assert!(
        std::fs::read(&file_a).unwrap() == bytes_a,
        "A's bytes changed"
    );
    assert_eq!(inode(&file_a), inode_a);

    #[cfg(target_os = "linux")]
    {
        assert!(mcp.try_wait().unwrap().is_none(), "terminus mcp exited");
        let exe = std::fs::read_link(format!("/proc/{}/exe", mcp.id())).unwrap();
        assert_eq!(exe, file_a.canonicalize().unwrap());
    }

    drop(mcp.stdin.take());
    mcp.wait().unwrap();
}

/// A rebuild at the same version is renamed over that version's file; the
/// same bytes again leave it alone.
#[test]
fn a_rebuild_at_the_same_version_replaces_its_file() {
    let fx = fixture();
    fx.install("3.0.0", "c");
    let first = inode(&fx.version_file("3.0.0"));

    let rebuilt = fx.install("3.0.0", "c-rebuilt");
    fx.assert_linked("3.0.0", &rebuilt);
    let second = inode(&fx.version_file("3.0.0"));
    assert_ne!(
        second, first,
        "the rebuild was written through, not renamed over"
    );

    fx.install("3.0.0", "c-rebuilt");
    fx.assert_linked("3.0.0", &rebuilt);
    assert_eq!(inode(&fx.version_file("3.0.0")), second);
}

/// A plain-file install from before the link, carrying the marker, is
/// replaced by the link.
#[test]
fn a_marked_plain_file_is_replaced_by_the_link() {
    let fx = fixture();
    copy_build(&fx.stable(), "");
    let bytes = fx.serve("1.0.0", "");
    let output = fx.run();
    assert!(output.status.success(), "{}", text(&output));
    fx.assert_linked("1.0.0", &bytes);
}

/// The link left behind when the versions folder was deleted is the script's
/// to replace.
#[test]
fn a_dangling_link_into_the_versions_folder_is_replaced() {
    let fx = fixture();
    fx.install("1.0.0", "a");
    std::fs::remove_dir_all(fx.versions()).unwrap();
    assert!(!fx.stable().exists() && fx.stable().is_symlink());

    let bytes = fx.serve("2.0.0", "b");
    let output = fx.run();
    assert!(output.status.success(), "{}", text(&output));
    fx.assert_linked("2.0.0", &bytes);
}
