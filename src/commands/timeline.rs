//! Timeline subcommand — git passthrough for conversation exploration (ADR 068, ADR 081).
//!
//! `vlinder timeline <args>` passes arguments to `git -C <conversations_dir>`.
//! Any git subcommand works: log, show, diff, branch, bisect.
//!
//! Domain-specific subcommands:
//! - `route` — show the full chain of messages for a session
//! - `checkout` — move HEAD to a specific commit (ADR 081)
//! - `repair` — branch from detached HEAD and re-execute (ADR 081)
//! - `promote` — make the current branch "main" (ADR 081)

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Subcommand;

use vlindercli::config::{Config, conversations_dir};
use vlindercli::domain::{AgentManifest, TimelineId, agent_routing_key};

use super::connect::{connect_harness, connect_registry, open_dag_store};

#[derive(Subcommand, Debug, PartialEq)]
pub enum TimelineCommand {
    /// Show the route for a session (all messages in order)
    Route {
        /// Session ID to show
        session_id: String,
    },
    /// Move HEAD to a specific point in the timeline
    Checkout {
        /// Git commit SHA or ref to checkout
        target: String,
    },
    /// Branch from current position and re-execute the agent
    Repair {
        /// Agent directory path (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Promote the current branch to main
    Promote,
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
        TimelineCommand::Checkout { target } => checkout(&dir, &target),
        TimelineCommand::Repair { path } => repair(&dir, path),
        TimelineCommand::Promote => promote(&dir),
        TimelineCommand::Git(args) => passthrough(&dir, &args),
    }
}

// ============================================================================
// Trailer reading helpers (ADR 081)
// ============================================================================

/// Read a trailer value from a git commit.
///
/// Uses `git log --format="%(trailers:key=<key>,valueonly)"` to extract
/// a specific trailer. Returns `None` if the trailer is missing or empty.
fn read_trailer(dir: &Path, commit: &str, key: &str) -> Option<String> {
    let format = format!("%(trailers:key={},valueonly)", key);
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["log", "-1", &format!("--format={}", format), commit])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

/// Get the current HEAD commit SHA.
fn read_head_sha(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ============================================================================
// Subcommand implementations
// ============================================================================

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

/// Checkout a specific point in the timeline (ADR 081).
///
/// Moves HEAD to the target commit (detached), persists the checked-out
/// state to the DAG store so that `agent run` resumes from it, and prints
/// trailer information so the user knows where they are.
fn checkout(dir: &Path, target: &str) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["checkout", target])
        .status();

    match status {
        Ok(s) if !s.success() => {
            eprintln!("Failed to checkout '{}'.", target);
            return;
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
            return;
        }
        _ => {}
    }

    // Read commit subject line
    let subject = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["log", "-1", "--format=%s", "HEAD"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    if let Some(ref subj) = subject {
        println!("At: {}", subj);
    }

    // Print trailers
    if let Some(session) = read_trailer(dir, "HEAD", "Session") {
        println!("Session:    {}", session);
    }
    if let Some(submission) = read_trailer(dir, "HEAD", "Submission") {
        println!("Submission: {}", submission);
    }
    if let Some(state) = read_trailer(dir, "HEAD", "State") {
        println!("State:      {}", state);

        // Persist checked-out state to the DAG store so that `agent run`
        // picks it up via `latest_state()`. Parse agent name from the
        // commit subject: "complete: <agent> → <harness>".
        if let Some(ref subj) = subject {
            if let Some(agent_name) = parse_agent_from_subject(subj) {
                if let Some(store) = open_dag_store(&Config::load()) {
                    match store.set_checkout_state(&agent_name, &state) {
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!("Warning: failed to persist checkout state: {}", e);
                        }
                    }
                }
            }
        }
    }
}

/// Parse agent name from a complete commit subject line.
///
/// Subject format: "complete: <agent> → <harness>"
/// Returns the agent routing key (e.g., "todoapp").
fn parse_agent_from_subject(subject: &str) -> Option<String> {
    let rest = subject.strip_prefix("complete: ")?;
    let agent = rest.split(" →").next()?;
    let agent = agent.trim();
    if agent.is_empty() { None } else { Some(agent.to_string()) }
}

