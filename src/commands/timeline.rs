//! Timeline subcommand — git passthrough for conversation exploration (ADR 068).
//!
//! `vlinder timeline <args>` passes arguments to `git -C <conversations_dir>`.
//! Any git subcommand works: log, show, diff, branch, bisect.
//!
//! The `route` subcommand is the only domain-specific addition — it shows
//! the full chain of messages for a session.

use std::path::Path;
use std::process::Command;

use clap::Subcommand;

use vlindercli::config::conversations_dir;

#[derive(Subcommand, Debug, PartialEq)]
pub enum TimelineCommand {
    /// Show the route for a session (all messages in order)
    Route {
        /// Session ID to show
        session_id: String,
    },
    /// Any other argument is passed directly to git
    #[command(external_subcommand)]
    Git(Vec<String>),
}

pub fn execute(cmd: TimelineCommand) {
    let dir = conversations_dir();

    if !dir.join(".git").exists() {
        eprintln!("No conversations yet. Run an agent first.");
        return;
    }

    match cmd {
        TimelineCommand::Route { session_id } => route(&dir, &session_id),
        TimelineCommand::Git(args) => passthrough(&dir, &args),
    }
}

/// Show the route for a session — all commits matching the session trailer.
fn route(dir: &Path, session_id: &str) {
    let grep_pattern = format!("Session: {}", session_id);
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["log", "--oneline", "--reverse", "--grep"])
        .arg(&grep_pattern)
        .status();

    match status {
        Ok(s) if !s.success() => {
            eprintln!("Session '{}' not found.", session_id);
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
        }
        _ => {}
    }
}

/// Pass arguments directly to git operating on the conversations repo.
fn passthrough(dir: &Path, args: &[String]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status();

    match status {
        Ok(s) if !s.success() => {
            std::process::exit(s.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
            std::process::exit(1);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use vlindercli::domain::workers::{DagWorker, GitDagWorker};
    use vlindercli::domain::{DagNode, MessageType, hash_dag_node};

    fn test_node(
        payload: &[u8],
        parent_hash: &str,
        message_type: MessageType,
        from: &str,
        to: &str,
        session_id: &str,
    ) -> DagNode {
        DagNode {
            hash: hash_dag_node(payload, parent_hash, &message_type, b""),
            parent_hash: parent_hash.to_string(),
            message_type,
            from: from.to_string(),
            to: to.to_string(),
            session_id: session_id.to_string(),
            submission_id: "sub-1".to_string(),
            payload: payload.to_vec(),
            diagnostics: Vec::new(),
            stderr: Vec::new(),
            created_at: DateTime::from_timestamp(1000, 0).unwrap(),
            state: None,
            protocol_version: String::new(),
        }
    }

    #[test]
    fn route_shows_session_commits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut worker = GitDagWorker::open(tmp.path(), "registry.local:9000", None).unwrap();

        let n1 = test_node(b"q", "", MessageType::Invoke, "cli", "agent-a", "sess-1");
        worker.on_message(&n1);
        let n2 = test_node(b"a", &n1.hash, MessageType::Complete, "agent-a", "cli", "sess-1");
        worker.on_message(&n2);

        // Both commits are on main, filterable by Session trailer
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["log", "--oneline", "--grep", "Session: sess-1", "main"])
            .output()
            .unwrap();

        let log = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = log.trim().lines().collect();
        assert_eq!(lines.len(), 2);
    }
}
