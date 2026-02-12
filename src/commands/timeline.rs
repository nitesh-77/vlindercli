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
    use vlindercli::domain::workers::GitDagWorker;
    use vlindercli::domain::{
        InvokeMessage, CompleteMessage, ObservableMessage,
        SubmissionId, SessionId, HarnessType,
        InvokeDiagnostics, ContainerDiagnostics,
        RuntimeType, ResourceId,
    };

    #[test]
    fn route_shows_session_commits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut worker = GitDagWorker::open(tmp.path(), "registry.local:9000", None).unwrap();

        let agent_id = ResourceId::new("http://127.0.0.1:9000/agents/agent-a");

        let invoke = InvokeMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            HarnessType::Cli,
            RuntimeType::Container,
            agent_id.clone(),
            b"q".to_vec(),
            None,
            InvokeDiagnostics { harness_version: "0.1.0".to_string(), history_turns: 0 },
        );
        let ts1 = DateTime::from_timestamp(1000, 0).unwrap();
        worker.on_observable_message(&ObservableMessage::Invoke(invoke), ts1);

        let complete = CompleteMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            agent_id,
            HarnessType::Cli,
            b"a".to_vec(),
            None,
            ContainerDiagnostics::placeholder(100),
        );
        let ts2 = DateTime::from_timestamp(1001, 0).unwrap();
        worker.on_observable_message(&ObservableMessage::Complete(complete), ts2);

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
