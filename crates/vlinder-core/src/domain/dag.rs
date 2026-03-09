//! DAG domain types — content-addressed Merkle DAG for runtime tracing (ADR 067).
//!
//! The platform observes every NATS message and writes one DAG node per message.
//! No pairing, no buffering — each message is independently meaningful.
//!
//! Each `DagNode` captures one message:
//! - `hash`: SHA-256(payload || parent_hash || message_type || diagnostics) — Merkle chain
//! - `parent_hash`: previous node in the same session (empty string for root)
//! - `message_type`: Invoke, Request, Response, Complete, or Delegate
//! - `from` / `to`: sender and receiver (parsed from NATS subject)
//! - `session_id`: groups nodes into a linear chain per session
//! - `submission_id`: groups all messages for one user request
//! - `payload`: raw message payload
//! - `created_at`: ISO-8601 timestamp

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// The six message types in the Vlinder protocol (ADR 044, 113).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Invoke,
    Request,
    Response,
    Complete,
    Delegate,
    Repair,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageType::Invoke => "invoke",
            MessageType::Request => "request",
            MessageType::Response => "response",
            MessageType::Complete => "complete",
            MessageType::Delegate => "delegate",
            MessageType::Repair => "repair",
        }
    }
}

impl std::str::FromStr for MessageType {
    type Err = String;

    /// Parse from string. Accepts both canonical names and NATS subject
    /// abbreviations (`req` -> Request, `res` -> Response).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "invoke" => Ok(MessageType::Invoke),
            "req" | "request" => Ok(MessageType::Request),
            "res" | "response" => Ok(MessageType::Response),
            "complete" => Ok(MessageType::Complete),
            "delegate" => Ok(MessageType::Delegate),
            "repair" => Ok(MessageType::Repair),
            _ => Err(format!("unknown message type: {}", s)),
        }
    }
}

/// A single node in the runtime-captured DAG.
///
/// One node per NATS message. No pairing — each message is independently
/// meaningful in the protocol trace.
#[derive(Debug, Clone, PartialEq)]
pub struct DagNode {
    pub hash: String,
    pub parent_hash: String,
    pub message_type: MessageType,
    pub from: String,
    pub to: String,
    pub session_id: String,
    pub submission_id: String,
    pub payload: Vec<u8>,
    /// JSON-serialized diagnostics for the message (ADR 071).
    /// Empty for messages captured before ADR 071.
    pub diagnostics: Vec<u8>,
    /// Raw stderr from container execution (Complete/Delegate only, ADR 071).
    /// Empty for non-container messages or messages captured before ADR 071.
    pub stderr: Vec<u8>,
    pub created_at: DateTime<Utc>,
    /// State hash from the state store (ADR 055).
    /// Present when the message carries state context (e.g. after kv_put).
    pub state: Option<String>,
    /// Protocol version of the sender (semver from Cargo.toml).
    /// Empty for messages captured before protocol versioning was added.
    pub protocol_version: String,
    /// Checkpoint handler name for durable execution (ADR 111).
    /// Present on Request and Response messages when the agent uses checkpoints.
    pub checkpoint: Option<String>,
    /// Operation for Request/Response/Repair messages (ADR 113).
    /// E.g., "run", "get", "put". None for Invoke/Complete/Delegate.
    pub operation: Option<String>,
}

/// Compute the content-addressed hash for a DAG node.
///
/// `sha256(payload || parent_hash || message_type || diagnostics || session_id)`
/// — the parent hash makes this a Merkle chain: changing any ancestor
/// invalidates all descendants. The session_id ensures that identical
/// messages in different sessions produce different hashes (prevents
/// INSERT OR IGNORE from silently dropping cross-session duplicates).
pub fn hash_dag_node(
    payload: &[u8],
    parent_hash: &str,
    message_type: &MessageType,
    diagnostics: &[u8],
    session_id: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    hasher.update(parent_hash.as_bytes());
    hasher.update(message_type.as_str().as_bytes());
    hasher.update(diagnostics);
    hasher.update(session_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Summary of a session for the viewer index page.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub session_id: String,
    pub agent_name: String,
    pub started_at: DateTime<Utc>,
    pub message_count: usize,
    pub is_open: bool,
}

/// A timeline row — represents a named branch of execution.
///
/// Timeline 1 is always "main". Fork creates a new timeline pointing
/// back to its parent. Sealed timelines reject new invocations.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub id: i64,
    pub branch_name: String,
    pub parent_timeline_id: Option<i64>,
    /// Submission hash at the fork point (if forked from a parent).
    pub fork_point: Option<String>,
    pub created_at: DateTime<Utc>,
    /// When this timeline was sealed (broken). None = still active.
    pub broken_at: Option<DateTime<Utc>>,
}

