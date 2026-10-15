//! The release workflows build and draft, and the owner signs and publishes
//! from his own machine. These tests keep it that way.

use std::path::Path;

/// Reading a secret, by either syntax, running a signing step, or holding a
/// private key.
const FORBIDDEN: [&str; 4] = ["secrets.", "secrets[", "-Y sign", "PRIVATE KEY"];

/// The one secret a workflow may read, and the one workflow that may read it.
const REVIEW_KEY: &str = "secrets.ANTHROPIC_API_KEY";
const REVIEW_WORKFLOW: &str = "fast.yml";

/// CI never holds the release key. The draft is made with the job's
/// `github.token`, and the only secret any workflow reads is the API key
/// `fast.yml` hands to its security-review job, which signs nothing. A
/// workflow that signs anything means the key has moved off the owner's
/// machine. An empty folder fails too, so a moved one cannot pass by reading
/// nothing.
#[test]
fn workflows_read_only_the_review_key_and_never_sign() {
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
        let review = path.file_name().is_some_and(|name| name == REVIEW_WORKFLOW);
        for (n, line) in text.lines().enumerate() {
            // Only the exact key is taken out, so a second secret on the same
            // line still fails.
            let line = if review {
                line.replace(REVIEW_KEY, "")
            } else {
                line.to_owned()
            };
            for needle in FORBIDDEN.iter().filter(|needle| line.contains(**needle)) {
                found.push(format!("{}:{}: {needle}", path.display(), n + 1));
            }
        }
    }
    assert!(workflows > 0, "{} holds no .yml file", dir.display());
    assert!(found.is_empty(), "{}", found.join("\n"));
}
