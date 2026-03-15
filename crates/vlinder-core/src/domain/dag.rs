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

use super::session::Session;

/// The message types in the Vlinder protocol (ADR 044, 113).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Invoke,
    Request,
    Response,
    Complete,
    Delegate,
    Repair,
    Fork,
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
            MessageType::Fork => "fork",
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
            "fork" => Ok(MessageType::Fork),
            _ => Err(format!("unknown message type: {}", s)),
        }
    }
}

/// A single node in the runtime-captured DAG.
///
/// Wraps the full `ObservableMessage` with DAG metadata:
/// content-addressed ID, Merkle parent pointer, and insertion timestamp.
/// Everything else lives on the message itself.
#[derive(Debug, Clone, PartialEq)]
pub struct DagNode {
    pub id: super::DagNodeId,
    pub parent_id: super::DagNodeId,
    pub created_at: DateTime<Utc>,
    pub message: super::ObservableMessage,
}

impl DagNode {
    pub fn message_type(&self) -> MessageType {
        self.message.message_type()
    }
    pub fn session_id(&self) -> &super::SessionId {
        self.message.session()
    }
    pub fn submission_id(&self) -> &super::SubmissionId {
        self.message.submission()
    }
    pub fn payload(&self) -> &[u8] {
        self.message.payload()
    }
    pub fn protocol_version(&self) -> &str {
        self.message.protocol_version()
    }
    pub fn timeline_id(&self) -> &super::TimelineId {
        self.message.timeline()
    }
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
    parent_id: &super::DagNodeId,
    message_type: &MessageType,
    diagnostics: &[u8],
    session_id: &super::SessionId,
) -> super::DagNodeId {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    hasher.update(parent_id.as_str().as_bytes());
    hasher.update(message_type.as_str().as_bytes());
    hasher.update(diagnostics);
    hasher.update(session_id.as_str().as_bytes());
    super::DagNodeId::from(format!("{:x}", hasher.finalize()))
}

/// Summary of a session for the viewer index page.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub session_id: super::SessionId,
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
    pub session_id: super::SessionId,
    pub parent_timeline_id: Option<i64>,
    /// DagNode at the fork point (if forked from a parent).
    pub fork_point: Option<super::DagNodeId>,
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

    /// Retrieve a node by its content-addressed ID.
    fn get_node(&self, id: &super::DagNodeId) -> Result<Option<DagNode>, String>;

    /// Retrieve a node by ID prefix. Returns an error if ambiguous.
    fn get_node_by_prefix(&self, prefix: &str) -> Result<Option<DagNode>, String> {
        // Default: try exact match first, then scan is left to implementors.
        self.get_node(&super::DagNodeId::from(prefix.to_string()))
    }

    /// Get all nodes in a session, ordered by `created_at`.
    fn get_session_nodes(&self, session_id: &super::SessionId) -> Result<Vec<DagNode>, String>;

    /// Get all children of a given parent ID.
    fn get_children(&self, parent_id: &super::DagNodeId) -> Result<Vec<DagNode>, String>;

    /// Get the most recent state hash associated with an agent (ADR 079).
    ///
    /// Scans nodes where the agent is either sender or receiver, returning
    /// the latest non-empty state. Returns None if no state has been recorded.
    fn latest_state(&self, agent_name: &str) -> Result<Option<String>, String>;

    /// Get the hash of the most recently inserted node for a session.
    ///
    /// Used by the transactional outbox to resume Merkle chaining
    /// when a session spans multiple process lifetimes.
    fn latest_node_hash(
        &self,
        session_id: &super::SessionId,
    ) -> Result<Option<super::DagNodeId>, String>;

    /// Set an override state for timeline checkout (ADR 081).
    ///
    /// When set, `latest_state()` returns this value instead of querying
    /// the dag_nodes table. Cleared automatically when a Complete message
    /// with state is recorded via `insert_node()`.
    fn set_checkout_state(&self, agent_name: &str, state: &str) -> Result<(), String>;

    // -------------------------------------------------------------------------
    // Timeline methods (ADR 093)
    // -------------------------------------------------------------------------

    /// Create a new timeline. Returns the auto-generated ID.
    fn create_timeline(
        &self,
        branch_name: &str,
        session_id: &super::SessionId,
        parent_id: Option<i64>,
        fork_point: Option<&super::DagNodeId>,
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

    /// Get all nodes for a submission (a single turn), ordered by `created_at`.
    fn get_nodes_by_submission(&self, submission_id: &str) -> Result<Vec<DagNode>, String>;

    /// Get all timelines whose fork point belongs to the given session.
    fn get_timelines_for_session(
        &self,
        session_id: &super::SessionId,
    ) -> Result<Vec<Timeline>, String>;

    /// Get the most recent DagNode on a timeline, optionally filtered by message type.
    fn latest_node_on_timeline(
        &self,
        timeline_id: i64,
        message_type: Option<MessageType>,
    ) -> Result<Option<DagNode>, String>;

    // -------------------------------------------------------------------------
    // Session CRUD
    // -------------------------------------------------------------------------

    /// Persist a new session. Idempotent (ignores if ID already exists).
    fn create_session(&self, session: &Session) -> Result<(), String>;

    /// Look up a session by its ID.
    fn get_session(&self, session_id: &super::SessionId) -> Result<Option<Session>, String>;

    /// Look up a session by its friendly name.
    fn get_session_by_name(&self, name: &str) -> Result<Option<Session>, String>;
}