/// A worker that persists observable messages to a DAG structure.
///
/// Git is one implementation (GitDagWorker). Any backend that can
/// preserve the chronological message stream would implement this.
pub trait DagWorker: Send {
    /// Persist a single observable message.
    fn on_observable_message(&mut self, msg: &super::ObservableMessage, created_at: DateTime<Utc>);
}

/// Persistence layer for DAG nodes.
pub trait DagStore: Send + Sync {
    /// Insert a node. Idempotent (content-addressed, INSERT OR IGNORE).
    fn insert_node(&self, node: &DagNode) -> Result<(), String>;

    /// Retrieve a node by its content hash.
    fn get_node(&self, hash: &str) -> Result<Option<DagNode>, String>;

    /// Get all nodes in a session, ordered by `created_at`.
    fn get_session_nodes(&self, session_id: &str) -> Result<Vec<DagNode>, String>;

    /// Get all children of a given parent hash.
    fn get_children(&self, parent_hash: &str) -> Result<Vec<DagNode>, String>;

    /// Get the most recent state hash associated with an agent (ADR 079).
    ///
    /// Scans nodes where the agent is either sender or receiver, returning
    /// the latest non-empty state. Returns None if no state has been recorded.
    fn latest_state(&self, agent_name: &str) -> Result<Option<String>, String>;

    /// Get the hash of the most recently inserted node for a session.
    ///
    /// Used by the transactional outbox to resume Merkle chaining
    /// when a session spans multiple process lifetimes.
    fn latest_node_hash(&self, session_id: &str) -> Result<Option<String>, String>;

    /// Set an override state for timeline checkout (ADR 081).
    ///
    /// When set, `latest_state()` returns this value instead of querying
    /// the dag_nodes table. Cleared automatically when a Complete message
    /// with state is recorded via `insert_node()`.
    fn set_checkout_state(&self, agent_name: &str, state: &str) -> Result<(), String>;

    // -------------------------------------------------------------------------
    // Timeline methods (ADR 093)
    // -------------------------------------------------------------------------

    /// Ensure timeline row 1 ("main") exists. Returns its ID (always 1).
    fn ensure_main_timeline(&self) -> Result<i64, String>;

    /// Create a new timeline. Returns the auto-generated ID.
    fn create_timeline(
        &self,
        branch_name: &str,
        parent_id: Option<i64>,
        fork_point: Option<&str>,
    ) -> Result<i64, String>;

    /// Look up a timeline by its branch name.
    fn get_timeline_by_branch(&self, branch_name: &str) -> Result<Option<Timeline>, String>;

    /// Look up a timeline by its integer ID.
    fn get_timeline(&self, id: i64) -> Result<Option<Timeline>, String>;

    /// Seal a timeline: set `broken_at` to now. Sealed timelines reject invocations.
    fn seal_timeline(&self, id: i64) -> Result<(), String>;

    /// Rename a timeline's branch name.
    fn rename_timeline(&self, id: i64, new_name: &str) -> Result<(), String>;

    /// Check whether a timeline has been sealed.
    fn is_timeline_sealed(&self, id: i64) -> Result<bool, String>;

    /// List all sessions with summary information.
    fn list_sessions(&self) -> Result<Vec<SessionSummary>, String>;
}

// ============================================================================
// In-Memory Implementation (for testing)
// ============================================================================

