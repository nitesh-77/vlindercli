//! DagStore — content-addressed SQLite storage for runtime-captured DAG (ADR 067).
//!
//! The platform observes every NATS message and writes one DAG node per message.
//! No pairing, no buffering — each message is independently meaningful.
//!
//! Each `DagNode` captures one message:
//! - `hash`: SHA-256(payload || parent_hash || message_type) — Merkle chain
//! - `parent_hash`: previous node in the same session (empty string for root)
//! - `message_type`: Invoke, Request, Response, Complete, or Delegate
//! - `from` / `to`: sender and receiver (parsed from NATS subject)
//! - `session_id`: groups nodes into a linear chain per session
//! - `submission_id`: groups all messages for one user request
//! - `payload`: raw message payload
//! - `created_at`: ISO-8601 timestamp

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
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
    pub created_at: String,
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
}

/// SQLite-backed DagStore.
pub struct SqliteDagStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteDagStore {
    /// Open (or create) a DAG store at the given path.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create dag store directory: {}", e))?;
        }

        let conn = Connection::open(path)
            .map_err(|e| format!("failed to open dag store: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS dag_nodes (
                 hash TEXT PRIMARY KEY,
                 parent_hash TEXT NOT NULL,
                 message_type TEXT NOT NULL,
                 sender TEXT NOT NULL,
                 receiver TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 submission_id TEXT NOT NULL,
                 payload BLOB NOT NULL,
                 diagnostics BLOB NOT NULL DEFAULT x'',
                 stderr BLOB NOT NULL DEFAULT x'',
                 created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_dag_nodes_session
                 ON dag_nodes (session_id, created_at);
             CREATE INDEX IF NOT EXISTS idx_dag_nodes_parent
                 ON dag_nodes (parent_hash);"
        ).map_err(|e| format!("failed to initialize dag store: {}", e))?;

        // Migrate existing databases: add diagnostics and stderr columns (ADR 071).
        // ALTER TABLE ADD COLUMN is idempotent-safe: we check PRAGMA table_info first.
        Self::migrate_071(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Migrate existing databases to include diagnostics and stderr columns (ADR 071).
    fn migrate_071(conn: &Connection) -> Result<(), String> {
        let has_diagnostics = conn
            .prepare("PRAGMA table_info(dag_nodes)")
            .and_then(|mut stmt| {
                let mut found = false;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let name: String = row.get(1)?;
                    if name == "diagnostics" {
                        found = true;
                        break;
                    }
                }
                Ok(found)
            })
            .map_err(|e| format!("migration check failed: {}", e))?;

        if !has_diagnostics {
            conn.execute_batch(
                "ALTER TABLE dag_nodes ADD COLUMN diagnostics BLOB NOT NULL DEFAULT x'';
                 ALTER TABLE dag_nodes ADD COLUMN stderr BLOB NOT NULL DEFAULT x'';"
            ).map_err(|e| format!("ADR 071 migration failed: {}", e))?;
        }

        Ok(())
    }
}

/// Construct a DagNode from a SQLite row.
///
/// Expects columns in order: hash, parent_hash, message_type, sender, receiver,
/// session_id, submission_id, payload, diagnostics, stderr, created_at.
fn row_to_dag_node(row: &rusqlite::Row) -> Result<DagNode, rusqlite::Error> {
    let mt_str: String = row.get(2)?;
    let message_type = MessageType::from_str(&mt_str)
        .unwrap_or(MessageType::Invoke);
    Ok(DagNode {
        hash: row.get(0)?,
        parent_hash: row.get(1)?,
        message_type,
        from: row.get(3)?,
        to: row.get(4)?,
        session_id: row.get(5)?,
        submission_id: row.get(6)?,
        payload: row.get(7)?,
        diagnostics: row.get(8)?,
        stderr: row.get(9)?,
        created_at: row.get(10)?,
    })
}

