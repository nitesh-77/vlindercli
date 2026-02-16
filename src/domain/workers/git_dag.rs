//! GitDagWorker — writes typed messages as git commits (ADR 064, 069, 070, 078).
//!
//! Each message becomes a commit. The commit tree accumulates — each commit
//! contains all previous message directories plus the new one. The working
//! tree always shows the full conversation state.
//!
//! Each message directory stores one file per field (ADR 078). Scalar fields
//! are plain-text blobs (just the value — the filename is the key). Binary
//! fields (payload, stderr) are raw blobs. Diagnostics are TOML via serde.
//! Git's content-addressing deduplicates identical blobs across messages.
//!
//! ```text
//! tree
//! ├── 20260211-143052.000-cli-invoke/
//! │   ├── type                    # "invoke"
//! │   ├── session_id              # "ses-abc123" — same blob across session
//! │   ├── submission_id           # "sub-def456"
//! │   ├── protocol_version        # "0.1.0"
//! │   ├── created_at              # "2026-02-12T14:30:52.000Z"
//! │   ├── harness                 # "cli"
//! │   ├── runtime                 # "container"
//! │   ├── agent_id                # "http://127.0.0.1:9000/agents/support-agent"
//! │   ├── payload                 # raw bytes
//! │   └── diagnostics.toml        # InvokeDiagnostics via serde
//! ├── 20260211-143053.000-support-agent-request/
//! │   ├── type                    # "request"
//! │   ├── session_id              # same blob as above
//! │   ├── ...per-field files...
//! │   ├── service                 # "infer"
//! │   ├── backend                 # "ollama"
//! │   ├── operation               # "run"
//! │   ├── sequence                # "1"
//! │   ├── payload
//! │   └── diagnostics.toml
//! ├── agent.toml
//! ├── platform.toml
//! └── models/
//! ```
//!
//! Directory names are `{YYYYMMDD-HHMMSS.mmm}-{sender}-{type}`. The timestamp is
//! the observed time (when the platform received the message). Natural `ls`
//! sorting gives chronological order.
//!
//! Commits advance the current branch (HEAD). Sessions are distinguished by
//! `Session:` trailers. Users can fork a branch to diverge from the timeline.
//!
//! Uses git plumbing commands (hash-object, mktree, commit-tree, update-ref)
//! for writes. Runs `git checkout -f` after each commit to keep the working
//! tree populated.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::domain::message::ObservableMessage;
use crate::domain::registry::Registry;

/// DAG worker that writes commits to a git repository.
pub struct GitDagWorker {
    repo_path: PathBuf,
    registry_host: String,
    /// Registry access for looking up agent/model state at commit time.
    registry: Option<Arc<dyn Registry>>,
    /// Last git commit hash — for commit chaining.
    last_commit: Option<String>,
}

