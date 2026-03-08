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
//! │   ├── hash                    # canonical domain hash (hash_dag_node)
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
//! Uses git2 (libgit2) for all git operations — no subprocess spawning, no
//! file lock contention between processes.

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use git2::{FileMode, Oid, Repository, RepositoryInitOptions, Signature, TreeBuilder};

use vlinder_core::domain::workers::dag::build_dag_node;
use vlinder_core::domain::{DagWorker, ObservableMessage, Registry};

/// DAG worker that writes commits to a git repository.
///
/// Stateless per-session: all session state (parent commit, canonical hash)
/// is read from `refs/sessions/<session_id>` on each message. No in-memory
/// maps survive across messages or restarts.
pub struct GitDagWorker {
    repo: Repository,
    registry_host: String,
    /// Registry access for looking up agent/model state at commit time.
    registry: Option<Arc<dyn Registry>>,
}

impl GitDagWorker {
    /// Open (or create) a git repo for DAG commits.
    pub fn open(
        repo_path: &Path,
        registry_host: &str,
        registry: Option<Arc<dyn Registry>>,
    ) -> Result<Self, String> {
        let repo = if repo_path.join(".git").exists() {
            Repository::open(repo_path)
        } else {
            std::fs::create_dir_all(repo_path)
                .map_err(|e| format!("failed to create repo directory: {}", e))?;
            Repository::init_opts(repo_path, RepositoryInitOptions::new().initial_head("main"))
        }
        .map_err(|e| format!("git repo open/init failed: {}", e))?;

        // Create an empty initial commit on main if the repo is fresh.
        // This gives `git checkout main` a clean working tree to return to.
        if repo.head().is_err() {
            let empty_tree = repo
                .treebuilder(None)
                .and_then(|tb| tb.write())
                .and_then(|oid| repo.find_tree(oid))
                .map_err(|e| format!("empty tree failed: {}", e))?;
            let sig = Signature::now("vlinder", "vlinder@localhost")
                .map_err(|e| format!("signature failed: {}", e))?;
            repo.commit(
                Some("refs/heads/main"),
                &sig,
                &sig,
                "Initialize conversations repository",
                &empty_tree,
                &[],
            )
            .map_err(|e| format!("initial commit failed: {}", e))?;
        }

        Ok(Self {
            repo,
            registry_host: registry_host.to_string(),
            registry,
        })
    }

    /// Resolve the session ref to a commit OID. Returns None for a new session.
    fn session_commit(&self, session_id: &str) -> Option<Oid> {
        let refname = format!("refs/sessions/{}", session_id);
        self.repo
            .find_reference(&refname)
            .ok()
            .and_then(|r| r.peel_to_commit().ok())
            .map(|c| c.id())
    }

    /// Read the canonical hash from the last commit's tree for a session.
    /// Returns empty string for a new session (no parent).
    fn session_canonical_hash(&self, session_id: &str) -> String {
        let commit_oid = match self.session_commit(session_id) {
            Some(oid) => oid,
            None => return String::new(),
        };
        let commit = match self.repo.find_commit(commit_oid) {
            Ok(c) => c,
            Err(_) => return String::new(),
        };
        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => return String::new(),
        };

        // Find the most recent message directory (last in sorted order)
        // and read its hash file.
        let mut msg_dirs: Vec<String> = Vec::new();
        for entry in tree.iter() {
            if let Some(name) = entry.name() {
                // Message dirs match the timestamp-sender-type pattern
                if name.contains('-') && entry.kind() == Some(git2::ObjectType::Tree) {
                    msg_dirs.push(name.to_string());
                }
            }
        }
        msg_dirs.sort();

        if let Some(last_dir) = msg_dirs.last() {
            let path = format!("{}/hash", last_dir);
            if let Ok(entry) = tree.get_path(std::path::Path::new(&path)) {
                if let Ok(blob) = self.repo.find_blob(entry.id()) {
                    return String::from_utf8_lossy(blob.content()).to_string();
                }
            }
        }