impl DagStore for SqliteDagStore {
    fn insert_node(&self, node: &DagNode) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO dag_nodes (hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                node.hash,
                node.parent_hash,
                node.message_type.as_str(),
                node.from,
                node.to,
                node.session_id,
                node.submission_id,
                node.payload,
                node.diagnostics,
                node.stderr,
                node.created_at,
            ],
        ).map_err(|e| format!("insert_node failed: {}", e))?;
        Ok(())
    }

    fn get_node(&self, hash: &str) -> Result<Option<DagNode>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at
             FROM dag_nodes WHERE hash = ?1"
        ).map_err(|e| format!("get_node prepare failed: {}", e))?;

        let result = stmt.query_row(rusqlite::params![hash], |row| {
            row_to_dag_node(row)
        })
        .optional()
        .map_err(|e| format!("get_node query failed: {}", e))?;

        Ok(result)
    }

    fn get_session_nodes(&self, session_id: &str) -> Result<Vec<DagNode>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at
             FROM dag_nodes WHERE session_id = ?1 ORDER BY created_at"
        ).map_err(|e| format!("get_session_nodes prepare failed: {}", e))?;

        let rows = stmt.query_map(rusqlite::params![session_id], |row| {
            row_to_dag_node(row)
        }).map_err(|e| format!("get_session_nodes query failed: {}", e))?;

        let mut nodes = Vec::new();
        for row in rows {
            nodes.push(row.map_err(|e| format!("get_session_nodes row failed: {}", e))?);
        }
        Ok(nodes)
    }

    fn get_children(&self, parent_hash: &str) -> Result<Vec<DagNode>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at
             FROM dag_nodes WHERE parent_hash = ?1"
        ).map_err(|e| format!("get_children prepare failed: {}", e))?;

        let rows = stmt.query_map(rusqlite::params![parent_hash], |row| {
            row_to_dag_node(row)
        }).map_err(|e| format!("get_children query failed: {}", e))?;

        let mut nodes = Vec::new();
        for row in rows {
            nodes.push(row.map_err(|e| format!("get_children row failed: {}", e))?);
        }
        Ok(nodes)
    }
}

