//! ConversationStore — git-backed conversation persistence (ADR 054).
//!
//! Stores session JSON files in `~/.vlinder/conversations/`, a git repository.
//! Each message (user input or agent response) is a separate commit.
//! The commit SHA becomes the SubmissionId, making the git commit chain
//! a logical clock for the entire system.

use std::path::PathBuf;
use std::process::Command;

use super::session::Session;

/// Git-backed conversation store.
///
/// Manages a git repository of session JSON files. Each commit represents
/// a single message (user input or agent response).
pub struct ConversationStore {
    dir: PathBuf,
}

/// Errors from conversation store operations.
#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Git(String),
    Json(serde_json::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "io error: {}", e),
            StoreError::Git(msg) => write!(f, "git error: {}", msg),
            StoreError::Json(e) => write!(f, "json error: {}", e),
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

impl ConversationStore {
    /// Open a conversation store at the given directory.
    ///
    /// Creates the directory and initializes a git repo if needed.
    /// Idempotent: safe to call on an existing repo.
    pub fn open(dir: PathBuf) -> Result<Self, StoreError> {
        std::fs::create_dir_all(&dir)?;

        if !dir.join(".git").exists() {
            let output = Command::new("git")
                .args(["init"])
                .current_dir(&dir)
                .output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(StoreError::Git(format!("git init failed: {}", stderr)));
            }
        }

