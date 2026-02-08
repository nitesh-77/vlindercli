//! Session subcommands — inspect conversation history.

use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::config::conversations_dir;

#[derive(Subcommand, Debug, PartialEq)]
pub enum SessionCommand {
    /// Show conversation timeline from git history
    Log {
        /// Filter by session ID
        #[arg(long)]
        session: Option<String>,
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
    },
}

pub fn execute(cmd: SessionCommand) {
    match cmd {
        SessionCommand::Log { session, agent } => log(session, agent),
    }
}

/// A parsed entry from the conversation git log.
struct LogEntry {
    sha: String,
    role: String,       // "user" or "agent"
    body: String,       // First line of message body
    session: String,    // Session trailer
    state: Option<String>, // State trailer (agent commits only)
}

fn log(session_filter: Option<String>, agent_filter: Option<String>) {
    let dir = conversations_dir();

    if !dir.join(".git").exists() {
        println!("No conversations yet.");
        return;
    }

    let entries = match parse_git_log(&dir) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("Failed to read conversation log: {}", e);
            return;
        }
    };

    if entries.is_empty() {
        println!("No conversations yet.");
        return;
    }

    // Apply filters
    let entries: Vec<_> = entries.into_iter()
        .filter(|e| {
            if let Some(ref s) = session_filter {
                if !e.session.contains(s) {
                    return false;
                }
            }
            true
        })
        .filter(|e| {
            if let Some(ref a) = agent_filter {
                // Agent name appears in the session filename (session JSON),
                // but we extract it from the commit body context. For now,
                // filter on session ID prefix which typically includes agent name.
                if !e.session.contains(a) && !e.body.contains(a) {
                    return false;
                }
            }
            true
        })
        .collect();

    if entries.is_empty() {
        println!("No matching conversations.");
        return;
    }

    // Print formatted timeline
    let mut current_session = String::new();
    for entry in &entries {
        if entry.session != current_session {
            if !current_session.is_empty() {
                println!();
            }
            println!("Session: {}", entry.session);
            current_session = entry.session.clone();
        }

        let short_sha = &entry.sha[..7.min(entry.sha.len())];
        let state_indicator = entry.state.as_ref()
            .map(|s| format!("  State: {}…", &s[..8.min(s.len())]))
            .unwrap_or_default();

        // Truncate body for display
        let body = if entry.body.len() > 60 {
            format!("{}…", &entry.body[..60])
        } else {
            entry.body.clone()
        };

        println!("  {} {:>5}  {}{}", short_sha, entry.role, body, state_indicator);
    }
}

/// Parse the git log from the conversations directory into structured entries.
fn parse_git_log(dir: &PathBuf) -> Result<Vec<LogEntry>, String> {
    let output = std::process::Command::new("git")
        .args(["log", "--reverse", "--format=%H%n%s%n%b%n---END---"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git log failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log failed: {}", stderr));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    // Split on our delimiter
    for block in text.split("---END---") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let lines: Vec<&str> = block.lines().collect();
        if lines.len() < 2 {
            continue;
        }

        let sha = lines[0].to_string();
        let role = lines[1].to_string(); // "user" or "agent"

        // Parse body (first non-empty line after subject) and trailers
        let mut body = String::new();
        let mut session = String::new();
        let mut state = None;

        for line in &lines[2..] {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(s) = line.strip_prefix("Session: ") {
                session = s.trim().to_string();
            } else if let Some(s) = line.strip_prefix("State: ") {
                state = Some(s.trim().to_string());
            } else if let Some(_) = line.strip_prefix("Submission: ") {
                // Skip submission trailer
            } else if body.is_empty() {
                body = line.to_string();
            }
        }

        if !sha.is_empty() && !role.is_empty() {
            entries.push(LogEntry {
                sha,
                role,
                body,
                session,
                state,
            });
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_with_state_trailer() {
        // Create a temp git repo with conversation commits
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        // Init repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .unwrap();

        // Create a file and commit as user
        std::fs::write(dir.join("session.json"), "{}").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "user\n\nhello world\n\nSession: ses-abc123"])
            .current_dir(&dir)
            .output()
            .unwrap();

        // Commit as agent with state
        std::fs::write(dir.join("session.json"), "{\"v\":2}").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "agent\n\nHere is the summary\n\nSession: ses-abc123\nSubmission: aaa\nState: deadbeef1234"])
            .current_dir(&dir)
            .output()
            .unwrap();

        let entries = parse_git_log(&dir).unwrap();
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].role, "user");
        assert_eq!(entries[0].body, "hello world");
        assert_eq!(entries[0].session, "ses-abc123");
        assert!(entries[0].state.is_none());

        assert_eq!(entries[1].role, "agent");
        assert_eq!(entries[1].body, "Here is the summary");
        assert_eq!(entries[1].session, "ses-abc123");
        assert_eq!(entries[1].state, Some("deadbeef1234".to_string()));
    }

    #[test]
    fn parse_log_empty_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().to_path_buf();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .unwrap();

        // git log on empty repo will fail — that's OK, we handle it
        let result = parse_git_log(&dir);
        // Either returns an error or empty vec, both acceptable
        match result {
            Ok(entries) => assert!(entries.is_empty()),
            Err(_) => {} // also fine
        }
    }
}