// ============================================================================
// In-Memory Implementation (for testing)
// ============================================================================

/// In-memory DagStore for unit tests that don't need SQLite.
pub struct InMemoryDagStore {
    nodes: std::sync::Mutex<Vec<DagNode>>,
    timelines: std::sync::Mutex<Vec<Timeline>>,
    checkout_states: std::sync::Mutex<std::collections::HashMap<String, String>>,
    sessions: std::sync::Mutex<Vec<Session>>,
}

impl InMemoryDagStore {
    pub fn new() -> Self {
        Self {
            nodes: std::sync::Mutex::new(Vec::new()),
            timelines: std::sync::Mutex::new(Vec::new()),
            checkout_states: std::sync::Mutex::new(std::collections::HashMap::new()),
            sessions: std::sync::Mutex::new(Vec::new()),
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
        // Idempotent: skip if ID already exists
        if !nodes.iter().any(|n| n.id == node.id) {
            nodes.push(node.clone());
        }
        Ok(())
    }

    fn get_node(&self, id: &super::DagNodeId) -> Result<Option<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        Ok(nodes.iter().find(|n| n.id == *id).cloned())
    }

    fn get_node_by_prefix(&self, prefix: &str) -> Result<Option<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        let matches: Vec<_> = nodes
            .iter()
            .filter(|n| n.id.as_str().starts_with(prefix))
            .collect();
        match matches.len() {
            0 => Ok(None),
            1 => Ok(Some(matches[0].clone())),
            n => Err(format!("ambiguous ID prefix '{}': {} matches", prefix, n)),
        }
    }

    fn get_session_nodes(&self, session_id: &super::SessionId) -> Result<Vec<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        let mut result: Vec<DagNode> = nodes
            .iter()
            .filter(|n| *n.session_id() == *session_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    fn get_children(&self, parent_id: &super::DagNodeId) -> Result<Vec<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        Ok(nodes
            .iter()
            .filter(|n| n.parent_id == *parent_id)
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
            .filter(|n| {
                let (from, to) = n.message.from_to();
                from == agent_name || to == agent_name
            })
            .find_map(|n| n.message.state().map(|s| s.to_string())))
    }

    fn latest_node_hash(
        &self,
        session_id: &super::SessionId,
    ) -> Result<Option<super::DagNodeId>, String> {
        let nodes = self.nodes.lock().unwrap();
        Ok(nodes
            .iter()
            .rev()
            .find(|n| *n.session_id() == *session_id)
            .map(|n| n.id.clone()))
    }

    fn set_checkout_state(&self, agent_name: &str, state: &str) -> Result<(), String> {
        self.checkout_states
            .lock()
            .unwrap()
            .insert(agent_name.to_string(), state.to_string());
        Ok(())
    }

    fn create_timeline(
        &self,
        branch_name: &str,
        session_id: &super::SessionId,
        parent_id: Option<i64>,
        fork_point: Option<&super::DagNodeId>,
    ) -> Result<i64, String> {
        let mut timelines = self.timelines.lock().unwrap();
        let id = timelines.len() as i64 + 1;
        timelines.push(Timeline {
            id,
            branch_name: branch_name.to_string(),
            session_id: session_id.clone(),
            parent_timeline_id: parent_id,
            fork_point: fork_point.cloned(),
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
        let mut sessions: std::collections::HashMap<super::SessionId, Vec<&DagNode>> =
            std::collections::HashMap::new();
        for node in nodes.iter() {
            sessions
                .entry(node.session_id().clone())
                .or_default()
                .push(node);
        }

        let mut summaries: Vec<SessionSummary> = sessions
            .into_iter()
            .map(|(session_id, mut nodes)| {
                nodes.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                let agent_name = nodes
                    .iter()
                    .find(|n| n.message_type() == MessageType::Invoke)
                    .map(|n| n.message.from_to().1)
                    .unwrap_or_default();
                let started_at = nodes.first().map(|n| n.created_at).unwrap_or_default();
                let message_count = nodes
                    .iter()
                    .filter(|n| {
                        n.message_type() == MessageType::Invoke
                            || n.message_type() == MessageType::Complete
                    })
                    .count();
                let is_open = nodes
                    .last()
                    .map(|n| n.message_type() != MessageType::Complete)
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

    fn get_nodes_by_submission(&self, submission_id: &str) -> Result<Vec<DagNode>, String> {
        let nodes = self.nodes.lock().unwrap();
        let mut result: Vec<DagNode> = nodes
            .iter()
            .filter(|n| n.submission_id().as_str() == submission_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    fn get_timelines_for_session(
        &self,
        session_id: &super::SessionId,
    ) -> Result<Vec<Timeline>, String> {
        let nodes = self.nodes.lock().unwrap();
        let session_hashes: std::collections::HashSet<&str> = nodes
            .iter()
            .filter(|n| *n.session_id() == *session_id)
            .map(|n| n.id.as_str())
            .collect();

        let timelines = self.timelines.lock().unwrap();
        Ok(timelines
            .iter()
            .filter(|t| {
                t.fork_point
                    .as_ref()
                    .map(|fp| session_hashes.contains(fp.as_str()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect())
    }

    fn latest_node_on_timeline(
        &self,
        timeline_id: i64,
        message_type: Option<MessageType>,
    ) -> Result<Option<DagNode>, String> {
        let tl_id_str = timeline_id.to_string();
        let nodes = self.nodes.lock().unwrap();
        Ok(nodes
            .iter()
            .rev()
            .filter(|n| n.timeline_id().as_str() == tl_id_str)
            .find(|n| message_type.is_none_or(|mt| n.message_type() == mt))
            .cloned())
    }

    fn create_session(&self, session: &Session) -> Result<(), String> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions
            .iter()
            .any(|s| s.session.as_str() == session.session.as_str())
        {
            return Ok(());
        }
        sessions.push(session.clone());
        Ok(())
    }

    fn get_session(&self, session_id: &super::SessionId) -> Result<Option<Session>, String> {
        let sessions = self.sessions.lock().unwrap();
        Ok(sessions.iter().find(|s| s.session == *session_id).cloned())
    }

    fn get_session_by_name(&self, name: &str) -> Result<Option<Session>, String> {
        let sessions = self.sessions.lock().unwrap();
        Ok(sessions.iter().find(|s| s.name == name).cloned())
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
            MessageType::Fork,
        ] {
            assert_eq!(MessageType::from_str(mt.as_str()), Ok(mt));
        }
    }

    #[test]
    fn message_type_from_unknown_returns_none() {
        assert!(MessageType::from_str("unknown").is_err());
    }

    // --- hash_dag_node tests ---

    use crate::domain::{DagNodeId, SessionId};

    fn did(s: &str) -> DagNodeId {
        DagNodeId::from(s.to_string())
    }

    fn sid(s: &str) -> SessionId {
        SessionId::try_from(s.to_string()).unwrap()
    }

    // Valid UUIDs for testing
    const SES_1: &str = "d4761d76-dee4-4ebf-9df4-43b52efa4f78";
    const SES_2: &str = "e2660cff-33d6-4428-acca-2d297dcc1cad";

    #[test]
    fn hash_is_valid_sha256() {
        let hash = hash_dag_node(b"hello", &did(""), &MessageType::Invoke, b"", &sid(SES_1));
        assert_eq!(hash.as_str().len(), 64);
        assert!(hash.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_changes_with_parent() {
        let h1 = hash_dag_node(b"payload", &did(""), &MessageType::Invoke, b"", &sid(SES_1));
        let h2 = hash_dag_node(
            b"payload",
            &did("parent-abc"),
            &MessageType::Invoke,
            b"",
            &sid(SES_1),
        );
        assert_ne!(h1, h2, "Merkle property: different parent → different hash");
    }

    #[test]
    fn hash_changes_with_payload() {
        let h1 = hash_dag_node(
            b"payload-a",
            &did(""),
            &MessageType::Invoke,
            b"",
            &sid(SES_1),
        );
        let h2 = hash_dag_node(
            b"payload-b",
            &did(""),
            &MessageType::Invoke,
            b"",
            &sid(SES_1),
        );
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_changes_with_message_type() {
        let h1 = hash_dag_node(b"payload", &did(""), &MessageType::Invoke, b"", &sid(SES_1));
        let h2 = hash_dag_node(
            b"payload",
            &did(""),
            &MessageType::Complete,
            b"",
            &sid(SES_1),
        );
        assert_ne!(h1, h2, "same payload, different type → different hash");
    }

    #[test]
    fn hash_changes_with_diagnostics() {
        let h1 = hash_dag_node(b"payload", &did(""), &MessageType::Invoke, b"", &sid(SES_1));
        let h2 = hash_dag_node(
            b"payload",
            &did(""),
            &MessageType::Invoke,
            b"{\"duration_ms\":100}",
            &sid(SES_1),
        );
        assert_ne!(h1, h2, "different diagnostics → different hash");
    }

    #[test]
    fn hash_changes_with_session_id() {
        let h1 = hash_dag_node(b"payload", &did(""), &MessageType::Invoke, b"", &sid(SES_1));
        let h2 = hash_dag_node(b"payload", &did(""), &MessageType::Invoke, b"", &sid(SES_2));
        assert_ne!(
            h1, h2,
            "same payload in different sessions → different hash"
        );
    }

    #[test]
    fn hash_is_deterministic() {
        let h1 = hash_dag_node(
            b"same",
            &did("p"),
            &MessageType::Request,
            b"diag",
            &sid(SES_1),
        );
        let h2 = hash_dag_node(
            b"same",
            &did("p"),
            &MessageType::Request,
            b"diag",
            &sid(SES_1),
        );
        assert_eq!(h1, h2);
    }
}
