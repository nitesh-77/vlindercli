//! Shared helpers for integration tests (ADR 082).
//!
//! Each integration test binary includes this via `mod common;`.
//! Provides isolated VLINDER_DIR, conversation repo setup, and git helpers.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Utc};

use vlindercli::domain::workers::GitDagWorker;
use vlindercli::domain::{
    CompleteMessage, ContainerDiagnostics, HarnessType, InvokeDiagnostics, InvokeMessage,
    ObservableMessage, ResourceId, RuntimeType, SessionId, SubmissionId,
};

/// Create an isolated VLINDER_DIR for this test.
///
/// Reads VLINDER_INTEGRATION_RUN, creates `<run_dir>/<test_name>/.vlinder/`,
/// sets VLINDER_DIR, and prints the path to stderr.
pub fn test_vlinder_dir(test_name: &str) -> PathBuf {
    let run_dir = std::env::var("VLINDER_INTEGRATION_RUN")
        .expect("VLINDER_INTEGRATION_RUN not set — use `just run-integration-tests`");
    let test_dir = PathBuf::from(run_dir).join(test_name).join(".vlinder");
    std::fs::create_dir_all(&test_dir).unwrap();
    std::env::set_var("VLINDER_DIR", &test_dir);
    eprintln!("VLINDER_DIR={}", test_dir.display());
    test_dir
}

/// Create a test conversations repo using GitDagWorker.
///
/// Opens a GitDagWorker at `<vlinder_dir>/conversations/` and returns it.
/// Caller uses `on_observable_message()` to populate the repo.
pub fn test_conversations_worker(vlinder_dir: &Path) -> GitDagWorker {
    let conv_dir = vlinder_dir.join("conversations");
    GitDagWorker::open(&conv_dir, "test.local:9000", None).unwrap()
}

/// Conversations directory path for a given VLINDER_DIR.
pub fn conversations_path(vlinder_dir: &Path) -> PathBuf {
    vlinder_dir.join("conversations")
}

// ============================================================================
// Git helpers
// ============================================================================

/// Run a git command in the given directory and return stdout.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git {} failed: {}", args.join(" "), e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {} failed: {}", args.join(" "), stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Read a trailer value from a git commit.
pub fn read_trailer(dir: &Path, commit: &str, key: &str) -> Option<String> {
    let format = format!("%(trailers:key={},valueonly)", key);
    let value = git(dir, &["log", "-1", &format!("--format={}", format), commit]).ok()?;
    if value.is_empty() { None } else { Some(value) }
}

/// Get the current HEAD commit SHA.
pub fn read_head_sha(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "--verify", "HEAD"]).ok()
}

// ============================================================================
// Message factory helpers
// ============================================================================

fn test_agent_id() -> ResourceId {
    ResourceId::new("http://127.0.0.1:9000/agents/test-agent")
}

/// Create an invoke message with the given payload and state.
pub fn make_invoke(
    session: &str,
    submission: &str,
    payload: &[u8],
    state: Option<String>,
    epoch_secs: i64,
) -> (ObservableMessage, DateTime<Utc>) {
    let msg = InvokeMessage::new(
        SubmissionId::from(submission.to_string()),
        SessionId::from(session.to_string()),
        HarnessType::Cli,
        RuntimeType::Container,
        test_agent_id(),
        payload.to_vec(),
        state,
        InvokeDiagnostics {
            harness_version: "0.1.0".to_string(),
            history_turns: 0,
        },
    );
    let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
    (ObservableMessage::Invoke(msg), created_at)
}

/// Create a complete message with the given payload and state.
pub fn make_complete(
    session: &str,
    submission: &str,
    payload: &[u8],
    state: Option<String>,
    epoch_secs: i64,
) -> (ObservableMessage, DateTime<Utc>) {
    let msg = CompleteMessage::new(
        SubmissionId::from(submission.to_string()),
        SessionId::from(session.to_string()),
        test_agent_id(),
        HarnessType::Cli,
        payload.to_vec(),
        state,
        ContainerDiagnostics::placeholder(100),
    );
    let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
    (ObservableMessage::Complete(msg), created_at)
}