        String::new()
    }

    /// Build a subtree for a single message — one file per field (ADR 078).
    /// Returns (tree OID, canonical hash) so the caller can update the chain.
    fn build_message_subtree(
        &self,
        msg: &ObservableMessage,
        created_at: DateTime<Utc>,
        canonical_parent: &str,
    ) -> Result<(Oid, String), String> {
        let mut tb = self
            .repo
            .treebuilder(None)
            .map_err(|e| format!("treebuilder failed: {}", e))?;

        let created_at_str = created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        // Common fields present on every message type
        self.insert_field(&mut tb, "session_id", msg.session().as_str())?;
        self.insert_field(&mut tb, "submission_id", msg.submission().as_str())?;
        self.insert_field(&mut tb, "protocol_version", msg.protocol_version())?;
        self.insert_field(&mut tb, "created_at", &created_at_str)?;

        // Payload — raw bytes, every message has one
        let payload_oid = self.write_blob(msg.payload())?;
        tb.insert("payload", payload_oid, FileMode::Blob.into())
            .map_err(|e| format!("insert payload failed: {}", e))?;

        // Type-specific fields + diagnostics
        match msg {
            ObservableMessage::Invoke(m) => {
                self.insert_field(&mut tb, "type", "invoke")?;
                self.insert_field(&mut tb, "harness", m.harness.as_str())?;
                self.insert_field(&mut tb, "runtime", m.runtime.as_str())?;
                self.insert_field(&mut tb, "agent_id", m.agent_id.as_str())?;
                if let Some(ref state) = m.state {
                    self.insert_field(&mut tb, "state", state)?;
                }
                self.insert_diagnostics_toml(&mut tb, &m.diagnostics)?;
            }
            ObservableMessage::Request(m) => {
                self.insert_field(&mut tb, "type", "request")?;
                self.insert_field(&mut tb, "agent_id", m.agent_id.as_str())?;
                self.insert_field(&mut tb, "service", m.service.service_type().as_str())?;
                self.insert_field(&mut tb, "backend", m.service.backend_str())?;
                self.insert_field(&mut tb, "operation", m.operation.as_str())?;
                self.insert_field(&mut tb, "sequence", &m.sequence.as_u32().to_string())?;
                if let Some(ref state) = m.state {
                    self.insert_field(&mut tb, "state", state)?;
                }
                self.insert_diagnostics_toml(&mut tb, &m.diagnostics)?;
            }
            ObservableMessage::Response(m) => {
                self.insert_field(&mut tb, "type", "response")?;
                self.insert_field(&mut tb, "agent_id", m.agent_id.as_str())?;
                self.insert_field(&mut tb, "service", m.service.service_type().as_str())?;
                self.insert_field(&mut tb, "backend", m.service.backend_str())?;
                self.insert_field(&mut tb, "operation", m.operation.as_str())?;
                self.insert_field(&mut tb, "sequence", &m.sequence.as_u32().to_string())?;
                self.insert_field(&mut tb, "correlation_id", m.correlation_id.as_str())?;
                if let Some(ref state) = m.state {
                    self.insert_field(&mut tb, "state", state)?;
                }
                self.insert_diagnostics_toml(&mut tb, &m.diagnostics)?;
            }
            ObservableMessage::Complete(m) => {
                self.insert_field(&mut tb, "type", "complete")?;
                self.insert_field(&mut tb, "agent_id", m.agent_id.as_str())?;
                self.insert_field(&mut tb, "harness", m.harness.as_str())?;
                if let Some(ref state) = m.state {
                    self.insert_field(&mut tb, "state", state)?;
                }
                self.insert_diagnostics_toml(&mut tb, &m.diagnostics)?;
                if !m.diagnostics.stderr.is_empty() {
                    let oid = self.write_blob(&m.diagnostics.stderr)?;
                    tb.insert("stderr", oid, FileMode::Blob.into())
                        .map_err(|e| format!("insert stderr failed: {}", e))?;
                }
            }
            ObservableMessage::Delegate(m) => {
                self.insert_field(&mut tb, "type", "delegate")?;
                self.insert_field(&mut tb, "caller_agent", m.caller.as_str())?;
                self.insert_field(&mut tb, "target_agent", m.target.as_str())?;
                self.insert_field(&mut tb, "nonce", m.nonce.as_str())?;
                if let Some(ref state) = m.state {
                    self.insert_field(&mut tb, "state", state)?;
                }
                self.insert_diagnostics_toml(&mut tb, &m.diagnostics)?;
                if !m.diagnostics.runtime.stderr.is_empty() {
                    let oid = self.write_blob(&m.diagnostics.runtime.stderr)?;
                    tb.insert("stderr", oid, FileMode::Blob.into())
                        .map_err(|e| format!("insert stderr failed: {}", e))?;
                }
            }
        }

        // Compute canonical hash and store it in the subtree
        let dag_node = build_dag_node(msg, canonical_parent);
        self.insert_field(&mut tb, "hash", &dag_node.hash)?;

        let tree_oid = tb
            .write()
            .map_err(|e| format!("write message subtree failed: {}", e))?;
        Ok((tree_oid, dag_node.hash))
    }

    /// Build the accumulated tree: all previous message directories + new one + metadata.
    /// Returns (tree OID, canonical hash) for the new message.
    #[allow(clippy::too_many_arguments)]
    fn build_accumulated_tree(
        &self,
        msg: &ObservableMessage,
        created_at: DateTime<Utc>,
        from: &str,
        _to: &str,
        msg_type: &str,
        parent_commit: Option<Oid>,
        canonical_parent: &str,
    ) -> Result<(Oid, String), String> {
        // Start from the parent commit's tree (if any)
        let parent_tree = parent_commit
            .and_then(|oid| self.repo.find_commit(oid).ok())
            .and_then(|c| c.tree().ok());

        let mut tb = self
            .repo
            .treebuilder(parent_tree.as_ref())
            .map_err(|e| format!("treebuilder failed: {}", e))?;

        // Remove top-level metadata — we re-add fresh copies
        let _ = tb.remove("agent.toml");
        let _ = tb.remove("platform.toml");
        let _ = tb.remove("models");

        // Add new message directory
        let (msg_tree, canonical_hash) =
            self.build_message_subtree(msg, created_at, canonical_parent)?;
        let msg_dir = format!(
            "{}-{}-{}",
            created_at.format("%Y%m%d-%H%M%S%.3f"),
            from,
            msg_type,
        );
        tb.insert(&msg_dir, msg_tree, FileMode::Tree.into())
            .map_err(|e| format!("insert message dir failed: {}", e))?;

        // Add top-level metadata from registry
        if let Some(ref registry) = self.registry {
            let agent_name = message_agent_name(msg);

            if let Some(agent) = registry.get_agent_by_name(&agent_name) {
                if let Ok(agent_toml) = toml::to_string_pretty(&agent) {
                    if let Ok(oid) = self.write_blob(agent_toml.as_bytes()) {
                        let _ = tb.insert("agent.toml", oid, FileMode::Blob.into());
                    }
                }

                if !agent.requirements.models.is_empty() {
                    if let Ok(models_oid) =
                        self.build_models_subtree(registry, &agent.requirements.models)
                    {
                        let _ = tb.insert("models", models_oid, FileMode::Tree.into());
                    }
                }
            }

            let platform_toml = format!(
                "version = \"{}\"\ncommit = \"{}\"\nregistry_host = \"{}\"\n",
                env!("CARGO_PKG_VERSION"),
                env!("VLINDER_GIT_SHA"),
                self.registry_host,
            );
            if let Ok(oid) = self.write_blob(platform_toml.as_bytes()) {
                let _ = tb.insert("platform.toml", oid, FileMode::Blob.into());
            }
        }

        let tree_oid = tb
            .write()
            .map_err(|e| format!("write accumulated tree failed: {}", e))?;
        Ok((tree_oid, canonical_hash))
    }

    /// Build a models/ subtree with one TOML file per model.
    fn build_models_subtree(
        &self,
        registry: &Arc<dyn Registry>,
        models: &std::collections::HashMap<String, String>,
    ) -> Result<Oid, String> {
        let mut tb = self
            .repo
            .treebuilder(None)
            .map_err(|e| format!("treebuilder failed: {}", e))?;

        let mut has_entries = false;
        for (alias, model_name) in models {
            if let Some(model) = registry.get_model(model_name) {
                if let Ok(model_toml) = toml::to_string_pretty(&model) {
                    let oid = self.write_blob(model_toml.as_bytes())?;
                    let filename = format!("{}.toml", alias.replace('/', "-"));
                    tb.insert(&filename, oid, FileMode::Blob.into())
                        .map_err(|e| format!("insert model failed: {}", e))?;
                    has_entries = true;
                }
            }
        }

        if !has_entries {
            return Err("no models to write".to_string());
        }

        tb.write()
            .map_err(|e| format!("write models subtree failed: {}", e))
    }

    /// Write a blob to the git object store.
    fn write_blob(&self, data: &[u8]) -> Result<Oid, String> {
        self.repo
            .blob(data)
            .map_err(|e| format!("blob write failed: {}", e))
    }

    /// Write a scalar string field as a blob and insert into a tree builder.
    fn insert_field(&self, tb: &mut TreeBuilder, name: &str, value: &str) -> Result<(), String> {
        let oid = self.write_blob(value.as_bytes())?;
        tb.insert(name, oid, FileMode::Blob.into())
            .map_err(|e| format!("insert field '{}' failed: {}", name, e))?;
        Ok(())
    }

    /// Serialize diagnostics to TOML and insert as a blob entry.
    fn insert_diagnostics_toml<T: serde::Serialize>(
        &self,
        tb: &mut TreeBuilder,
        diagnostics: &T,
    ) -> Result<(), String> {
        let toml_str = toml::to_string_pretty(diagnostics)
            .map_err(|e| format!("diagnostics TOML serialize failed: {}", e))?;
        let oid = self.write_blob(toml_str.as_bytes())?;
        tb.insert("diagnostics.toml", oid, FileMode::Blob.into())
            .map_err(|e| format!("insert diagnostics.toml failed: {}", e))?;
        Ok(())
    }
}