impl GitDagWorker {
    /// Open (or create) a git repo for DAG commits.
    pub fn open(
        repo_path: &Path,
        registry_host: &str,
        registry: Option<Arc<dyn Registry>>,
    ) -> Result<Self, String> {
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

        // Read current HEAD for commit chaining (resume after restart)
        let last_commit = Command::new("git")
            .args(["rev-parse", "--verify", "HEAD"])
            .current_dir(repo_path)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            registry_host: registry_host.to_string(),
            registry,
            last_commit,
        })
    }

    /// Process a typed message — the primary entry point (ADR 078).
    pub fn on_observable_message(&mut self, msg: &ObservableMessage, created_at: DateTime<Utc>) {
        let result = (|| -> Result<(), String> {
            let (from, to, msg_type) = message_routing(msg);

            // 1. Build accumulated tree (all previous messages + new one)
            let tree_hash = self.build_accumulated_tree(msg, created_at, &from, &to, msg_type)?;

            // 2. Parent is the previous commit (chronological order)
            let parent = self.last_commit.as_deref();

            // 3. Build commit message with trailers for filtering
            let mut message = format!(
                "{}: {} \u{2192} {}\n\nSession: {}\nSubmission: {}",
                msg_type,
                from,
                to,
                msg.session(),
                msg.submission(),
            );
            if let Some(state) = message_state(msg) {
                message.push_str(&format!("\nState: {}", state));
            }
            let pv = msg.protocol_version();
            if !pv.is_empty() {
                message.push_str(&format!("\nProtocol-Version: {}", pv));
            }

            // 4. Author = message sender (ADR 069)
            let author_email = format!("{}@{}", from, self.registry_host);

            // 5. Create commit
            let commit_hash = self.commit_tree(
                &tree_hash,
                parent,
                &message,
                &from,
                &author_email,
                &created_at,
            )?;

            // 6. Advance current branch (HEAD follows the symbolic ref)
            self.update_ref("HEAD", &commit_hash)?;

            // 7. Sync working tree so files are visible in the directory
            let _ = Command::new("git")
                .args(["checkout", "-f"])
                .current_dir(&self.repo_path)
                .output();

            // 8. Track last commit
            self.last_commit = Some(commit_hash);

            Ok(())
        })();

        if let Err(e) = result {
            tracing::error!(error = %e, "Failed to write git commit");
        }
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

    /// Write a scalar field as a plain-text blob. Returns a mktree entry line.
    fn write_field(&self, name: &str, value: &str) -> Result<String, String> {
        let blob = self.write_blob(value.as_bytes())?;
        Ok(format!("100644 blob {}\t{}", blob, name))
    }

    /// Create a tree from mktree-format entries. Returns the tree hash.
    fn make_tree_from_entries(&self, entries: &str) -> Result<String, String> {
        let mut child = Command::new("git")
            .args(["mktree"])
            .current_dir(&self.repo_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git mktree spawn failed: {}", e))?;

        child.stdin.take().unwrap().write_all(entries.as_bytes())
            .map_err(|e| format!("git mktree write failed: {}", e))?;

        let output = child.wait_with_output()
            .map_err(|e| format!("git mktree wait failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git mktree failed: {}", stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// List top-level entries from a commit's tree. Returns mktree-compatible lines.
    fn ls_tree(&self, commit: &str) -> Result<Vec<String>, String> {
        let output = Command::new("git")
            .args(["ls-tree", commit])
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| format!("git ls-tree failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git ls-tree failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    /// Build a subtree for a single message — one file per field (ADR 078).
    fn build_message_subtree(
        &self,
        msg: &ObservableMessage,
        created_at: DateTime<Utc>,
    ) -> Result<String, String> {
        let mut entries: Vec<String> = Vec::new();
        let created_at_str = created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        // Common fields present on every message type
        entries.push(self.write_field("session_id", msg.session().as_str())?);
        entries.push(self.write_field("submission_id", msg.submission().as_str())?);
        entries.push(self.write_field("protocol_version", msg.protocol_version())?);
        entries.push(self.write_field("created_at", &created_at_str)?);

        // Payload — raw bytes, every message has one
        let payload_blob = self.write_blob(msg.payload())?;
        entries.push(format!("100644 blob {}\tpayload", payload_blob));

        // Type-specific fields + diagnostics
        match msg {
            ObservableMessage::Invoke(m) => {
                entries.push(self.write_field("type", "invoke")?);
                entries.push(self.write_field("harness", m.harness.as_str())?);
                entries.push(self.write_field("runtime", m.runtime.as_str())?);
                entries.push(self.write_field("agent_id", m.agent_id.as_str())?);
                if let Some(ref state) = m.state {
                    entries.push(self.write_field("state", state)?);
                }
                self.write_diagnostics_toml(&m.diagnostics, &mut entries)?;
            }
            ObservableMessage::Request(m) => {
                entries.push(self.write_field("type", "request")?);
                entries.push(self.write_field("agent_id", m.agent_id.as_str())?);
                entries.push(self.write_field("service", m.service.as_str())?);
                entries.push(self.write_field("backend", &m.backend)?);
                entries.push(self.write_field("operation", m.operation.as_str())?);
                entries.push(self.write_field("sequence", &m.sequence.as_u32().to_string())?);
                if let Some(ref state) = m.state {
                    entries.push(self.write_field("state", state)?);
                }
                self.write_diagnostics_toml(&m.diagnostics, &mut entries)?;
            }
            ObservableMessage::Response(m) => {
                entries.push(self.write_field("type", "response")?);
                entries.push(self.write_field("agent_id", m.agent_id.as_str())?);
                entries.push(self.write_field("service", m.service.as_str())?);
                entries.push(self.write_field("backend", &m.backend)?);
                entries.push(self.write_field("operation", m.operation.as_str())?);
                entries.push(self.write_field("sequence", &m.sequence.as_u32().to_string())?);
                entries.push(self.write_field("correlation_id", m.correlation_id.as_str())?);
                if let Some(ref state) = m.state {
                    entries.push(self.write_field("state", state)?);
                }
                self.write_diagnostics_toml(&m.diagnostics, &mut entries)?;
            }
            ObservableMessage::Complete(m) => {
                entries.push(self.write_field("type", "complete")?);
                entries.push(self.write_field("agent_id", m.agent_id.as_str())?);
                entries.push(self.write_field("harness", m.harness.as_str())?);
                if let Some(ref state) = m.state {
                    entries.push(self.write_field("state", state)?);
                }
                // diagnostics.toml (stderr is #[serde(skip)])
                self.write_diagnostics_toml(&m.diagnostics, &mut entries)?;
                // stderr as separate binary blob
                if !m.diagnostics.stderr.is_empty() {
                    let blob = self.write_blob(&m.diagnostics.stderr)?;
                    entries.push(format!("100644 blob {}\tstderr", blob));
                }
            }
            ObservableMessage::Delegate(m) => {
                entries.push(self.write_field("type", "delegate")?);
                entries.push(self.write_field("caller_agent", &m.caller_agent)?);
                entries.push(self.write_field("target_agent", &m.target_agent)?);
                entries.push(self.write_field("reply_subject", &m.reply_subject)?);
                if let Some(ref state) = m.state {
                    entries.push(self.write_field("state", state)?);
                }
                self.write_diagnostics_toml(&m.diagnostics, &mut entries)?;
                if !m.diagnostics.container.stderr.is_empty() {
                    let blob = self.write_blob(&m.diagnostics.container.stderr)?;
                    entries.push(format!("100644 blob {}\tstderr", blob));
                }
            }
        }

        let entries_str = entries.join("\n") + "\n";
        self.make_tree_from_entries(&entries_str)
    }

    /// Serialize diagnostics to TOML and add as a blob entry.
    fn write_diagnostics_toml<T: serde::Serialize>(
        &self,
        diagnostics: &T,
        entries: &mut Vec<String>,
    ) -> Result<(), String> {
        let toml_str = toml::to_string_pretty(diagnostics)
            .map_err(|e| format!("diagnostics TOML serialize failed: {}", e))?;
        let blob = self.write_blob(toml_str.as_bytes())?;
        entries.push(format!("100644 blob {}\tdiagnostics.toml", blob));
        Ok(())
    }

    /// Build the accumulated tree: all previous message directories + new one + metadata.
    fn build_accumulated_tree(
        &self,
        msg: &ObservableMessage,
        created_at: DateTime<Utc>,
        from: &str,
        _to: &str,
        msg_type: &str,
    ) -> Result<String, String> {
        // Start with parent tree entries (if any)
        let mut entries: Vec<String> = if let Some(ref parent) = self.last_commit {
            let mut existing = self.ls_tree(parent)?;
            // Remove top-level metadata entries — we'll re-add them fresh
            existing.retain(|e| {
                !e.ends_with("\tagent.toml") &&
                !e.ends_with("\tplatform.toml") &&
                !e.ends_with("\tmodels")
            });
            existing
        } else {
            Vec::new()
        };

        // Add new message directory: {YYYYMMDD-HHMMSS.mmm}-{sender}-{type}
        let msg_tree = self.build_message_subtree(msg, created_at)?;
        let msg_dir = format!(
            "{}-{}-{}",
            created_at.format("%Y%m%d-%H%M%S%.3f"),
            from,
            msg_type,
        );
        entries.push(format!("040000 tree {}\t{}", msg_tree, msg_dir));

        // Add top-level metadata from registry
        if let Some(ref registry) = self.registry {
            let agent_name = message_agent_name(msg);

            if let Some(agent) = registry.get_agent_by_name(&agent_name) {
                if let Ok(agent_toml) = toml::to_string_pretty(&agent) {
                    if let Ok(blob) = self.write_blob(agent_toml.as_bytes()) {
                        entries.push(format!("100644 blob {}\tagent.toml", blob));
                    }
                }

                if !agent.requirements.models.is_empty() {
                    if let Ok(models_tree) = self.build_models_subtree(registry, &agent.requirements.models) {
                        entries.push(format!("040000 tree {}\tmodels", models_tree));
                    }
                }
            }

            let platform_toml = format!(
                "version = \"{}\"\ncommit = \"{}\"\nregistry_host = \"{}\"\n",
                env!("CARGO_PKG_VERSION"),
                env!("VLINDER_GIT_SHA"),
                self.registry_host,
            );
            if let Ok(blob) = self.write_blob(platform_toml.as_bytes()) {
                entries.push(format!("100644 blob {}\tplatform.toml", blob));
            }
        }

        // Join entries with newlines and create tree
        let entries_str = entries.join("\n") + "\n";
        self.make_tree_from_entries(&entries_str)
    }

    /// Build a models/ subtree with one TOML file per model.
    fn build_models_subtree(
        &self,
        registry: &Arc<dyn Registry>,
        models: &std::collections::HashMap<String, crate::domain::ResourceId>,
    ) -> Result<String, String> {
        let mut entries = String::new();

        for alias in models.keys() {
            if let Some(model) = registry.get_model(alias) {
                if let Ok(model_toml) = toml::to_string_pretty(&model) {
                    let blob = self.write_blob(model_toml.as_bytes())?;
                    let filename = format!("{}.toml", alias.replace('/', "-"));
                    entries.push_str(&format!("100644 blob {}\t{}\n", blob, filename));
                }
            }
        }

        if entries.is_empty() {
            return Err("no models to write".to_string());
        }

        self.make_tree_from_entries(&entries)
    }

    /// Create a commit from a tree. Returns the commit hash.
    fn commit_tree(
        &self,
        tree_hash: &str,
        parent: Option<&str>,
        message: &str,
        author_name: &str,
        author_email: &str,
        author_date: &DateTime<Utc>,
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

        let date_value = format!("@{} +0000", author_date.timestamp());

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

/// Extract (from, to, type_str) from an ObservableMessage for commit metadata.
fn message_routing(msg: &ObservableMessage) -> (String, String, &'static str) {
    match msg {
        ObservableMessage::Invoke(m) => (
            m.harness.as_str().to_string(),
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
            "invoke",
        ),
        ObservableMessage::Request(m) => (
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
            format!("{}.{}", m.service, m.backend),
            "request",
        ),
        ObservableMessage::Response(m) => (
            format!("{}.{}", m.service, m.backend),
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
            "response",
        ),
        ObservableMessage::Complete(m) => (
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
            m.harness.as_str().to_string(),
            "complete",
        ),
        ObservableMessage::Delegate(m) => (
            m.caller_agent.clone(),
            m.target_agent.clone(),
            "delegate",
        ),
    }
}

/// Extract the agent name for registry lookup.
fn message_agent_name(msg: &ObservableMessage) -> String {
    match msg {
        ObservableMessage::Invoke(m) =>
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
        ObservableMessage::Request(m) =>
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
        ObservableMessage::Response(m) =>
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
        ObservableMessage::Complete(m) =>
            m.agent_id.as_str().rsplit('/').next().unwrap_or(m.agent_id.as_str()).to_string(),
        ObservableMessage::Delegate(m) => m.target_agent.clone(),
    }
}

/// Extract state from the message if present.
fn message_state(msg: &ObservableMessage) -> Option<&str> {
    match msg {
        ObservableMessage::Invoke(m) => m.state.as_deref(),
        ObservableMessage::Request(m) => m.state.as_deref(),
        ObservableMessage::Response(m) => m.state.as_deref(),
        ObservableMessage::Complete(m) => m.state.as_deref(),
        ObservableMessage::Delegate(m) => m.state.as_deref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::*;
    use crate::domain::diagnostics::*;
    use crate::domain::{ContainerId, Operation, RuntimeType, ResourceId, ServiceType, Agent, SecretStore};
    use crate::secret_store::InMemorySecretStore;
    use crate::registry::InMemoryRegistry;

    fn test_agent_id() -> ResourceId {
        ResourceId::new("http://127.0.0.1:9000/agents/support-agent")
    }

    fn test_invoke(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        let msg = InvokeMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            payload.to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
                history_turns: 3,
            },
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Invoke(msg), created_at)
    }

    fn test_request(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        let msg = RequestMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            ServiceType::Infer,
            "ollama",
            Operation::Run,
            Sequence::from(1),
            payload.to_vec(),
            None,
            RequestDiagnostics {
                sequence: 1,
                endpoint: "/infer".to_string(),
                request_bytes: 1024,
                received_at_ms: 1700000000000,
            },
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Request(msg), created_at)
    }

    fn test_response(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        // Build a request first to derive the response
        let request = RequestMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            ServiceType::Infer,
            "ollama",
            Operation::Run,
            Sequence::from(1),
            b"prompt".to_vec(),
            None,
            RequestDiagnostics {
                sequence: 1,
                endpoint: "/infer".to_string(),
                request_bytes: 0,
                received_at_ms: 0,
            },
        );
        let msg = ResponseMessage::from_request_with_diagnostics(
            &request,
            payload.to_vec(),
            ServiceDiagnostics {
                service: ServiceType::Infer,
                backend: "ollama".to_string(),
                duration_ms: 1800,
                metrics: ServiceMetrics::Inference {
                    tokens_input: 512,
                    tokens_output: 908,
                    model: "phi3:latest".to_string(),
                },
            },
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Response(msg), created_at)
    }

    fn test_complete(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        let msg = CompleteMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            HarnessType::Cli,
            payload.to_vec(),
            None,
            ContainerDiagnostics::placeholder(100),
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Complete(msg), created_at)
    }

    fn test_delegate(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        let msg = DelegateMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            "coordinator",
            "summarizer",
            payload.to_vec(),
            "vlinder.sub-1.delegate-reply",
            None,
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(50) },
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Delegate(msg), created_at)
    }

    fn test_worker() -> (GitDagWorker, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let worker = GitDagWorker::open(tmp.path(), "registry.local:9000", None).unwrap();
        (worker, tmp)
    }

    fn test_secret_store() -> Arc<dyn SecretStore> {
        Arc::new(InMemorySecretStore::new())
    }

    fn test_worker_with_registry() -> (GitDagWorker, tempfile::TempDir, Arc<InMemoryRegistry>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = Arc::new(InMemoryRegistry::new(test_secret_store()));
        registry.register_runtime(RuntimeType::Container);

        let agent = Agent::from_toml(r#"
            name = "support-agent"
            description = "Support"
            runtime = "container"
            executable = "localhost/support-agent:latest"
            [requirements]

        "#).unwrap();
        registry.register_agent(agent).unwrap();

        let worker = GitDagWorker::open(
            tmp.path(),
            "registry.local:9000",
            Some(Arc::clone(&registry) as Arc<dyn Registry>),
        ).unwrap();

        (worker, tmp, registry)
    }

    // --- Basic commit tests ---

    #[test]
    fn open_creates_git_repo() {
        let (_worker, tmp) = test_worker();
        assert!(tmp.path().join(".git").exists());
    }

    #[test]
    fn open_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        GitDagWorker::open(tmp.path(), "host", None).unwrap();
        GitDagWorker::open(tmp.path(), "host", None).unwrap();
        assert!(tmp.path().join(".git").exists());
    }

    #[test]
    fn commit_advances_main() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let sha = worker.git(&["rev-parse", "--verify", "main"]).unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn commit_message_first_line() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"payload", 1000);

        worker.on_observable_message(&msg, ts);

        let subject = worker.git(&["log", "-1", "--format=%s", "main"]).unwrap();
        assert_eq!(subject, "invoke: cli \u{2192} support-agent");
    }

    #[test]
    fn commit_message_trailers() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"payload", 1000);

        worker.on_observable_message(&msg, ts);

        let body = worker.git(&["log", "-1", "--format=%b", "main"]).unwrap();
        assert!(body.contains("Session: sess-1"), "body: {}", body);
        assert!(body.contains("Submission: sub-1"), "body: {}", body);
    }

    #[test]
    fn author_is_message_sender() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"data", 1000);

        worker.on_observable_message(&msg, ts);

        let author = worker.git(&["log", "-1", "--format=%an <%ae>", "main"]).unwrap();
        assert_eq!(author, "cli <cli@registry.local:9000>");
    }

    #[test]
    fn committer_is_platform() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"data", 1000);

        worker.on_observable_message(&msg, ts);

        let committer = worker.git(&["log", "-1", "--format=%cn <%ce>", "main"]).unwrap();
        assert_eq!(committer, "vlinder <vlinder@localhost>");
    }

    #[test]
    fn author_date_matches_node() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"data", 1700000000);

        worker.on_observable_message(&msg, ts);

        let date = worker.git(&["log", "-1", "--format=%at", "main"]).unwrap();
        assert_eq!(date, "1700000000");
    }

    // --- Per-field storage tests (ADR 078) ---

    #[test]
    fn invoke_directory_has_per_field_files() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"my-payload", 1000);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        let show = |field: &str| worker.git(&["show", &format!("main:{}/{}", dir, field)]);

        assert_eq!(show("type").unwrap(), "invoke");
        assert_eq!(show("session_id").unwrap(), "sess-1");
        assert_eq!(show("submission_id").unwrap(), "sub-1");
        assert_eq!(show("harness").unwrap(), "cli");
        assert_eq!(show("runtime").unwrap(), "container");
        assert!(show("agent_id").unwrap().contains("support-agent"));
        assert_eq!(show("payload").unwrap(), "my-payload");
        assert_eq!(show("created_at").unwrap(), "1970-01-01T00:16:40.000Z");
        // protocol_version is set by the constructor
        assert!(!show("protocol_version").unwrap().is_empty());
        // diagnostics.toml present with typed fields
        let diag = show("diagnostics.toml").unwrap();
        assert!(diag.contains("harness_version"), "diag: {}", diag);
        assert!(diag.contains("history_turns"), "diag: {}", diag);
    }

    #[test]
    fn request_directory_has_service_fields() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_request(b"prompt", 1001);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001641.000-support-agent-request";
        let show = |field: &str| worker.git(&["show", &format!("main:{}/{}", dir, field)]);

        assert_eq!(show("type").unwrap(), "request");
        assert_eq!(show("service").unwrap(), "infer");
        assert_eq!(show("backend").unwrap(), "ollama");
        assert_eq!(show("operation").unwrap(), "run");
        assert_eq!(show("sequence").unwrap(), "1");
        assert!(show("agent_id").unwrap().contains("support-agent"));
    }

    #[test]
    fn response_directory_has_correlation_id() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_response(b"answer", 1002);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001642.000-infer.ollama-response";
        let show = |field: &str| worker.git(&["show", &format!("main:{}/{}", dir, field)]);

        assert_eq!(show("type").unwrap(), "response");
        assert!(show("correlation_id").is_ok(), "should have correlation_id");
        assert_eq!(show("service").unwrap(), "infer");
        // ServiceDiagnostics contains token counts
        let diag = show("diagnostics.toml").unwrap();
        assert!(diag.contains("duration_ms"), "diag: {}", diag);
    }

    #[test]
    fn complete_directory_has_harness_and_stderr() {
        let (mut worker, _tmp) = test_worker();
        let msg_inner = CompleteMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            HarnessType::Cli,
            b"done".to_vec(),
            None,
            ContainerDiagnostics {
                stderr: b"WARN: something".to_vec(),
                runtime: ContainerRuntimeInfo {
                    engine_version: "5.3.1".to_string(),
                    image_ref: None,
                    image_digest: None,
                    container_id: ContainerId::new("abc123"),
                },
                duration_ms: 2300,
            },
        );
        let msg = ObservableMessage::Complete(msg_inner);
        let ts = DateTime::from_timestamp(1003, 0).unwrap();

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001643.000-support-agent-complete";
        let show = |field: &str| worker.git(&["show", &format!("main:{}/{}", dir, field)]);

        assert_eq!(show("type").unwrap(), "complete");
        assert_eq!(show("harness").unwrap(), "cli");
        assert_eq!(show("stderr").unwrap(), "WARN: something");
        // diagnostics.toml should NOT contain stderr (it's #[serde(skip)])
        let diag = show("diagnostics.toml").unwrap();
        assert!(diag.contains("duration_ms"), "diag: {}", diag);
        assert!(!diag.contains("stderr"), "stderr should be stripped from diagnostics: {}", diag);
    }

    #[test]
    fn delegate_directory_has_caller_target_reply() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_delegate(b"delegate-payload", 1004);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001644.000-coordinator-delegate";
        let show = |field: &str| worker.git(&["show", &format!("main:{}/{}", dir, field)]);

        assert_eq!(show("type").unwrap(), "delegate");
        assert_eq!(show("caller_agent").unwrap(), "coordinator");
        assert_eq!(show("target_agent").unwrap(), "summarizer");
        assert_eq!(show("reply_subject").unwrap(), "vlinder.sub-1.delegate-reply");
    }

    #[test]
    fn state_file_present_when_state_set() {
        let (mut worker, _tmp) = test_worker();
        let invoke = InvokeMessage::new(
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"hello".to_vec(),
            Some("abc123state".to_string()),
            InvokeDiagnostics { harness_version: "0.1.0".to_string(), history_turns: 0 },
        );
        let msg = ObservableMessage::Invoke(invoke);
        let ts = DateTime::from_timestamp(1000, 0).unwrap();

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        let state = worker.git(&["show", &format!("main:{}/state", dir)]).unwrap();
        assert_eq!(state, "abc123state");
    }

    #[test]
    fn state_file_absent_when_no_state() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        let result = worker.git(&["show", &format!("main:{}/state", dir)]);
        assert!(result.is_err(), "should not have state file when None");
    }

    #[test]
    fn stderr_file_absent_when_empty() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_complete(b"done", 1000);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-support-agent-complete";
        let result = worker.git(&["show", &format!("main:{}/stderr", dir)]);
        assert!(result.is_err(), "should not have stderr when empty");
    }

    // --- Accumulation and chaining tests ---

    #[test]
    fn messages_accumulate_in_tree() {
        let (mut worker, _tmp) = test_worker();

        let (m1, t1) = test_invoke(b"q", 1000);
        worker.on_observable_message(&m1, t1);

        let (m2, t2) = test_request(b"r", 1001);
        worker.on_observable_message(&m2, t2);

        let (m3, t3) = test_response(b"a", 1002);
        worker.on_observable_message(&m3, t3);

        let ls = worker.git(&["ls-tree", "--name-only", "main"]).unwrap();
        assert!(ls.contains("19700101-001640.000-cli-invoke"), "ls: {}", ls);
        assert!(ls.contains("19700101-001641.000-support-agent-request"), "ls: {}", ls);
        assert!(ls.contains("19700101-001642.000-infer.ollama-response"), "ls: {}", ls);
    }

    #[test]
    fn commits_chain_correctly() {
        let (mut worker, _tmp) = test_worker();

        let (m1, t1) = test_invoke(b"first", 1000);
        worker.on_observable_message(&m1, t1);
        let commit1 = worker.git(&["rev-parse", "main"]).unwrap();

        let (m2, t2) = test_request(b"second", 1001);
        worker.on_observable_message(&m2, t2);
        let commit2 = worker.git(&["rev-parse", "main"]).unwrap();

        assert_ne!(commit1, commit2);
        let parent = worker.git(&["log", "-1", "--format=%P", "main"]).unwrap();
        assert_eq!(parent, commit1);
    }

    #[test]
    fn first_commit_is_root() {
        let (mut worker, _tmp) = test_worker();
        let (msg, ts) = test_invoke(b"first", 1000);

        worker.on_observable_message(&msg, ts);

        let parent = worker.git(&["log", "-1", "--format=%P", "main"]).unwrap();
        assert_eq!(parent, "");
    }

    #[test]
    fn all_five_message_types_produce_commits() {
        let (mut worker, _tmp) = test_worker();

        let (m1, t1) = test_invoke(b"1", 1000);
        worker.on_observable_message(&m1, t1);
        let (m2, t2) = test_request(b"2", 1001);
        worker.on_observable_message(&m2, t2);
        let (m3, t3) = test_response(b"3", 1002);
        worker.on_observable_message(&m3, t3);
        let (m4, t4) = test_delegate(b"4", 1003);
        worker.on_observable_message(&m4, t4);
        let (m5, t5) = test_complete(b"5", 1004);
        worker.on_observable_message(&m5, t5);

        let count = worker.git(&["rev-list", "--count", "main"]).unwrap();
        assert_eq!(count, "5");
    }

    // --- Rich tree tests (ADR 070) ---

    #[test]
    fn commit_tree_contains_agent_toml_when_registry_available() {
        let (mut worker, _tmp, _registry) = test_worker_with_registry();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let content = worker.git(&["show", "main:agent.toml"]).unwrap();
        assert!(content.contains("support-agent"), "agent.toml: {}", content);
    }

    #[test]
    fn commit_tree_contains_platform_toml() {
        let (mut worker, _tmp, _registry) = test_worker_with_registry();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let content = worker.git(&["show", "main:platform.toml"]).unwrap();
        assert!(content.contains("version"), "platform.toml: {}", content);
        assert!(content.contains("registry_host"), "platform.toml: {}", content);
    }

    #[test]
    fn working_tree_is_populated() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"visible", 1000);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        assert!(tmp.path().join(dir).join("payload").exists());
        let content = std::fs::read(tmp.path().join(dir).join("payload")).unwrap();
        assert_eq!(content, b"visible");
    }

    #[test]
    fn open_resumes_last_commit() {
        let tmp = tempfile::TempDir::new().unwrap();

        {
            let mut worker = GitDagWorker::open(tmp.path(), "host", None).unwrap();
            let (m1, t1) = test_invoke(b"1", 1000);
            worker.on_observable_message(&m1, t1);
            let (m2, t2) = test_request(b"2", 1001);
            worker.on_observable_message(&m2, t2);
        }

        let mut worker = GitDagWorker::open(tmp.path(), "host", None).unwrap();
        assert!(worker.last_commit.is_some());

        let (m3, t3) = test_complete(b"3", 2000);
        worker.on_observable_message(&m3, t3);

        let count = worker.git(&["rev-list", "--count", "main"]).unwrap();
        assert_eq!(count, "3");
    }
}
