//! Terminus opens no network connection.
//!
//! No source file in either crate names a socket type. Comment lines are
//! scanned too, so a socket cannot come back with the comment that explains it.
//! Not gated on `testkit`, so `cargo test -p terminus-core` always runs it.

use std::path::{Path, PathBuf};

const BANNED: [&str; 4] = ["std::net", "TcpStream", "TcpListener", "UdpSocket"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_source_file_names_a_socket() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots = [
        core.join("src"),
        core.parent().unwrap().join("terminus/src"),
    ];
    for root in &roots {
        let mut files = Vec::new();
        rust_files(root, &mut files);
        // Each crate has dozens of source files; a walk that found a handful
        // went somewhere else.
        assert!(
            files.len() >= 20,
            "only {} files under {}",
            files.len(),
            root.display()
        );
        for path in files {
            let source = std::fs::read_to_string(&path).unwrap();
            for term in BANNED {
                assert!(!source.contains(term), "{} names {term}", path.display());
            }
        }
    }
}