impl DagWorker for GitDagWorker {
    fn on_observable_message(&mut self, msg: &ObservableMessage, created_at: DateTime<Utc>) {
        let result = (|| -> Result<(), String> {
            let session_id = msg.session().as_str().to_string();
            let (from, to, msg_type) = message_routing(msg);

            // 1. Resolve session state from refs (stateless — no in-memory maps)
            let parent_commit_oid = self.session_commit(&session_id);
            let canonical_parent = self.session_canonical_hash(&session_id);

            // 2. Build accumulated tree (session's previous messages + new one)
            let (tree_oid, _canonical_hash) = self.build_accumulated_tree(
                msg,
                created_at,
                &from,
                &to,
                msg_type,
                parent_commit_oid,
                &canonical_parent,
            )?;
            let tree = self
                .repo
                .find_tree(tree_oid)
                .map_err(|e| format!("find tree failed: {}", e))?;

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

            // 4. Author = message sender (ADR 069), committer = platform
            let author_email = format!("{}@{}", from, self.registry_host);
            let timestamp = git2::Time::new(created_at.timestamp(), 0);
            let author = Signature::new(&from, &author_email, &timestamp)
                .map_err(|e| format!("author signature failed: {}", e))?;
            let committer = Signature::new("vlinder", "vlinder@localhost", &timestamp)
                .map_err(|e| format!("committer signature failed: {}", e))?;

            // 5. Parent: session's last commit, or orphan for new session
            let parent_commit = parent_commit_oid.and_then(|oid| self.repo.find_commit(oid).ok());
            let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

            tracing::debug!(
                msg_type,
                from = from.as_str(),
                to = to.as_str(),
                session = %session_id,
                parent = ?parent_commit_oid,
                "Committing message",
            );

            // 6. Create commit object
            let commit_oid = self
                .repo
                .commit(None, &author, &committer, &message, &tree, &parents)
                .map_err(|e| format!("commit failed: {}", e))?;

            // 7. Update session ref (HEAD stays on main — working tree untouched)
            let refname = format!("refs/sessions/{}", session_id);
            let reflog_msg = format!("{}: {} → {}", msg_type, from, to);
            self.repo
                .reference(&refname, commit_oid, true, &reflog_msg)
                .map_err(|e| format!("session ref update failed: {}", e))?;

            tracing::debug!(commit = %commit_oid, session = %session_id, "Commit succeeded");

            Ok(())
        })();

        if let Err(e) = result {
            tracing::error!(error = %e, "Failed to write git commit");
        }
    }
}

