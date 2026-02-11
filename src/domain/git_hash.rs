//! In-process SHA-1 computation using git's object format (ADR 070).
//!
//! Git hashes objects as `SHA-1("{type} {len}\0{content}")`. This module
//! replicates that algorithm so the harness can compute SubmissionIds
//! without shelling out to git.

use sha1::{Digest, Sha1};

/// Compute the SHA-1 hash of a git object.
///
/// Equivalent to `git hash-object -t {object_type} --stdin < content`.
/// Format: `SHA-1("{object_type} {content_len}\0{content}")`.
pub fn git_hash_object(object_type: &str, content: &[u8]) -> String {
    let header = format!("{} {}\0", object_type, content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Compute a SubmissionId as a git commit SHA-1.
///
/// Builds a blob→tree→commit chain using git's exact byte format and
/// returns the commit hash. The result is deterministic and git-compatible.
///
/// # Arguments
/// * `payload` — the message content (becomes a blob named "payload")
/// * `parent_hash` — parent commit SHA (empty string for root commits)
/// * `author` — author identity string, e.g. "cli <cli@localhost>"
/// * `timestamp` — unix timestamp for both author and committer dates
/// * `message` — commit message
pub fn compute_submission_id(
    payload: &[u8],
    parent_hash: &str,
    author: &str,
    timestamp: i64,
    message: &str,
) -> String {
    // 1. Hash payload as a blob
    let blob_hash = git_hash_object("blob", payload);
    let blob_raw = hex_to_bytes(&blob_hash);

    // 2. Build tree with single "payload" entry
    //    Git tree format: "100644 payload\0{20_raw_bytes}"
    let mut tree_content = Vec::new();
    tree_content.extend_from_slice(b"100644 payload\0");
    tree_content.extend_from_slice(&blob_raw);
    let tree_hash = git_hash_object("tree", &tree_content);

    // 3. Build commit object
    let mut commit = format!("tree {}\n", tree_hash);
    if !parent_hash.is_empty() {
        commit.push_str(&format!("parent {}\n", parent_hash));
    }
    commit.push_str(&format!(
        "author {} {} +0000\ncommitter {} {} +0000\n\n{}\n",
        author, timestamp, author, timestamp, message,
    ));

    git_hash_object("commit", commit.as_bytes())
}

/// Convert a hex string to raw bytes (40-char hex → 20 bytes).
fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// Encode raw bytes as lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// Re-export for use by sha1 — the `hex` crate isn't a dependency,
// so we use sha1's own Digest trait which gives us finalize().
// We manually encode to hex above.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        super::hex_encode(bytes.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_hash_matches_known_git_sha() {
        // `echo "hello" | git hash-object --stdin` = ce013625030ba8dba906f756967f9e9ca394464a
        // Note: echo adds a trailing newline, so "hello\n" is 6 bytes
        let hash = git_hash_object("blob", b"hello\n");
        assert_eq!(hash, "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn blob_hash_empty_content() {
        // `echo -n "" | git hash-object --stdin` = e69de29bb2d1d6434b8b29ae775ad8c2e48c5391
        let hash = git_hash_object("blob", b"");
        assert_eq!(hash, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn root_commit_produces_40_char_hex() {
        let hash = compute_submission_id(
            b"test payload",
            "",
            "cli <cli@localhost>",
            1700000000,
            "test commit",
        );
        assert_eq!(hash.len(), 40);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn chained_commit_differs_from_root() {
        let root = compute_submission_id(
            b"first",
            "",
            "cli <cli@localhost>",
            1700000000,
            "root",
        );

        let chained = compute_submission_id(
            b"second",
            &root,
            "cli <cli@localhost>",
            1700000001,
            "chained",
        );

        assert_ne!(root, chained);
        assert_eq!(chained.len(), 40);
    }

    #[test]
    fn deterministic_same_inputs_same_output() {
        let h1 = compute_submission_id(
            b"payload",
            "",
            "cli <cli@localhost>",
            1700000000,
            "test",
        );

        let h2 = compute_submission_id(
            b"payload",
            "",
            "cli <cli@localhost>",
            1700000000,
            "test",
        );

        assert_eq!(h1, h2);
    }

    #[test]
    fn different_payloads_different_hashes() {
        let h1 = compute_submission_id(
            b"payload-a",
            "",
            "cli <cli@localhost>",
            1700000000,
            "test",
        );

        let h2 = compute_submission_id(
            b"payload-b",
            "",
            "cli <cli@localhost>",
            1700000000,
            "test",
        );

        assert_ne!(h1, h2);
    }

    #[test]
    fn hex_to_bytes_round_trip() {
        let original = vec![0xce, 0x01, 0x36, 0x25];
        let hex_str = hex_encode(&original);
        let back = hex_to_bytes(&hex_str);
        assert_eq!(original, back);
    }

    #[test]
    fn round_trip_with_git_plumbing() {
        // This test verifies our in-process computation matches actual git.
        // It creates a temp repo, writes the same content with git plumbing,
        // and compares the SHAs.
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();

        // git init
        let status = std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(status.status.success());

        // Write blob via git
        let payload = b"round-trip test payload";
        let mut child = std::process::Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let output = child.wait_with_output().unwrap();
        let git_blob_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Our in-process blob hash should match
        let our_blob_hash = git_hash_object("blob", payload);
        assert_eq!(our_blob_hash, git_blob_hash);
    }

    use std::io::Write;
}
