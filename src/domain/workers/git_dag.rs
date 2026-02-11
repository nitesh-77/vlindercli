//! GitDagWorker — writes DAG nodes as git commits (ADR 064, 069).
//!
//! Each DagNode becomes a commit. The author is the message sender (ADR 069).
//! The commit tree holds the payload as a blob. Sessions are branches under
//! `refs/heads/sessions/<session_id>`.
//!
//! Uses git plumbing commands (hash-object, mktree, commit-tree, update-ref)
//! to work entirely in `.git/objects/` without touching the working directory.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::storage::dag_store::DagNode;
use super::dag::DagWorker;

/// DAG worker that writes commits to a git repository.
pub struct GitDagWorker {
    repo_path: PathBuf,
    registry_host: String,
    /// Last git commit hash per session — for commit chaining.
    last_commit: HashMap<String, String>,
}

impl GitDagWorker {
    /// Open (or create) a git repo for DAG commits.
    pub fn open(repo_path: &Path, registry_host: &str) -> Result<Self, String> {
        std::fs::create_dir_all(repo_path)
            .map_err(|e| format!("failed to create repo directory: {}", e))?;

        if !repo_path.join(".git").exists() {
            let output = Command::new("git")
                .args(["init"])
                .current_dir(repo_path)
                .output()
                .map_err(|e| format!("git init failed: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("git init failed: {}", stderr));
            }
        }

        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            registry_host: registry_host.to_string(),
            last_commit: HashMap::new(),
        })
    }

    /// Write a blob to the git object store. Returns the blob hash.
    fn write_blob(&self, data: &[u8]) -> Result<String, String> {
        let mut child = Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(&self.repo_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git hash-object spawn failed: {}", e))?;

        child.stdin.take().unwrap().write_all(data)
            .map_err(|e| format!("git hash-object write failed: {}", e))?;

        let output = child.wait_with_output()
            .map_err(|e| format!("git hash-object wait failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git hash-object failed: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Create a tree containing a single `payload` blob. Returns the tree hash.
    fn make_tree(&self, blob_hash: &str) -> Result<String, String> {
        let entry = format!("100644 blob {}\tpayload\n", blob_hash);

        let mut child = Command::new("git")
            .args(["mktree"])
            .current_dir(&self.repo_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git mktree spawn failed: {}", e))?;

        child.stdin.take().unwrap().write_all(entry.as_bytes())
            .map_err(|e| format!("git mktree write failed: {}", e))?;

        let output = child.wait_with_output()
            .map_err(|e| format!("git mktree wait failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git mktree failed: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Create a commit from a tree. Returns the commit hash.
    fn commit_tree(
        &self,
        tree_hash: &str,
        parent: Option<&str>,
        message: &str,
        author_name: &str,
        author_email: &str,
        author_date: &str,
    ) -> Result<String, String> {
        let mut args = vec!["commit-tree", tree_hash];
        let parent_owned;
        if let Some(p) = parent {
            args.push("-p");
            parent_owned = p.to_string();
            args.push(&parent_owned);
        }
        args.push("-m");
        args.push(message);

        let date_value = format!("@{} +0000", author_date);

        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.repo_path)
            .env("GIT_AUTHOR_NAME", author_name)
            .env("GIT_AUTHOR_EMAIL", author_email)
            .env("GIT_AUTHOR_DATE", &date_value)
            .env("GIT_COMMITTER_NAME", "vlinder")
            .env("GIT_COMMITTER_EMAIL", "vlinder@localhost")
            .output()
            .map_err(|e| format!("git commit-tree failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git commit-tree failed: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Update a ref to point to a commit.
    fn update_ref(&self, refname: &str, commit_hash: &str) -> Result<(), String> {
        let output = Command::new("git")
            .args(["update-ref", refname, commit_hash])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| format!("git update-ref failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git update-ref failed: {}", stderr));
        }

        Ok(())
    }

    /// Run a git command and return stdout.
    #[cfg(test)]
    fn git(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| format!("git {} failed: {}", args.join(" "), e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git {} failed: {}", args.join(" "), stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

impl DagWorker for GitDagWorker {
    fn on_message(&mut self, node: &DagNode) {
        let result = (|| -> Result<(), String> {
            // 1. Write payload blob
            let blob_hash = self.write_blob(&node.payload)?;

            // 2. Create tree with payload blob
            let tree_hash = self.make_tree(&blob_hash)?;

            // 3. Get parent commit for this session
            let parent = self.last_commit.get(&node.session_id).map(|s| s.as_str());

            // 4. Build commit message
            let message = format!(
                "{}: {} \u{2192} {}\n\nSession: {}\nSubmission: {}\nHash: {}",
                node.message_type.as_str(),
                node.from,
                node.to,
                node.session_id,
                node.submission_id,
                node.hash,
            );

            // 5. Author = message sender (ADR 069)
            let author_email = format!("{}@{}", node.from, self.registry_host);

            // 6. Create commit
            let commit_hash = self.commit_tree(
                &tree_hash,
                parent,
                &message,
                &node.from,
                &author_email,
                &node.created_at,
            )?;

            // 7. Update session branch ref
            let refname = format!("refs/heads/sessions/{}", node.session_id);
            self.update_ref(&refname, &commit_hash)?;

            // 8. Track last commit
            self.last_commit.insert(node.session_id.clone(), commit_hash);

            Ok(())
        })();

        if let Err(e) = result {
            tracing::error!(error = %e, hash = %node.hash, "Failed to write git commit");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::dag_store::{MessageType, hash_dag_node};

    fn test_node(
        payload: &[u8],
        parent_hash: &str,
        message_type: MessageType,
        from: &str,
        to: &str,
    ) -> DagNode {
        DagNode {
            hash: hash_dag_node(payload, parent_hash, &message_type),
            parent_hash: parent_hash.to_string(),
            message_type,
            from: from.to_string(),
            to: to.to_string(),
            session_id: "sess-1".to_string(),
            submission_id: "sub-1".to_string(),
            payload: payload.to_vec(),
            created_at: "1000".to_string(),
        }
    }

    fn test_worker() -> (GitDagWorker, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let worker = GitDagWorker::open(tmp.path(), "registry.local:9000").unwrap();
        (worker, tmp)
    }

    #[test]
    fn open_creates_git_repo() {
        let (_worker, tmp) = test_worker();
        assert!(tmp.path().join(".git").exists());
    }

    #[test]
    fn open_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        GitDagWorker::open(tmp.path(), "host").unwrap();
        GitDagWorker::open(tmp.path(), "host").unwrap();
        assert!(tmp.path().join(".git").exists());
    }

    #[test]
    fn commit_creates_session_branch() {
        let (mut worker, _tmp) = test_worker();
        let node = test_node(b"hello", "", MessageType::Invoke, "cli", "agent-a");

        worker.on_message(&node);

        let sha = worker.git(&["rev-parse", "--verify", "sessions/sess-1"]).unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn commit_message_first_line() {
        let (mut worker, _tmp) = test_worker();
        let node = test_node(b"payload", "", MessageType::Invoke, "cli", "support-agent");

        worker.on_message(&node);

        let subject = worker.git(&["log", "-1", "--format=%s", "sessions/sess-1"]).unwrap();
        assert_eq!(subject, "invoke: cli \u{2192} support-agent");
    }

    #[test]
    fn commit_message_trailers() {
        let (mut worker, _tmp) = test_worker();
        let node = test_node(b"payload", "", MessageType::Invoke, "cli", "support-agent");
        let expected_hash = node.hash.clone();

        worker.on_message(&node);

        let body = worker.git(&["log", "-1", "--format=%b", "sessions/sess-1"]).unwrap();
        assert!(body.contains("Session: sess-1"), "body: {}", body);
        assert!(body.contains("Submission: sub-1"), "body: {}", body);
        assert!(body.contains(&format!("Hash: {}", expected_hash)), "body: {}", body);
    }

    #[test]
    fn author_is_message_sender() {
        let (mut worker, _tmp) = test_worker();
        let node = test_node(b"data", "", MessageType::Invoke, "cli", "agent-a");

        worker.on_message(&node);

        let author = worker.git(&["log", "-1", "--format=%an <%ae>", "sessions/sess-1"]).unwrap();
        assert_eq!(author, "cli <cli@registry.local:9000>");
    }

    #[test]
    fn committer_is_platform() {
        let (mut worker, _tmp) = test_worker();
        let node = test_node(b"data", "", MessageType::Invoke, "cli", "agent-a");

        worker.on_message(&node);

        let committer = worker.git(&["log", "-1", "--format=%cn <%ce>", "sessions/sess-1"]).unwrap();
        assert_eq!(committer, "vlinder <vlinder@localhost>");
    }

    #[test]
    fn author_date_matches_node() {
        let (mut worker, _tmp) = test_worker();
        let mut node = test_node(b"data", "", MessageType::Invoke, "cli", "agent-a");
        node.created_at = "1700000000".to_string();

        worker.on_message(&node);

        let date = worker.git(&["log", "-1", "--format=%at", "sessions/sess-1"]).unwrap();
        assert_eq!(date, "1700000000");
    }

    #[test]
    fn payload_stored_as_blob() {
        let (mut worker, _tmp) = test_worker();
        let node = test_node(b"my-payload-data", "", MessageType::Invoke, "cli", "agent-a");

        worker.on_message(&node);

        let commit = worker.git(&["rev-parse", "sessions/sess-1"]).unwrap();

        // Tree should contain a "payload" entry
        let tree = worker.git(&["log", "-1", "--format=%T", "sessions/sess-1"]).unwrap();
        let ls_tree = worker.git(&["ls-tree", &tree]).unwrap();
        assert!(ls_tree.contains("payload"), "ls-tree: {}", ls_tree);

        // Read the blob content
        let content = worker.git(&["show", &format!("{}:payload", commit)]).unwrap();
        assert_eq!(content, "my-payload-data");
    }

    #[test]
    fn commits_chain_within_session() {
        let (mut worker, _tmp) = test_worker();

        let n1 = test_node(b"first", "", MessageType::Invoke, "cli", "agent-a");
        worker.on_message(&n1);
        let commit1 = worker.git(&["rev-parse", "sessions/sess-1"]).unwrap();

        let n2 = test_node(b"second", &n1.hash, MessageType::Request, "agent-a", "infer.ollama");
        worker.on_message(&n2);
        let commit2 = worker.git(&["rev-parse", "sessions/sess-1"]).unwrap();

        assert_ne!(commit1, commit2);

        // commit2's parent should be commit1
        let parent = worker.git(&["log", "-1", "--format=%P", "sessions/sess-1"]).unwrap();
        assert_eq!(parent, commit1);
    }

    #[test]
    fn first_commit_is_root() {
        let (mut worker, _tmp) = test_worker();
        let node = test_node(b"first", "", MessageType::Invoke, "cli", "agent-a");

        worker.on_message(&node);

        // Root commit has no parent
        let parent = worker.git(&["log", "-1", "--format=%P", "sessions/sess-1"]).unwrap();
        assert_eq!(parent, "");
    }

    #[test]
    fn different_sessions_get_different_branches() {
        let (mut worker, _tmp) = test_worker();

        let mut n1 = test_node(b"sess1", "", MessageType::Invoke, "cli", "agent-a");
        n1.session_id = "sess-1".to_string();
        worker.on_message(&n1);

        let mut n2 = test_node(b"sess2", "", MessageType::Invoke, "cli", "agent-b");
        n2.session_id = "sess-2".to_string();
        worker.on_message(&n2);

        let c1 = worker.git(&["rev-parse", "sessions/sess-1"]).unwrap();
        let c2 = worker.git(&["rev-parse", "sessions/sess-2"]).unwrap();
        assert_ne!(c1, c2);

        // Each is a root commit (no parent)
        let p1 = worker.git(&["log", "-1", "--format=%P", "sessions/sess-1"]).unwrap();
        let p2 = worker.git(&["log", "-1", "--format=%P", "sessions/sess-2"]).unwrap();
        assert_eq!(p1, "");
        assert_eq!(p2, "");
    }

    #[test]
    fn git_log_shows_route() {
        let (mut worker, _tmp) = test_worker();

        let n1 = test_node(b"q", "", MessageType::Invoke, "cli", "support-agent");
        worker.on_message(&n1);

        let n2 = test_node(b"r", &n1.hash, MessageType::Request, "support-agent", "infer.ollama");
        worker.on_message(&n2);

        let n3 = test_node(b"a", &n2.hash, MessageType::Response, "infer.ollama", "support-agent");
        worker.on_message(&n3);

        let n4 = test_node(b"done", &n3.hash, MessageType::Complete, "support-agent", "cli");
        worker.on_message(&n4);

        let log = worker.git(&["log", "--oneline", "--reverse", "sessions/sess-1"]).unwrap();
        let lines: Vec<&str> = log.lines().collect();

        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("invoke: cli"), "line 0: {}", lines[0]);
        assert!(lines[1].contains("request: support-agent"), "line 1: {}", lines[1]);
        assert!(lines[2].contains("response: infer.ollama"), "line 2: {}", lines[2]);
        assert!(lines[3].contains("complete: support-agent"), "line 3: {}", lines[3]);
    }

    #[test]
    fn git_log_author_filters_by_agent() {
        let (mut worker, _tmp) = test_worker();

        let n1 = test_node(b"q", "", MessageType::Invoke, "cli", "agent-a");
        worker.on_message(&n1);

        let n2 = test_node(b"r", &n1.hash, MessageType::Request, "agent-a", "infer.ollama");
        worker.on_message(&n2);

        let n3 = test_node(b"a", &n2.hash, MessageType::Response, "infer.ollama", "agent-a");
        worker.on_message(&n3);

        let n4 = test_node(b"done", &n3.hash, MessageType::Complete, "agent-a", "cli");
        worker.on_message(&n4);

        // --author=agent-a should show only agent-a's commits
        let log = worker.git(&[
            "log", "--oneline", "--author=agent-a", "sessions/sess-1",
        ]).unwrap();
        let lines: Vec<&str> = log.lines().collect();

        assert_eq!(lines.len(), 2, "expected 2 agent-a commits, got: {:?}", lines);
        assert!(lines[0].contains("complete: agent-a"));
        assert!(lines[1].contains("request: agent-a"));
    }

    #[test]
    fn git_shortlog_shows_activity_per_agent() {
        let (mut worker, _tmp) = test_worker();

        let n1 = test_node(b"q", "", MessageType::Invoke, "cli", "agent-a");
        worker.on_message(&n1);

        let n2 = test_node(b"r", &n1.hash, MessageType::Complete, "agent-a", "cli");
        worker.on_message(&n2);

        let shortlog = worker.git(&["shortlog", "-sn", "sessions/sess-1"]).unwrap();
        assert!(shortlog.contains("cli"), "shortlog: {}", shortlog);
        assert!(shortlog.contains("agent-a"), "shortlog: {}", shortlog);
    }

    #[test]
    fn all_five_message_types_produce_commits() {
        let (mut worker, _tmp) = test_worker();

        let n1 = test_node(b"1", "", MessageType::Invoke, "cli", "a");
        worker.on_message(&n1);

        let n2 = test_node(b"2", &n1.hash, MessageType::Request, "a", "infer.ollama");
        worker.on_message(&n2);

        let n3 = test_node(b"3", &n2.hash, MessageType::Response, "infer.ollama", "a");
        worker.on_message(&n3);

        let n4 = test_node(b"4", &n3.hash, MessageType::Delegate, "a", "b");
        worker.on_message(&n4);

        let n5 = test_node(b"5", &n4.hash, MessageType::Complete, "a", "cli");
        worker.on_message(&n5);

        let count = worker.git(&["rev-list", "--count", "sessions/sess-1"]).unwrap();
        assert_eq!(count, "5");
    }
}