/// Extract (from, to, type_str) from an ObservableMessage for commit metadata.
fn message_routing(msg: &ObservableMessage) -> (String, String, &'static str) {
    match msg {
        ObservableMessage::Invoke(m) => (
            m.harness.as_str().to_string(),
            m.agent_id.to_string(),
            "invoke",
        ),
        ObservableMessage::Request(m) => (
            m.agent_id.to_string(),
            format!("{}.{}", m.service.service_type(), m.service.backend_str()),
            "request",
        ),
        ObservableMessage::Response(m) => (
            format!("{}.{}", m.service.service_type(), m.service.backend_str()),
            m.agent_id.to_string(),
            "response",
        ),
        ObservableMessage::Complete(m) => (
            m.agent_id.to_string(),
            m.harness.as_str().to_string(),
            "complete",
        ),
        ObservableMessage::Delegate(m) => (m.caller.to_string(), m.target.to_string(), "delegate"),
    }
}

/// Extract the agent name for registry lookup.
fn message_agent_name(msg: &ObservableMessage) -> String {
    match msg {
        ObservableMessage::Invoke(m) => m.agent_id.to_string(),
        ObservableMessage::Request(m) => m.agent_id.to_string(),
        ObservableMessage::Response(m) => m.agent_id.to_string(),
        ObservableMessage::Complete(m) => m.agent_id.to_string(),
        ObservableMessage::Delegate(m) => m.target.to_string(),
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
    use std::process::Command;
    use vlinder_core::domain::{
        Agent, AgentId, CompleteMessage, ContainerId, DelegateDiagnostics, DelegateMessage,
        HarnessType, InMemoryRegistry, InMemorySecretStore, InferenceBackendType,
        InvokeDiagnostics, InvokeMessage, Nonce, ObservableMessage, Operation, RequestDiagnostics,
        RequestMessage, ResponseMessage, RuntimeDiagnostics, RuntimeInfo, RuntimeType, SecretStore,
        Sequence, ServiceBackend, ServiceDiagnostics, ServiceMetrics, ServiceType, SessionId,
        SubmissionId, TimelineId,
    };

    fn test_agent_id() -> AgentId {
        AgentId::new("support-agent")
    }

    fn test_invoke(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        let msg = InvokeMessage::new(
            TimelineId::main(),
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
            String::new(),
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Invoke(msg), created_at)
    }

    fn test_request(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        let msg = RequestMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
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
        let request = RequestMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            ServiceBackend::Infer(InferenceBackendType::Ollama),
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
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            HarnessType::Cli,
            payload.to_vec(),
            None,
            RuntimeDiagnostics::placeholder(100),
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Complete(msg), created_at)
    }

    fn test_delegate(payload: &[u8], epoch_secs: i64) -> (ObservableMessage, DateTime<Utc>) {
        let msg = DelegateMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            AgentId::new("coordinator"),
            AgentId::new("summarizer"),
            payload.to_vec(),
            Nonce::new("nonce-1"),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(50),
            },
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

        let agent = Agent::from_toml(
            r#"
            name = "support-agent"
            description = "Support"
            runtime = "container"
            executable = "localhost/support-agent:latest"
            [requirements]

        "#,
        )
        .unwrap();
        registry.register_agent(agent).unwrap();

        let worker = GitDagWorker::open(
            tmp.path(),
            "registry.local:9000",
            Some(Arc::clone(&registry) as Arc<dyn Registry>),
        )
        .unwrap();

        (worker, tmp, registry)
    }

    /// Session ref for the default test session (sess-1).
    const SESS1_REF: &str = "refs/sessions/sess-1";

    /// Run a git command against the test repo. Tests still use the CLI to
    /// verify that git2-written objects are readable by standard git.
    fn git(repo_path: &Path, args: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .map_err(|e| format!("git {} failed: {}", args.join(" "), e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git {} failed: {}", args.join(" "), stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let sha = git(
            tmp.path(),
            &["rev-parse", "--verify", "refs/sessions/sess-1"],
        )
        .unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn commit_message_first_line() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"payload", 1000);

        worker.on_observable_message(&msg, ts);

        let subject = git(tmp.path(), &["log", "-1", "--format=%s", SESS1_REF]).unwrap();
        assert_eq!(subject, "invoke: cli \u{2192} support-agent");
    }

    #[test]
    fn commit_message_trailers() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"payload", 1000);

        worker.on_observable_message(&msg, ts);

        let body = git(tmp.path(), &["log", "-1", "--format=%b", SESS1_REF]).unwrap();
        assert!(body.contains("Session: sess-1"), "body: {}", body);
        assert!(body.contains("Submission: sub-1"), "body: {}", body);
    }

    #[test]
    fn complete_trailers_readable_by_timeline() {
        let (mut worker, tmp) = test_worker();

        let (invoke, ts1) = test_invoke(b"question", 1000);
        worker.on_observable_message(&invoke, ts1);

        let complete = CompleteMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            HarnessType::Cli,
            b"answer".to_vec(),
            Some("state-abc123".to_string()),
            RuntimeDiagnostics::placeholder(100),
        );
        let ts2 = DateTime::from_timestamp(1001, 0).unwrap();
        worker.on_observable_message(&ObservableMessage::Complete(complete), ts2);

        let session = git(
            tmp.path(),
            &[
                "log",
                "-1",
                "--format=%(trailers:key=Session,valueonly)",
                SESS1_REF,
            ],
        )
        .unwrap();
        let submission = git(
            tmp.path(),
            &[
                "log",
                "-1",
                "--format=%(trailers:key=Submission,valueonly)",
                SESS1_REF,
            ],
        )
        .unwrap();
        let state = git(
            tmp.path(),
            &[
                "log",
                "-1",
                "--format=%(trailers:key=State,valueonly)",
                SESS1_REF,
            ],
        )
        .unwrap();

        assert_eq!(session.trim(), "sess-1");
        assert_eq!(submission.trim(), "sub-1");
        assert_eq!(state.trim(), "state-abc123");
    }

    #[test]
    fn author_is_message_sender() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"data", 1000);

        worker.on_observable_message(&msg, ts);

        let author = git(tmp.path(), &["log", "-1", "--format=%an <%ae>", SESS1_REF]).unwrap();
        assert_eq!(author, "cli <cli@registry.local:9000>");
    }

    #[test]
    fn committer_is_platform() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"data", 1000);

        worker.on_observable_message(&msg, ts);

        let committer = git(tmp.path(), &["log", "-1", "--format=%cn <%ce>", SESS1_REF]).unwrap();
        assert_eq!(committer, "vlinder <vlinder@localhost>");
    }

    #[test]
    fn author_date_matches_node() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"data", 1700000000);

        worker.on_observable_message(&msg, ts);

        let date = git(tmp.path(), &["log", "-1", "--format=%at", SESS1_REF]).unwrap();
        assert_eq!(date, "1700000000");
    }

    // --- Per-field storage tests (ADR 078) ---

    #[test]
    fn invoke_directory_has_per_field_files() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"my-payload", 1000);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        let show = |field: &str| {
            git(
                tmp.path(),
                &["show", &format!("{}:{}/{}", SESS1_REF, dir, field)],
            )
        };

        assert_eq!(show("type").unwrap(), "invoke");
        assert_eq!(show("session_id").unwrap(), "sess-1");
        assert_eq!(show("submission_id").unwrap(), "sub-1");
        assert_eq!(show("harness").unwrap(), "cli");
        assert_eq!(show("runtime").unwrap(), "container");
        assert!(show("agent_id").unwrap().contains("support-agent"));
        assert_eq!(show("payload").unwrap(), "my-payload");
        assert_eq!(show("created_at").unwrap(), "1970-01-01T00:16:40.000Z");
        assert!(!show("protocol_version").unwrap().is_empty());
        let diag = show("diagnostics.toml").unwrap();
        assert!(diag.contains("harness_version"), "diag: {}", diag);
        assert!(diag.contains("history_turns"), "diag: {}", diag);
    }

    #[test]
    fn request_directory_has_service_fields() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_request(b"prompt", 1001);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001641.000-support-agent-request";
        let show = |field: &str| {
            git(
                tmp.path(),
                &["show", &format!("{}:{}/{}", SESS1_REF, dir, field)],
            )
        };

        assert_eq!(show("type").unwrap(), "request");
        assert_eq!(show("service").unwrap(), "infer");
        assert_eq!(show("backend").unwrap(), "ollama");
        assert_eq!(show("operation").unwrap(), "run");
        assert_eq!(show("sequence").unwrap(), "1");
        assert!(show("agent_id").unwrap().contains("support-agent"));
    }

    #[test]
    fn response_directory_has_correlation_id() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_response(b"answer", 1002);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001642.000-infer.ollama-response";
        let show = |field: &str| {
            git(
                tmp.path(),
                &["show", &format!("{}:{}/{}", SESS1_REF, dir, field)],
            )
        };

        assert_eq!(show("type").unwrap(), "response");
        assert!(show("correlation_id").is_ok(), "should have correlation_id");
        assert_eq!(show("service").unwrap(), "infer");
        let diag = show("diagnostics.toml").unwrap();
        assert!(diag.contains("duration_ms"), "diag: {}", diag);
    }

    #[test]
    fn complete_directory_has_harness_and_stderr() {
        let (mut worker, tmp) = test_worker();
        let msg_inner = CompleteMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            test_agent_id(),
            HarnessType::Cli,
            b"done".to_vec(),
            None,
            RuntimeDiagnostics {
                stderr: b"WARN: something".to_vec(),
                runtime: RuntimeInfo::Container {
                    engine_version: "5.3.1".to_string(),
                    image_ref: None,
                    image_digest: None,
                    container_id: ContainerId::new("abc123"),
                },
                duration_ms: 2300,
                health: None,
            },
        );
        let msg = ObservableMessage::Complete(msg_inner);
        let ts = DateTime::from_timestamp(1003, 0).unwrap();

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001643.000-support-agent-complete";
        let show = |field: &str| {
            git(
                tmp.path(),
                &["show", &format!("{}:{}/{}", SESS1_REF, dir, field)],
            )
        };

        assert_eq!(show("type").unwrap(), "complete");
        assert_eq!(show("harness").unwrap(), "cli");
        assert_eq!(show("stderr").unwrap(), "WARN: something");
        let diag = show("diagnostics.toml").unwrap();
        assert!(diag.contains("duration_ms"), "diag: {}", diag);
        assert!(
            !diag.contains("stderr"),
            "stderr should be stripped from diagnostics: {}",
            diag
        );
    }

    #[test]
    fn delegate_directory_has_caller_target_reply() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_delegate(b"delegate-payload", 1004);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001644.000-coordinator-delegate";
        let show = |field: &str| {
            git(
                tmp.path(),
                &["show", &format!("{}:{}/{}", SESS1_REF, dir, field)],
            )
        };

        assert_eq!(show("type").unwrap(), "delegate");
        assert_eq!(show("caller_agent").unwrap(), "coordinator");
        assert_eq!(show("target_agent").unwrap(), "summarizer");
        assert_eq!(show("nonce").unwrap(), "nonce-1");
    }

    #[test]
    fn state_file_present_when_state_set() {
        let (mut worker, tmp) = test_worker();
        let invoke = InvokeMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from("sess-1".to_string()),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            b"hello".to_vec(),
            Some("abc123state".to_string()),
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
                history_turns: 0,
            },
            String::new(),
        );
        let msg = ObservableMessage::Invoke(invoke);
        let ts = DateTime::from_timestamp(1000, 0).unwrap();

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        let state = git(
            tmp.path(),
            &["show", &format!("{}:{}/state", SESS1_REF, dir)],
        )
        .unwrap();
        assert_eq!(state, "abc123state");
    }

    #[test]
    fn state_file_absent_when_no_state() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        let result = git(
            tmp.path(),
            &["show", &format!("{}:{}/state", SESS1_REF, dir)],
        );
        assert!(result.is_err(), "should not have state file when None");
    }

    #[test]
    fn stderr_file_absent_when_empty() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_complete(b"done", 1000);

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-support-agent-complete";
        let result = git(
            tmp.path(),
            &["show", &format!("{}:{}/stderr", SESS1_REF, dir)],
        );
        assert!(result.is_err(), "should not have stderr when empty");
    }

    // --- Accumulation and chaining tests ---

    #[test]
    fn messages_accumulate_in_tree() {
        let (mut worker, tmp) = test_worker();

        let (m1, t1) = test_invoke(b"q", 1000);
        worker.on_observable_message(&m1, t1);

        let (m2, t2) = test_request(b"r", 1001);
        worker.on_observable_message(&m2, t2);

        let (m3, t3) = test_response(b"a", 1002);
        worker.on_observable_message(&m3, t3);

        let ls = git(tmp.path(), &["ls-tree", "--name-only", SESS1_REF]).unwrap();
        assert!(ls.contains("19700101-001640.000-cli-invoke"), "ls: {}", ls);
        assert!(
            ls.contains("19700101-001641.000-support-agent-request"),
            "ls: {}",
            ls
        );
        assert!(
            ls.contains("19700101-001642.000-infer.ollama-response"),
            "ls: {}",
            ls
        );
    }

    #[test]
    fn commits_chain_correctly() {
        let (mut worker, tmp) = test_worker();

        let (m1, t1) = test_invoke(b"first", 1000);
        worker.on_observable_message(&m1, t1);
        let commit1 = git(tmp.path(), &["rev-parse", SESS1_REF]).unwrap();

        let (m2, t2) = test_request(b"second", 1001);
        worker.on_observable_message(&m2, t2);
        let commit2 = git(tmp.path(), &["rev-parse", SESS1_REF]).unwrap();

        assert_ne!(commit1, commit2);
        let parent = git(tmp.path(), &["log", "-1", "--format=%P", SESS1_REF]).unwrap();
        assert_eq!(parent, commit1);
    }

    #[test]
    fn first_commit_is_root() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"first", 1000);

        worker.on_observable_message(&msg, ts);

        let parent = git(tmp.path(), &["log", "-1", "--format=%P", SESS1_REF]).unwrap();
        assert_eq!(parent, "");
    }

    #[test]
    fn all_five_message_types_produce_commits() {
        let (mut worker, tmp) = test_worker();

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

        let count = git(tmp.path(), &["rev-list", "--count", SESS1_REF]).unwrap();
        assert_eq!(count, "5");
    }

    // --- Rich tree tests (ADR 070) ---

    #[test]
    fn commit_tree_contains_agent_toml_when_registry_available() {
        let (mut worker, tmp, _registry) = test_worker_with_registry();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let content = git(tmp.path(), &["show", &format!("{}:agent.toml", SESS1_REF)]).unwrap();
        assert!(content.contains("support-agent"), "agent.toml: {}", content);
    }

    #[test]
    fn commit_tree_contains_platform_toml() {
        let (mut worker, tmp, _registry) = test_worker_with_registry();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let content = git(
            tmp.path(),
            &["show", &format!("{}:platform.toml", SESS1_REF)],
        )
        .unwrap();
        assert!(content.contains("version"), "platform.toml: {}", content);
        assert!(
            content.contains("registry_host"),
            "platform.toml: {}",
            content
        );
    }

    #[test]
    fn working_tree_stays_empty() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"visible", 1000);

        worker.on_observable_message(&msg, ts);

        // Working tree should not have message directories (no auto-checkout)
        let dir = "19700101-001640.000-cli-invoke";
        assert!(
            !tmp.path().join(dir).exists(),
            "working tree should be empty — data is in git objects"
        );

        // But data is accessible via git show
        let payload = git(
            tmp.path(),
            &["show", &format!("{}:{}/payload", SESS1_REF, dir)],
        )
        .unwrap();
        assert_eq!(payload, "visible");
    }

    #[test]
    fn main_branch_has_empty_initial_commit() {
        let (_worker, tmp) = test_worker();

        let count = git(tmp.path(), &["rev-list", "--count", "main"]).unwrap();
        assert_eq!(count, "1");

        let ls = git(tmp.path(), &["ls-tree", "--name-only", "main"]).unwrap();
        assert_eq!(ls, "", "main should have an empty tree");
    }

    // --- Sessions as islands tests ---

    fn test_invoke_session(
        payload: &[u8],
        epoch_secs: i64,
        session: &str,
    ) -> (ObservableMessage, DateTime<Utc>) {
        let msg = InvokeMessage::new(
            TimelineId::main(),
            SubmissionId::from("sub-1".to_string()),
            SessionId::from(session.to_string()),
            HarnessType::Cli,
            RuntimeType::Container,
            test_agent_id(),
            payload.to_vec(),
            None,
            InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
                history_turns: 0,
            },
            String::new(),
        );
        let created_at = DateTime::from_timestamp(epoch_secs, 0).unwrap();
        (ObservableMessage::Invoke(msg), created_at)
    }

    #[test]
    fn session_creates_ref() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"hello", 1000);

        worker.on_observable_message(&msg, ts);

        let sha = git(
            tmp.path(),
            &["rev-parse", "--verify", "refs/sessions/sess-1"],
        )
        .unwrap();
        assert_eq!(sha.len(), 40);
    }

    #[test]
    fn different_sessions_are_orphan_islands() {
        let (mut worker, tmp) = test_worker();

        let (m1, t1) = test_invoke_session(b"sess1", 1000, "sess-1");
        worker.on_observable_message(&m1, t1);

        let (m2, t2) = test_invoke_session(b"sess2", 1001, "sess-2");
        worker.on_observable_message(&m2, t2);

        // Each session has its own ref
        let sha1 = git(tmp.path(), &["rev-parse", "refs/sessions/sess-1"]).unwrap();
        let sha2 = git(tmp.path(), &["rev-parse", "refs/sessions/sess-2"]).unwrap();
        assert_ne!(sha1, sha2);

        // Each is a root commit (no parent)
        let parent1 = git(tmp.path(), &["log", "-1", "--format=%P", &sha1]).unwrap();
        let parent2 = git(tmp.path(), &["log", "-1", "--format=%P", &sha2]).unwrap();
        assert_eq!(parent1, "", "sess-1 should be orphan");
        assert_eq!(parent2, "", "sess-2 should be orphan");
    }

    #[test]
    fn messages_chain_within_session_via_ref() {
        let (mut worker, tmp) = test_worker();

        let (m1, t1) = test_invoke(b"first", 1000);
        worker.on_observable_message(&m1, t1);
        let commit1 = git(tmp.path(), &["rev-parse", "refs/sessions/sess-1"]).unwrap();

        let (m2, t2) = test_request(b"second", 1001);
        worker.on_observable_message(&m2, t2);
        let commit2 = git(tmp.path(), &["rev-parse", "refs/sessions/sess-1"]).unwrap();

        assert_ne!(commit1, commit2);
        let parent = git(tmp.path(), &["log", "-1", "--format=%P", &commit2]).unwrap();
        assert_eq!(parent, commit1, "second commit should parent first");
    }

    #[test]
    fn session_tree_only_contains_own_messages() {
        let (mut worker, tmp) = test_worker();

        let (m1, t1) = test_invoke_session(b"sess1-msg", 1000, "sess-1");
        worker.on_observable_message(&m1, t1);

        let (m2, t2) = test_invoke_session(b"sess2-msg", 1001, "sess-2");
        worker.on_observable_message(&m2, t2);

        // sess-1's tree should only have its own message
        let ls1 = git(
            tmp.path(),
            &["ls-tree", "--name-only", "refs/sessions/sess-1"],
        )
        .unwrap();
        assert!(ls1.contains("19700101-001640.000-cli-invoke"));
        assert!(
            !ls1.contains("19700101-001641.000"),
            "sess-1 should not have sess-2's message"
        );

        // sess-2's tree should only have its own message
        let ls2 = git(
            tmp.path(),
            &["ls-tree", "--name-only", "refs/sessions/sess-2"],
        )
        .unwrap();
        assert!(ls2.contains("19700101-001641.000-cli-invoke"));
        assert!(
            !ls2.contains("19700101-001640.000"),
            "sess-2 should not have sess-1's message"
        );
    }

    #[test]
    fn stateless_worker_resumes_session_from_ref() {
        let tmp = tempfile::TempDir::new().unwrap();

        // First worker instance writes one message
        {
            let mut worker = GitDagWorker::open(tmp.path(), "host", None).unwrap();
            let (m1, t1) = test_invoke(b"first", 1000);
            worker.on_observable_message(&m1, t1);
        }

        // Second worker instance — no in-memory state carried over
        {
            let mut worker = GitDagWorker::open(tmp.path(), "host", None).unwrap();
            let (m2, t2) = test_request(b"second", 1001);
            worker.on_observable_message(&m2, t2);
        }

        // Should have 2 commits chained in sess-1
        let count = git(tmp.path(), &["rev-list", "--count", "refs/sessions/sess-1"]).unwrap();
        assert_eq!(count, "2");
    }

    // --- Canonical hash tests ---

    #[test]
    fn message_subtree_contains_canonical_hash() {
        let (mut worker, tmp) = test_worker();
        let (msg, ts) = test_invoke(b"my-payload", 1000);

        // Compute the expected canonical hash (same as RecordingQueue would)
        let expected_node = vlinder_core::domain::workers::dag::build_dag_node(&msg, "");

        worker.on_observable_message(&msg, ts);

        let dir = "19700101-001640.000-cli-invoke";
        let hash = git(
            tmp.path(),
            &["show", &format!("{}:{}/hash", SESS1_REF, dir)],
        )
        .unwrap();
        assert_eq!(
            hash, expected_node.hash,
            "hash file should contain canonical hash"
        );
    }

    #[test]
    fn canonical_hashes_chain_across_messages() {
        let (mut worker, tmp) = test_worker();

        let (m1, t1) = test_invoke(b"first", 1000);
        let expected1 = vlinder_core::domain::workers::dag::build_dag_node(&m1, "");
        worker.on_observable_message(&m1, t1);

        let dir1 = "19700101-001640.000-cli-invoke";
        let hash1 = git(
            tmp.path(),
            &["show", &format!("{}:{}/hash", SESS1_REF, dir1)],
        )
        .unwrap();
        assert_eq!(hash1, expected1.hash);

        let (m2, t2) = test_request(b"second", 1001);
        let expected2 = vlinder_core::domain::workers::dag::build_dag_node(&m2, &hash1);
        worker.on_observable_message(&m2, t2);

        let dir2 = "19700101-001641.000-support-agent-request";
        let hash2 = git(
            tmp.path(),
            &["show", &format!("{}:{}/hash", SESS1_REF, dir2)],
        )
        .unwrap();
        assert_eq!(hash2, expected2.hash, "second hash should chain from first");
        assert_ne!(hash1, hash2);
    }
}
