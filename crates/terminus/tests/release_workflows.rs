//! The release workflows build and draft, and the owner signs and publishes
//! from his own machine. These tests keep it that way.

use std::path::Path;

/// Reading a secret, by either syntax, running a signing step, or holding a
/// private key.
const FORBIDDEN: [&str; 4] = ["secrets.", "secrets[", "-Y sign", "PRIVATE KEY"];

/// CI never holds the release key. The draft is made with the job's
/// `github.token`, so no workflow has a reason to read a secret, and a
/// workflow that signs anything means the key has moved off the owner's
/// machine. An empty folder fails too, so a moved one cannot pass by reading
/// nothing.
#[test]
fn workflows_hold_no_secret_and_no_signing_step() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    let mut workflows = 0;
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "yml") {
            workflows += 1;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (n, line) in text.lines().enumerate() {
            for needle in FORBIDDEN.iter().filter(|needle| line.contains(**needle)) {
                found.push(format!("{}:{}: {needle}", path.display(), n + 1));
            }
        }
    }
    assert!(workflows > 0, "{} holds no .yml file", dir.display());
    assert!(found.is_empty(), "{}", found.join("\n"));
}