        Ok(Self { dir })
    }

    /// Commit a user input message. Returns the commit SHA.
    ///
    /// The SHA becomes the SubmissionId — no self-referential trailer.
    /// Commit message format:
    /// ```text
    /// user
    ///
    /// <user input>
    ///
    /// Session: <session_id>
    /// ```
    pub fn commit_user_input(&self, session: &Session) -> Result<String, StoreError> {
        let filename = session.filename();
        let json = serde_json::to_string_pretty(session)?;
        let filepath = self.dir.join(&filename);
        std::fs::write(&filepath, &json)?;

        // git add
        self.git(&["add", &filename])?;

        // Build commit message
        let input = session.open.as_deref().unwrap_or("");
        let message = format!(
            "user\n\n{}\n\nSession: {}",
            input,
            session.session.as_str()
        );

        // git commit
        self.git(&["commit", "-m", &message])?;

        // Return SHA
        self.rev_parse_head()
    }

    /// Commit an agent response. Returns the commit SHA.
    ///
    /// References the SubmissionId (user input commit SHA) in a trailer.
    /// Optionally includes a State trailer linking to the state DAG (ADR 055).
    /// Commit message format:
    /// ```text
    /// agent
    ///
    /// <agent response>
    ///
    /// Session: <session_id>
    /// Submission: <submission_sha>
    /// State: <state_hash>          ← only if state is Some
    /// ```
    pub fn commit_agent_response(
        &self,
        session: &Session,
        submission_id: &str,
        state: Option<&str>,
    ) -> Result<String, StoreError> {
        let filename = session.filename();
        let json = serde_json::to_string_pretty(session)?;
        let filepath = self.dir.join(&filename);
        std::fs::write(&filepath, &json)?;

        // git add
        self.git(&["add", &filename])?;

        // Find the last agent response in history for the commit body
        let response = session.history.iter().rev()
            .find_map(|entry| {
                if let super::session::HistoryEntry::Agent { agent, .. } = entry {
                    Some(agent.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("");

        let mut message = format!(
            "agent\n\n{}\n\nSession: {}\nSubmission: {}",
            response,
            session.session.as_str(),
            submission_id
        );

        if let Some(state_hash) = state {
            message.push_str(&format!("\nState: {}", state_hash));
        }

        // git commit
        self.git(&["commit", "-m", &message])?;

        // Return SHA
        self.rev_parse_head()
    }

    /// Read the State trailer from a git commit message.
    ///
    /// Returns the state hash if the commit has a `State: <hash>` trailer.
    pub fn read_state_trailer(&self, commit_sha: &str) -> Result<Option<String>, StoreError> {
        let body = self.git(&["log", "-1", "--format=%B", commit_sha])?;
        for line in body.lines() {
            if let Some(hash) = line.strip_prefix("State: ") {
                let hash = hash.trim();
                if !hash.is_empty() {
                    return Ok(Some(hash.to_string()));
                }
            }
        }
        Ok(None)
    }

    /// Find the latest state hash for a given agent on the current timeline.
    ///
    /// Scans backwards from HEAD for agent response commits that touched
    /// this agent's session files, returns the first State trailer found.
    /// Naturally follows the current git branch — forks are transparent.
    pub fn latest_state_for_agent(&self, agent_name: &str) -> Result<Option<String>, StoreError> {
        let pathspec = format!("*_{}_*.json", agent_name);
        let output = match self.git(&[
            "log", "--format=%s%n%b%n---END---", "--", &pathspec
        ]) {
            Ok(output) => output,
            Err(_) => return Ok(None), // No history yet
        };

        for block in output.split("---END---") {
            let block = block.trim();
            if block.is_empty() { continue; }
            let lines: Vec<&str> = block.lines().collect();
            if lines.is_empty() || lines[0].trim() != "agent" { continue; }
            for line in &lines[1..] {
                if let Some(hash) = line.strip_prefix("State: ") {
                    let hash = hash.trim();
                    if !hash.is_empty() {
                        return Ok(Some(hash.to_string()));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Fork the conversation timeline at a given commit.
    ///
    /// Creates a new git branch from the target commit and switches to it.
    /// All subsequent commits land on this branch. The old timeline is preserved.
    pub fn fork_at(&self, commit: &str) -> Result<String, StoreError> {
        let short = &commit[..8.min(commit.len())];
        let branch_name = format!("fork-{}", short);
        self.git(&["checkout", "-b", &branch_name, commit])?;
        Ok(branch_name)
    }

    /// Load a session from a JSON file by filename.
    pub fn load(&self, filename: &str) -> Result<Session, StoreError> {
        let filepath = self.dir.join(filename);
        let json = std::fs::read_to_string(&filepath)?;
        let session: Session = serde_json::from_str(&json)?;
        Ok(session)
    }

    /// Run a git command in the store directory.
    fn git(&self, args: &[&str]) -> Result<String, StoreError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(StoreError::Git(format!(
                "git {} failed: {}",
                args.join(" "),
                stderr
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Get the SHA of HEAD.
    fn rev_parse_head(&self) -> Result<String, StoreError> {
        self.git(&["rev-parse", "HEAD"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{SessionId, SubmissionId};

    fn test_session() -> Session {
        Session::new(
            SessionId::from("ses-abc12345".to_string()),
            "pensieve",
        )
    }

    #[test]
    fn open_creates_git_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        assert!(tmp.path().join(".git").exists());
        // Verify it's a valid git repo
        let result = store.git(&["status"]);
        assert!(result.is_ok());
    }

    #[test]
    fn open_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Open twice — should not fail
        ConversationStore::open(tmp.path().to_path_buf()).unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        assert!(tmp.path().join(".git").exists());
        let result = store.git(&["status"]);
        assert!(result.is_ok());
    }

    #[test]
    fn commit_user_input_returns_sha() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("placeholder".to_string()));

        let sha = store.commit_user_input(&session).unwrap();

        // SHA should be valid hex (40 chars)
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn commit_returns_different_shas() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("first", SubmissionId::from("sub1".to_string()));
        let sha1 = store.commit_user_input(&session).unwrap();

        session.record_agent_response("reply to first");
        let sha2 = store.commit_agent_response(&session, &sha1, None).unwrap();

        assert_ne!(sha1, sha2);
    }

    #[test]
    fn load_round_trips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("a1b2c3d".to_string()));
        let filename = session.filename();

        store.commit_user_input(&session).unwrap();

        let loaded = store.load(&filename).unwrap();
        assert_eq!(loaded.session, session.session);
        assert_eq!(loaded.agent, session.agent);
        assert_eq!(loaded.open, Some("hello".to_string()));
        assert_eq!(loaded.history.len(), 1);
    }

    #[test]
    fn git_log_reads_like_conversation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();

        // Turn 1: user
        session.record_user_input("summarize this", SubmissionId::from("placeholder".to_string()));
        let sha1 = store.commit_user_input(&session).unwrap();

        // Turn 1: agent
        session.record_agent_response("Here's the summary...");
        store.commit_agent_response(&session, &sha1, None).unwrap();

        // Turn 2: user
        session.record_user_input("what about point 3?", SubmissionId::from("placeholder2".to_string()));
        let sha2 = store.commit_user_input(&session).unwrap();

        // Turn 2: agent
        session.record_agent_response("Point 3 discusses...");
        store.commit_agent_response(&session, &sha2, None).unwrap();

        // Verify git log
        let log = store.git(&["log", "--oneline", "--reverse"]).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 4);

        // Each line should be "sha subject"
        assert!(lines[0].contains("user"));
        assert!(lines[1].contains("agent"));
        assert!(lines[2].contains("user"));
        assert!(lines[3].contains("agent"));
    }

    #[test]
    fn commit_message_includes_session_trailer() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("placeholder".to_string()));
        store.commit_user_input(&session).unwrap();

        let log = store.git(&["log", "-1", "--format=%B"]).unwrap();
        assert!(log.contains("Session: ses-abc12345"));
    }

    #[test]
    fn agent_commit_includes_submission_trailer() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("placeholder".to_string()));
        let sha = store.commit_user_input(&session).unwrap();

        session.record_agent_response("hi there");
        store.commit_agent_response(&session, &sha, None).unwrap();

        let log = store.git(&["log", "-1", "--format=%B"]).unwrap();
        assert!(log.contains("Session: ses-abc12345"));
        assert!(log.contains(&format!("Submission: {}", sha)));
    }

    #[test]
    fn agent_commit_includes_state_trailer() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("placeholder".to_string()));
        let sha = store.commit_user_input(&session).unwrap();

        session.record_agent_response("hi there");
        let state_hash = "abc123def456";
        store.commit_agent_response(&session, &sha, Some(state_hash)).unwrap();

        let log = store.git(&["log", "-1", "--format=%B"]).unwrap();
        assert!(log.contains(&format!("State: {}", state_hash)));
    }

    #[test]
    fn read_state_trailer_returns_hash() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("placeholder".to_string()));
        let sha = store.commit_user_input(&session).unwrap();

        session.record_agent_response("hi there");
        let state_hash = "abc123def456";
        let agent_sha = store.commit_agent_response(&session, &sha, Some(state_hash)).unwrap();

        let result = store.read_state_trailer(&agent_sha).unwrap();
        assert_eq!(result, Some(state_hash.to_string()));
    }

    #[test]
    fn read_state_trailer_returns_none_for_user_commits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("placeholder".to_string()));
        let sha = store.commit_user_input(&session).unwrap();

        let result = store.read_state_trailer(&sha).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn latest_state_returns_none_with_no_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let result = store.latest_state_for_agent("pensieve").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn latest_state_finds_state_from_agent_commit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();
        session.record_user_input("hello", SubmissionId::from("sub1".to_string()));
        let sha = store.commit_user_input(&session).unwrap();

        session.record_agent_response("hi there");
        store.commit_agent_response(&session, &sha, Some("deadbeef1234")).unwrap();

        let result = store.latest_state_for_agent("pensieve").unwrap();
        assert_eq!(result, Some("deadbeef1234".to_string()));
    }

    #[test]
    fn latest_state_skips_user_commits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();

        // Agent commit with state
        session.record_user_input("hello", SubmissionId::from("sub1".to_string()));
        let sha = store.commit_user_input(&session).unwrap();
        session.record_agent_response("hi");
        store.commit_agent_response(&session, &sha, Some("state111")).unwrap();

        // Another user commit (no state)
        session.record_user_input("more", SubmissionId::from("sub2".to_string()));
        store.commit_user_input(&session).unwrap();

        // latest_state should still find state111 (skipping user commit)
        let result = store.latest_state_for_agent("pensieve").unwrap();
        assert_eq!(result, Some("state111".to_string()));
    }

    #[test]
    fn latest_state_returns_most_recent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ConversationStore::open(tmp.path().to_path_buf()).unwrap();

        let mut session = test_session();

        // First turn with state "aaa"
        session.record_user_input("first", SubmissionId::from("sub1".to_string()));
        let sha1 = store.commit_user_input(&session).unwrap();
        session.record_agent_response("reply1");
        store.commit_agent_response(&session, &sha1, Some("aaa")).unwrap();

        // Second turn with state "bbb"
        session.record_user_input("second", SubmissionId::from("sub2".to_string()));
        let sha2 = store.commit_user_input(&session).unwrap();
        session.record_agent_response("reply2");
        store.commit_agent_response(&session, &sha2, Some("bbb")).unwrap();

        // Should return the most recent state "bbb"
        let result = store.latest_state_for_agent("pensieve").unwrap();
        assert_eq!(result, Some("bbb".to_string()));
    }
}