/// Repair from current position — create a branch and re-execute (ADR 081).
///
/// Expects HEAD to be detached (from a prior `checkout`). Creates a
/// `repair-YYYY-MM-DD` branch, restores the agent's state from the
/// State: trailer, and enters the REPL for re-execution.
fn repair(dir: &Path, path: Option<PathBuf>) {
    // 1. Read HEAD
    let Some(head_sha) = read_head_sha(dir) else {
        eprintln!("Cannot read HEAD. Is there a conversation?");
        return;
    };

    // 2. Read trailers
    let session_trailer = read_trailer(dir, "HEAD", "Session");
    let state_trailer = read_trailer(dir, "HEAD", "State");
    let submission_trailer = read_trailer(dir, "HEAD", "Submission");

    // 3. Verify detached HEAD
    let symbolic = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output();

    let is_detached = match symbolic {
        Ok(ref o) => !o.status.success(),
        Err(_) => true,
    };

    if !is_detached {
        eprintln!("HEAD is on a branch — repair only works from a detached HEAD.");
        eprintln!("Use 'vlinder timeline checkout <sha>' first.");
        return;
    }

    // 4. Create repair branch with unique name (repair-YYYY-MM-DD-N)
    let config = Config::load();
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let dag_store = open_dag_store(&config);

    // Count existing repair branches for today to generate unique suffix
    let counter = dag_store.as_ref()
        .and_then(|store| {
            // Count timelines with branch_name like repair-{date}-%
            // We don't have a count method, so iterate by trying names
            let mut n = 0;
            loop {
                n += 1;
                let name = format!("repair-{}-{}", date, n);
                match store.get_timeline_by_branch(&name) {
                    Ok(Some(_)) => continue,
                    _ => break,
                }
            }
            Some(n)
        })
        .unwrap_or(1);

    let branch_name = format!("repair-{}-{}", date, counter);
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["checkout", "-b", &branch_name])
        .status();

    match status {
        Ok(s) if !s.success() => {
            eprintln!("Failed to create branch '{}'.", branch_name);
            return;
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
            return;
        }
        _ => {}
    }

    // Insert timeline row (ADR 093)
    let timeline_id = if let Some(ref store) = dag_store {
        if let Err(e) = store.ensure_main_timeline() {
            eprintln!("Warning: failed to ensure main timeline: {}", e);
        }
        match store.create_timeline(&branch_name, Some(1), Some(&head_sha)) {
            Ok(id) => {
                println!("Timeline {} created for branch '{}'.", id, branch_name);
                Some(id)
            }
            Err(e) => {
                eprintln!("Warning: failed to create timeline row: {}", e);
                None
            }
        }
    } else {
        None
    };

    println!("Created branch '{}' from {}", branch_name, &head_sha[..8.min(head_sha.len())]);

    if let Some(ref session) = session_trailer {
        println!("Session:    {}", session);
    }
    if let Some(ref submission) = submission_trailer {
        println!("Submission: {}", submission);
    }
    if let Some(ref state) = state_trailer {
        println!("State:      {}", state);
    }

    // 5-8. Deploy agent via registry, connect harness via gRPC, enter REPL
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let absolute_path = match agent_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to resolve agent path: {}", e);
            return;
        }
    };

    // Deploy via registry gRPC (ADR 103) — file I/O is a CLI concern
    let registry = connect_registry(&config);
    let manifest_path = absolute_path.join("agent.toml");
    let manifest = match AgentManifest::load(&manifest_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to load agent manifest: {:?}", e);
            return;
        }
    };
    let agent = match registry.register_manifest(manifest) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to deploy agent: {}", e);
            return;
        }
    };
    let agent_id = agent.id.clone();
    let agent_name = agent_routing_key(&agent_id);

    // Connect harness via gRPC — the daemon owns queue and registry
    let mut harness = connect_harness(&config);

    // Set timeline on harness so invocations use the correct branch-scoped subjects (ADR 093)
    if let Some(id) = timeline_id {
        harness.set_timeline(TimelineId::from(id), false);
    }

    harness.start_session(&agent_name);

    // Restore state from trailer
    if let Some(state) = state_trailer {
        println!("Restoring state {}…", &state[..8.min(state.len())]);
        harness.set_initial_state(state);
    }

    println!();

    // Enter REPL with synchronous run_agent (ADR 092)
    super::repl::run(|input| {
        match harness.run_agent(&agent_id, input) {
            Ok(result) => result,
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Promote the current branch to main (ADR 081).
///
/// Labels the old main as `broken-YYYY-MM-DD`, moves main to the current
/// HEAD, and switches to main.
fn promote(dir: &Path) {
    // 1. Get current branch name
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output();

    let current_branch = match output {
        Ok(ref o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => {
            eprintln!("Cannot determine current branch. Are you on a branch?");
            return;
        }
    };

    // 2. Refuse if already on main
    if current_branch == "main" {
        eprintln!("Already on main — nothing to promote.");
        return;
    }

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let broken_name = format!("broken-{}", date);

    // Update timeline rows (ADR 093): seal old main, rename both
    if let Some(store) = open_dag_store(&Config::load()) {
        let _ = store.ensure_main_timeline();

        if let Ok(Some(old_main)) = store.get_timeline_by_branch("main") {
            if let Err(e) = store.seal_timeline(old_main.id) {
                eprintln!("Warning: failed to seal old main timeline: {}", e);
            }
            if let Err(e) = store.rename_timeline(old_main.id, &broken_name) {
                eprintln!("Warning: failed to rename old main timeline: {}", e);
            }
        }

        if let Ok(Some(promoted)) = store.get_timeline_by_branch(&current_branch) {
            if let Err(e) = store.rename_timeline(promoted.id, "main") {
                eprintln!("Warning: failed to rename promoted timeline to main: {}", e);
            }
        }
    }

    // 3. Label old main as broken-YYYY-MM-DD
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["branch", &broken_name, "main"])
        .status();

    match status {
        Ok(s) if !s.success() => {
            eprintln!("Failed to label old main as '{}'.", broken_name);
            return;
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
            return;
        }
        _ => {}
    }

    // 4. Move main to current HEAD
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["branch", "-f", "main", "HEAD"])
        .status();

    match status {
        Ok(s) if !s.success() => {
            eprintln!("Failed to move main to HEAD.");
            return;
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
            return;
        }
        _ => {}
    }

    // 5. Switch to main
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["checkout", "main"])
        .status();

    match status {
        Ok(s) if !s.success() => {
            eprintln!("Failed to switch to main.");
            return;
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
            return;
        }
        _ => {}
    }

    println!("Old main labeled as '{}'.", broken_name);
    println!("Promoted '{}' to main.", current_branch);
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
    use super::*;

    /// Helper: run a git command in a directory, panicking on failure.
    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {:?} failed: {}",
            args, String::from_utf8_lossy(&status.stderr));
    }

    /// Create a test repo with an invoke+complete commit pair.
    ///
    /// Uses raw git commands — no daemon types. Produces the same commit
    /// structure the GitDagWorker writes: subject lines with routing info,
    /// git trailers for Session/Submission/State.
    fn test_repo() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();

        git(dir, &["init", "-b", "main"]);
        git(dir, &["config", "user.email", "test@test.com"]);
        git(dir, &["config", "user.name", "test"]);

        // Invoke commit — needs a file so git has something to commit
        std::fs::write(dir.join("invoke"), "payload").unwrap();
        git(dir, &["add", "invoke"]);
        let invoke_msg = "invoke: cli → agent-a\n\nSession: sess-1\nSubmission: sub-1";
        std::fs::write(dir.join(".commit_msg"), invoke_msg).unwrap();
        git(dir, &["commit", "-F", ".commit_msg"]);

        // Complete commit
        std::fs::write(dir.join("complete"), "payload").unwrap();
        git(dir, &["add", "complete"]);
        let complete_msg = "complete: agent-a → cli\n\nSession: sess-1\nSubmission: sub-1\nState: state-abc123";
        std::fs::write(dir.join(".commit_msg"), complete_msg).unwrap();
        git(dir, &["commit", "-F", ".commit_msg"]);

        tmp
    }

    #[test]
    fn route_shows_session_commits() {
        let tmp = test_repo();

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

    /// Reader side of the trailer contract. Verifies the CLI can extract
    /// Session/Submission/State trailers from git commits.
    ///
    /// Writer side: `git_dag::tests::complete_trailers_readable_by_timeline`.
    #[test]
    fn checkout_reads_trailers() {
        let tmp = test_repo();

        // Get the HEAD commit (complete message)
        let head = read_head_sha(tmp.path()).unwrap();

        // Read trailers from the complete commit
        let session = read_trailer(tmp.path(), &head, "Session");
        let submission = read_trailer(tmp.path(), &head, "Submission");
        let state = read_trailer(tmp.path(), &head, "State");

        assert_eq!(session.as_deref(), Some("sess-1"));
        assert_eq!(submission.as_deref(), Some("sub-1"));
        assert_eq!(state.as_deref(), Some("state-abc123"));
    }

    #[test]
    fn repair_creates_branch() {
        let tmp = test_repo();

        // Get first commit (invoke)
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["rev-list", "--reverse", "main"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let first_commit: String = stdout.trim().lines().next().unwrap().to_string();

        // Detach HEAD at first commit
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["checkout", &first_commit])
            .output()
            .unwrap();
        assert!(status.status.success());

        // Create a repair branch (simulating what repair() does)
        let branch_name = "repair-test";
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["checkout", "-b", branch_name])
            .output()
            .unwrap();
        assert!(status.status.success());

        // Verify branch exists and points to the right commit
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["rev-parse", branch_name])
            .output()
            .unwrap();
        let branch_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(branch_sha, first_commit);
    }

    #[test]
    fn promote_moves_main_and_labels_broken() {
        let tmp = test_repo();

        // Get current main SHA
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["rev-parse", "main"])
            .output()
            .unwrap();
        let original_main = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Get first commit
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["rev-list", "--reverse", "main"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let first_commit: String = stdout.trim().lines().next().unwrap().to_string();

        // Create a branch at the first commit
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["checkout", "-b", "fix-branch", &first_commit])
            .output()
            .unwrap();
        assert!(status.status.success());

        // Simulate promote: label old main, move main, switch
        let broken_name = "broken-test";
        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["branch", broken_name, "main"])
            .output()
            .unwrap();

        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["branch", "-f", "main", "HEAD"])
            .output()
            .unwrap();

        std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["checkout", "main"])
            .output()
            .unwrap();

        // Verify: broken-test points to original main
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["rev-parse", broken_name])
            .output()
            .unwrap();
        let broken_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(broken_sha, original_main);

        // Verify: main now points to first_commit
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["rev-parse", "main"])
            .output()
            .unwrap();
        let new_main = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(new_main, first_commit);
    }

    #[test]
    fn parse_agent_from_complete_subject() {
        assert_eq!(
            parse_agent_from_subject("complete: todoapp → cli"),
            Some("todoapp".to_string()),
        );
    }

    #[test]
    fn parse_agent_ignores_non_complete_subjects() {
        assert_eq!(parse_agent_from_subject("invoke: cli → todoapp"), None);
        assert_eq!(parse_agent_from_subject("request: todoapp → kv.sqlite"), None);
    }

    #[test]
    fn parse_agent_handles_edge_cases() {
        assert_eq!(parse_agent_from_subject("complete: "), None);
        assert_eq!(parse_agent_from_subject(""), None);
        assert_eq!(
            parse_agent_from_subject("complete: my-agent → cli"),
            Some("my-agent".to_string()),
        );
    }
}
