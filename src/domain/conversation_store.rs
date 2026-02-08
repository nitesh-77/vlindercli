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
    /// Commit message format:
    /// ```text
    /// agent
    ///
    /// <agent response>
    ///
    /// Session: <session_id>
    /// Submission: <submission_sha>
    /// ```
    pub fn commit_agent_response(
        &self,
        session: &Session,
        submission_id: &str,
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

        let message = format!(
            "agent\n\n{}\n\nSession: {}\nSubmission: {}",
            response,
            session.session.as_str(),
            submission_id
        );

        // git commit
        self.git(&["commit", "-m", &message])?;

        // Return SHA
        self.rev_parse_head()
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
        let sha2 = store.commit_agent_response(&session, &sha1).unwrap();

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
        store.commit_agent_response(&session, &sha1).unwrap();

        // Turn 2: user
        session.record_user_input("what about point 3?", SubmissionId::from("placeholder2".to_string()));
        let sha2 = store.commit_user_input(&session).unwrap();

        // Turn 2: agent
        session.record_agent_response("Point 3 discusses...");
        store.commit_agent_response(&session, &sha2).unwrap();

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
        store.commit_agent_response(&session, &sha).unwrap();

        let log = store.git(&["log", "-1", "--format=%B"]).unwrap();
        assert!(log.contains("Session: ses-abc12345"));
        assert!(log.contains(&format!("Submission: {}", sha)));
    }
}