/// In-memory DagStore for unit tests that don't need SQLite.
pub struct InMemoryDagStore {
    nodes: std::sync::Mutex<Vec<DagNode>>,
    timelines: std::sync::Mutex<Vec<Timeline>>,
    checkout_states: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl InMemoryDagStore {
    pub fn new() -> Self {
        Self {
            nodes: std::sync::Mutex::new(Vec::new()),
            timelines: std::sync::Mutex::new(Vec::new()),
            checkout_states: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for InMemoryDagStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DagStore for InMemoryDagStore {
    fn insert_node(&self, node: &DagNode) -> Result<(), String> {
        let mut nodes = self.nodes.lock().unwrap();
        // Idempotent: skip if hash already exists
        if !nodes.iter().any(|n| n.hash == node.hash) {
            nodes.push(node.clone());
        }
        Ok(())
    }

    fn get_node(&self, hash: &str) -> Result<Option<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        Ok(nodes.iter().find(|n| n.hash == hash).cloned())
    }

    fn get_session_nodes(&self, session_id: &str) -> Result<Vec<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        let mut result: Vec<DagNode> = nodes
            .iter()
            .filter(|n| n.session_id == session_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    fn get_children(&self, parent_hash: &str) -> Result<Vec<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        Ok(nodes
            .iter()
            .filter(|n| n.parent_hash == parent_hash)
            .cloned()
            .collect())
    }

    fn latest_state(&self, agent_name: &str) -> Result<Option<String>, String> {
        // Check checkout override first
        let overrides = self.checkout_states.lock().unwrap();
        if let Some(state) = overrides.get(agent_name) {
            return Ok(Some(state.clone()));
        }
        drop(overrides);

        let nodes = self.nodes.lock().unwrap();
        Ok(nodes
            .iter()
            .rev()
            .filter(|n| n.from == agent_name || n.to == agent_name)
            .find_map(|n| n.state.clone()))
    }

    fn latest_node_hash(&self, session_id: &str) -> Result<Option<String>, String> {
        let nodes = self.nodes.lock().unwrap();
        Ok(nodes
            .iter()
            .rev()
            .find(|n| n.session_id == session_id)
            .map(|n| n.hash.clone()))
    }

    fn set_checkout_state(&self, agent_name: &str, state: &str) -> Result<(), String> {
        self.checkout_states
            .lock()
            .unwrap()
            .insert(agent_name.to_string(), state.to_string());
        Ok(())
    }

    fn ensure_main_timeline(&self) -> Result<i64, String> {
        let mut timelines = self.timelines.lock().unwrap();
        if !timelines.iter().any(|t| t.id == 1) {
            timelines.push(Timeline {
                id: 1,
                branch_name: "main".to_string(),
                parent_timeline_id: None,
                fork_point: None,
                created_at: Utc::now(),
                broken_at: None,
            });
        }
        Ok(1)
    }

    fn create_timeline(
        &self,
        branch_name: &str,
        parent_id: Option<i64>,
        fork_point: Option<&str>,
    ) -> Result<i64, String> {
        let mut timelines = self.timelines.lock().unwrap();
        let id = timelines.len() as i64 + 1;
        timelines.push(Timeline {
            id,
            branch_name: branch_name.to_string(),
            parent_timeline_id: parent_id,
            fork_point: fork_point.map(|s| s.to_string()),
            created_at: Utc::now(),
            broken_at: None,
        });
        Ok(id)
    }

    fn get_timeline_by_branch(&self, branch_name: &str) -> Result<Option<Timeline>, String> {
        let timelines = self.timelines.lock().unwrap();
        Ok(timelines
            .iter()
            .find(|t| t.branch_name == branch_name)
            .cloned())
    }

    fn get_timeline(&self, id: i64) -> Result<Option<Timeline>, String> {
        let timelines = self.timelines.lock().unwrap();
        Ok(timelines.iter().find(|t| t.id == id).cloned())
    }

    fn seal_timeline(&self, id: i64) -> Result<(), String> {
        let mut timelines = self.timelines.lock().unwrap();
        if let Some(t) = timelines.iter_mut().find(|t| t.id == id) {
            t.broken_at = Some(Utc::now());
        }
        Ok(())
    }

    fn rename_timeline(&self, id: i64, new_name: &str) -> Result<(), String> {
        let mut timelines = self.timelines.lock().unwrap();
        if let Some(t) = timelines.iter_mut().find(|t| t.id == id) {
            t.branch_name = new_name.to_string();
        }
        Ok(())
    }

    fn is_timeline_sealed(&self, id: i64) -> Result<bool, String> {
        let timelines = self.timelines.lock().unwrap();
        Ok(timelines
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.broken_at.is_some())
            .unwrap_or(false))
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>, String> {
        let nodes = self.nodes.lock().unwrap();
        let mut sessions: std::collections::HashMap<String, Vec<&DagNode>> =
            std::collections::HashMap::new();
        for node in nodes.iter() {
            sessions
                .entry(node.session_id.clone())
                .or_default()
                .push(node);
        }

        let mut summaries: Vec<SessionSummary> = sessions
            .into_iter()
            .map(|(session_id, mut nodes)| {
                nodes.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                let agent_name = nodes
                    .iter()
                    .find(|n| n.message_type == MessageType::Invoke)
                    .map(|n| n.to.clone())
                    .unwrap_or_default();
                let started_at = nodes.first().map(|n| n.created_at).unwrap_or_default();
                let message_count = nodes
                    .iter()
                    .filter(|n| {
                        n.message_type == MessageType::Invoke
                            || n.message_type == MessageType::Complete
                    })
                    .count();
                let is_open = nodes
                    .last()
                    .map(|n| n.message_type != MessageType::Complete)
                    .unwrap_or(false);
                SessionSummary {
                    session_id,
                    agent_name,
                    started_at,
                    message_count,
                    is_open,
                }
            })
            .collect();

        summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // --- MessageType tests ---

    #[test]
    fn message_type_round_trips() {
        for mt in [
            MessageType::Invoke,
            MessageType::Request,
            MessageType::Response,
            MessageType::Complete,
            MessageType::Delegate,
            MessageType::Repair,
        ] {
            assert_eq!(MessageType::from_str(mt.as_str()), Ok(mt));
        }
    }

    #[test]
    fn message_type_from_unknown_returns_none() {
        assert!(MessageType::from_str("unknown").is_err());
    }

    // --- hash_dag_node tests ---

    #[test]
    fn hash_is_valid_sha256() {
        let hash = hash_dag_node(b"hello", "", &MessageType::Invoke, b"", "sess-1");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_changes_with_parent() {
        let h1 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"", "sess-1");
        let h2 = hash_dag_node(
            b"payload",
            "parent-abc",
            &MessageType::Invoke,
            b"",
            "sess-1",
        );
        assert_ne!(h1, h2, "Merkle property: different parent → different hash");
    }

    #[test]
    fn hash_changes_with_payload() {
        let h1 = hash_dag_node(b"payload-a", "", &MessageType::Invoke, b"", "sess-1");
        let h2 = hash_dag_node(b"payload-b", "", &MessageType::Invoke, b"", "sess-1");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_changes_with_message_type() {
        let h1 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"", "sess-1");
        let h2 = hash_dag_node(b"payload", "", &MessageType::Complete, b"", "sess-1");
        assert_ne!(h1, h2, "same payload, different type → different hash");
    }

    #[test]
    fn hash_changes_with_diagnostics() {
        let h1 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"", "sess-1");
        let h2 = hash_dag_node(
            b"payload",
            "",
            &MessageType::Invoke,
            b"{\"duration_ms\":100}",
            "sess-1",
        );
        assert_ne!(h1, h2, "different diagnostics → different hash");
    }

    #[test]
    fn hash_changes_with_session_id() {
        let h1 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"", "sess-1");
        let h2 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"", "sess-2");
        assert_ne!(
            h1, h2,
            "same payload in different sessions → different hash"
        );
    }

    #[test]
    fn hash_is_deterministic() {
        let h1 = hash_dag_node(b"same", "p", &MessageType::Request, b"diag", "sess-1");
        let h2 = hash_dag_node(b"same", "p", &MessageType::Request, b"diag", "sess-1");
        assert_eq!(h1, h2);
    }
}
