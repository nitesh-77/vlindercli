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

/// The five message types in the Vlinder protocol (ADR 044).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Invoke,
    Request,
    Response,
    Complete,
    Delegate,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageType::Invoke => "invoke",
            MessageType::Request => "request",
            MessageType::Response => "response",
            MessageType::Complete => "complete",
            MessageType::Delegate => "delegate",
        }
    }

    /// Parse from string. Accepts both canonical names and NATS subject
    /// abbreviations (`req` → Request, `res` → Response).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "invoke" => Some(MessageType::Invoke),
            "req" | "request" => Some(MessageType::Request),
            "res" | "response" => Some(MessageType::Response),
            "complete" => Some(MessageType::Complete),
            "delegate" => Some(MessageType::Delegate),
            _ => None,
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
}

/// Compute the content-addressed hash for a DAG node.
///
/// `sha256(payload || parent_hash || message_type || diagnostics)` — the parent hash
/// makes this a Merkle chain: changing any ancestor invalidates all descendants.
/// The message type is included so identical payloads with different types
/// produce different hashes. Diagnostics are included for integrity (ADR 071).
pub fn hash_dag_node(payload: &[u8], parent_hash: &str, message_type: &MessageType, diagnostics: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    hasher.update(parent_hash.as_bytes());
    hasher.update(message_type.as_str().as_bytes());
    hasher.update(diagnostics);
    format!("{:x}", hasher.finalize())
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- MessageType tests ---

    #[test]
    fn message_type_round_trips() {
        for mt in [
            MessageType::Invoke,
            MessageType::Request,
            MessageType::Response,
            MessageType::Complete,
            MessageType::Delegate,
        ] {
            assert_eq!(MessageType::from_str(mt.as_str()), Some(mt));
        }
    }

    #[test]
    fn message_type_from_unknown_returns_none() {
        assert_eq!(MessageType::from_str("unknown"), None);
    }

    // --- hash_dag_node tests ---

    #[test]
    fn hash_is_valid_sha256() {
        let hash = hash_dag_node(b"hello", "", &MessageType::Invoke, b"");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_changes_with_parent() {
        let h1 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"");
        let h2 = hash_dag_node(b"payload", "parent-abc", &MessageType::Invoke, b"");
        assert_ne!(h1, h2, "Merkle property: different parent → different hash");
    }

    #[test]
    fn hash_changes_with_payload() {
        let h1 = hash_dag_node(b"payload-a", "", &MessageType::Invoke, b"");
        let h2 = hash_dag_node(b"payload-b", "", &MessageType::Invoke, b"");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_changes_with_message_type() {
        let h1 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"");
        let h2 = hash_dag_node(b"payload", "", &MessageType::Complete, b"");
        assert_ne!(h1, h2, "same payload, different type → different hash");
    }

    #[test]
    fn hash_changes_with_diagnostics() {
        let h1 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"");
        let h2 = hash_dag_node(b"payload", "", &MessageType::Invoke, b"{\"duration_ms\":100}");
        assert_ne!(h1, h2, "different diagnostics → different hash");
    }

    #[test]
    fn hash_is_deterministic() {
        let h1 = hash_dag_node(b"same", "p", &MessageType::Request, b"diag");
        let h2 = hash_dag_node(b"same", "p", &MessageType::Request, b"diag");
        assert_eq!(h1, h2);
    }
}