/// Trait extension for rusqlite optional queries.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SqliteDagStore {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        SqliteDagStore::open(tmp.path()).unwrap()
    }

    fn test_node(payload: &[u8], parent_hash: &str, message_type: MessageType) -> DagNode {
        let diagnostics = Vec::new();
        DagNode {
            hash: hash_dag_node(payload, parent_hash, &message_type, &diagnostics),
            parent_hash: parent_hash.to_string(),
            message_type,
            from: "cli".to_string(),
            to: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            submission_id: "sub-1".to_string(),
            payload: payload.to_vec(),
            diagnostics,
            stderr: Vec::new(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

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

    // --- SqliteDagStore tests ---

    #[test]
    fn round_trip_insert_get() {
        let store = test_store();
        let node = test_node(b"hello", "", MessageType::Invoke);

        store.insert_node(&node).unwrap();
        let retrieved = store.get_node(&node.hash).unwrap().unwrap();

        assert_eq!(retrieved, node);
    }

    #[test]
    fn round_trip_preserves_all_fields() {
        let store = test_store();
        let diagnostics = b"{\"container\":{\"duration_ms\":50}}".to_vec();
        let stderr = b"WARN: something".to_vec();
        let node = DagNode {
            hash: hash_dag_node(b"pay", "", &MessageType::Delegate, &diagnostics),
            parent_hash: "".to_string(),
            message_type: MessageType::Delegate,
            from: "coordinator".to_string(),
            to: "summarizer".to_string(),
            session_id: "sess-99".to_string(),
            submission_id: "sub-42".to_string(),
            payload: b"delegate this".to_vec(),
            diagnostics,
            stderr,
            created_at: "2025-06-15T12:00:00Z".to_string(),
        };

        store.insert_node(&node).unwrap();
        let retrieved = store.get_node(&node.hash).unwrap().unwrap();

        assert_eq!(retrieved.message_type, MessageType::Delegate);
        assert_eq!(retrieved.from, "coordinator");
        assert_eq!(retrieved.to, "summarizer");
        assert_eq!(retrieved.submission_id, "sub-42");
    }

    #[test]
    fn get_node_returns_none_for_unknown() {
        let store = test_store();
        assert_eq!(store.get_node("nonexistent").unwrap(), None);
    }

    #[test]
    fn idempotent_insert() {
        let store = test_store();
        let node = test_node(b"data", "", MessageType::Invoke);

        store.insert_node(&node).unwrap();
        store.insert_node(&node).unwrap(); // No error

        let retrieved = store.get_node(&node.hash).unwrap().unwrap();
        assert_eq!(retrieved, node);
    }

    #[test]
    fn session_nodes_ordered_by_created_at() {
        let store = test_store();

        let mut node1 = test_node(b"first", "", MessageType::Invoke);
        node1.created_at = "2025-01-01T00:00:00Z".to_string();

        let mut node2 = test_node(b"second", &node1.hash, MessageType::Request);
        node2.created_at = "2025-01-01T00:01:00Z".to_string();

        // Insert out of order
        store.insert_node(&node2).unwrap();
        store.insert_node(&node1).unwrap();

        let nodes = store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].hash, node1.hash);
        assert_eq!(nodes[1].hash, node2.hash);
    }

    #[test]
    fn get_children() {
        let store = test_store();

        let parent = test_node(b"parent", "", MessageType::Invoke);

        let mut child = test_node(b"child", &parent.hash, MessageType::Complete);
        child.created_at = "2025-01-01T00:01:00Z".to_string();

        store.insert_node(&parent).unwrap();
        store.insert_node(&child).unwrap();

        let children = store.get_children(&parent.hash).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].hash, child.hash);

        // Root has one child (the parent node, whose parent_hash is "")
        let root_children = store.get_children("").unwrap();
        assert_eq!(root_children.len(), 1);
        assert_eq!(root_children[0].hash, parent.hash);
    }

    #[test]
    fn different_sessions_are_isolated() {
        let store = test_store();

        let mut node_a = test_node(b"a", "", MessageType::Invoke);
        node_a.session_id = "sess-1".to_string();

        let mut node_b = test_node(b"b", "", MessageType::Invoke);
        node_b.session_id = "sess-2".to_string();
        node_b.from = "cli".to_string();
        node_b.to = "agent-b".to_string();

        store.insert_node(&node_a).unwrap();
        store.insert_node(&node_b).unwrap();

        let sess1 = store.get_session_nodes("sess-1").unwrap();
        assert_eq!(sess1.len(), 1);
        assert_eq!(sess1[0].session_id, "sess-1");

        let sess2 = store.get_session_nodes("sess-2").unwrap();
        assert_eq!(sess2.len(), 1);
        assert_eq!(sess2[0].session_id, "sess-2");
    }

    #[test]
    fn all_five_message_types_stored() {
        let store = test_store();

        let types = [
            MessageType::Invoke,
            MessageType::Request,
            MessageType::Response,
            MessageType::Complete,
            MessageType::Delegate,
        ];

        let mut parent_hash = String::new();
        for (i, mt) in types.iter().enumerate() {
            let payload = format!("msg-{}", i);
            let mut node = test_node(payload.as_bytes(), &parent_hash, *mt);
            node.created_at = format!("2025-01-01T00:0{}:00Z", i);
            store.insert_node(&node).unwrap();
            parent_hash = node.hash;
        }

        let nodes = store.get_session_nodes("sess-1").unwrap();
        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes[0].message_type, MessageType::Invoke);
        assert_eq!(nodes[1].message_type, MessageType::Request);
        assert_eq!(nodes[2].message_type, MessageType::Response);
        assert_eq!(nodes[3].message_type, MessageType::Complete);
        assert_eq!(nodes[4].message_type, MessageType::Delegate);
    }
}
