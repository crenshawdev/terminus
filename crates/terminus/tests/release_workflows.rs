//! The release workflows build and draft, and the owner signs and publishes
//! from his own machine. These tests keep it that way.

use std::path::Path;

/// Reading a secret, by either syntax, running a signing step, or holding a
/// private key.
const FORBIDDEN: [&str; 4] = ["secrets.", "secrets[", "-Y sign", "PRIVATE KEY"];

/// The one line that may read a secret, and the one workflow it may sit in.
/// The whole line has to match, so a secret whose name only starts with the
/// review key's name is still refused.
const REVIEW_KEY_LINE: &str = "ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}";
const REVIEW_WORKFLOW: &str = "fast.yml";

/// What one line of a workflow holds that it must not.
fn forbidden_in(workflow: &str, line: &str) -> Vec<&'static str> {
    if workflow == REVIEW_WORKFLOW && line.trim() == REVIEW_KEY_LINE {
        return Vec::new();
    }
    FORBIDDEN.into_iter().filter(|needle| line.contains(needle)).collect()
}

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
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            for needle in forbidden_in(name, line) {
                found.push(format!("{}:{}: {needle}", path.display(), n + 1));
            }
        }
    }
    assert!(workflows > 0, "{} holds no .yml file", dir.display());
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// The review-key line passes only whole and only in `fast.yml`. A secret
/// whose name starts with the review key's name is a different secret, and
/// the review key under another name is a second place it can leak from.
#[test]
fn only_the_whole_review_key_line_passes() {
    let allowed = "          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}";
    assert!(forbidden_in("fast.yml", allowed).is_empty());
    assert_eq!(forbidden_in("release.yml", allowed), ["secrets."]);
    for line in [
        "          SIGNING_KEY: ${{ secrets.ANTHROPIC_API_KEY_RELEASE }}",
        "          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}${{ secrets.OTHER }}",
        "          KEY: ${{ secrets.ANTHROPIC_API_KEY }}",
    ] {
        assert_eq!(forbidden_in("fast.yml", line), ["secrets."], "{line}");
    }
}
