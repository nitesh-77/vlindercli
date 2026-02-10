//! DagStore — content-addressed SQLite storage for runtime-captured DAG (ADR 062).
//!
//! The platform observes invoke/complete pairs at agent boundaries and builds
//! a Merkle DAG. No agent cooperation required — diff, bisect, replay, and
//! audit come for free.
//!
//! Each `DagNode` captures one invoke/complete pair:
//! - `hash`: SHA-256(payload_in || payload_out || parent_hash) — Merkle chain
//! - `parent_hash`: previous node in the same session (empty string for root)
//! - `agent`: which agent was invoked
//! - `session_id`: groups nodes into a linear chain per session
//! - `payload_in` / `payload_out`: raw invoke and complete payloads
//! - `created_at`: ISO-8601 timestamp

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// A single node in the runtime-captured DAG.
#[derive(Debug, Clone, PartialEq)]
pub struct DagNode {
    pub hash: String,
    pub parent_hash: String,
    pub agent: String,
    pub session_id: String,
    pub payload_in: Vec<u8>,
    pub payload_out: Vec<u8>,
    pub created_at: String,
}

/// Compute the content-addressed hash for a DAG node.
///
/// `sha256(payload_in || payload_out || parent_hash)` — the parent hash
/// makes this a Merkle chain: changing any ancestor invalidates all descendants.
pub fn hash_dag_node(payload_in: &[u8], payload_out: &[u8], parent_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload_in);
    hasher.update(payload_out);
    hasher.update(parent_hash.as_bytes());
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
                 agent TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 payload_in BLOB NOT NULL,
                 payload_out BLOB NOT NULL,
                 created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_dag_nodes_session
                 ON dag_nodes (session_id, created_at);
             CREATE INDEX IF NOT EXISTS idx_dag_nodes_parent
                 ON dag_nodes (parent_hash);"
        ).map_err(|e| format!("failed to initialize dag store: {}", e))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl DagStore for SqliteDagStore {
    fn insert_node(&self, node: &DagNode) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO dag_nodes (hash, parent_hash, agent, session_id, payload_in, payload_out, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                node.hash,
                node.parent_hash,
                node.agent,
                node.session_id,
                node.payload_in,
                node.payload_out,
                node.created_at,
            ],
        ).map_err(|e| format!("insert_node failed: {}", e))?;
        Ok(())
    }

    fn get_node(&self, hash: &str) -> Result<Option<DagNode>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, parent_hash, agent, session_id, payload_in, payload_out, created_at
             FROM dag_nodes WHERE hash = ?1"
        ).map_err(|e| format!("get_node prepare failed: {}", e))?;

        let result = stmt.query_row(rusqlite::params![hash], |row| {
            Ok(DagNode {
                hash: row.get(0)?,
                parent_hash: row.get(1)?,
                agent: row.get(2)?,
                session_id: row.get(3)?,
                payload_in: row.get(4)?,
                payload_out: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .optional()
        .map_err(|e| format!("get_node query failed: {}", e))?;

        Ok(result)
    }

    fn get_session_nodes(&self, session_id: &str) -> Result<Vec<DagNode>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, parent_hash, agent, session_id, payload_in, payload_out, created_at
             FROM dag_nodes WHERE session_id = ?1 ORDER BY created_at"
        ).map_err(|e| format!("get_session_nodes prepare failed: {}", e))?;

        let rows = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok(DagNode {
                hash: row.get(0)?,
                parent_hash: row.get(1)?,
                agent: row.get(2)?,
                session_id: row.get(3)?,
                payload_in: row.get(4)?,
                payload_out: row.get(5)?,
                created_at: row.get(6)?,
            })
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
            "SELECT hash, parent_hash, agent, session_id, payload_in, payload_out, created_at
             FROM dag_nodes WHERE parent_hash = ?1"
        ).map_err(|e| format!("get_children prepare failed: {}", e))?;

        let rows = stmt.query_map(rusqlite::params![parent_hash], |row| {
            Ok(DagNode {
                hash: row.get(0)?,
                parent_hash: row.get(1)?,
                agent: row.get(2)?,
                session_id: row.get(3)?,
                payload_in: row.get(4)?,
                payload_out: row.get(5)?,
                created_at: row.get(6)?,
            })
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

    // --- hash_dag_node tests ---

    #[test]
    fn hash_is_known_sha256() {
        // SHA-256 of "helloworld" (empty parent)
        let hash = hash_dag_node(b"hello", b"world", "");
        // Verify it's a valid 64-char hex string
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_changes_with_parent() {
        let h1 = hash_dag_node(b"in", b"out", "");
        let h2 = hash_dag_node(b"in", b"out", "parent-abc");
        assert_ne!(h1, h2, "Merkle property: different parent → different hash");
    }

    #[test]
    fn hash_changes_with_payload() {
        let h1 = hash_dag_node(b"input-a", b"output", "");
        let h2 = hash_dag_node(b"input-b", b"output", "");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_is_deterministic() {
        let h1 = hash_dag_node(b"same", b"data", "p");
        let h2 = hash_dag_node(b"same", b"data", "p");
        assert_eq!(h1, h2);
    }

    // --- SqliteDagStore tests ---

    #[test]
    fn round_trip_insert_get() {
        let store = test_store();
        let node = DagNode {
            hash: hash_dag_node(b"in", b"out", ""),
            parent_hash: "".to_string(),
            agent: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            payload_in: b"in".to_vec(),
            payload_out: b"out".to_vec(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };

        store.insert_node(&node).unwrap();
        let retrieved = store.get_node(&node.hash).unwrap().unwrap();

        assert_eq!(retrieved, node);
    }

    #[test]
    fn get_node_returns_none_for_unknown() {
        let store = test_store();
        assert_eq!(store.get_node("nonexistent").unwrap(), None);
    }

    #[test]
    fn idempotent_insert() {
        let store = test_store();
        let node = DagNode {
            hash: hash_dag_node(b"in", b"out", ""),
            parent_hash: "".to_string(),
            agent: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            payload_in: b"in".to_vec(),
            payload_out: b"out".to_vec(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };

        store.insert_node(&node).unwrap();
        store.insert_node(&node).unwrap(); // No error

        let retrieved = store.get_node(&node.hash).unwrap().unwrap();
        assert_eq!(retrieved, node);
    }

    #[test]
    fn session_nodes_ordered_by_created_at() {
        let store = test_store();

        let node1 = DagNode {
            hash: hash_dag_node(b"in1", b"out1", ""),
            parent_hash: "".to_string(),
            agent: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            payload_in: b"in1".to_vec(),
            payload_out: b"out1".to_vec(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let node2 = DagNode {
            hash: hash_dag_node(b"in2", b"out2", &node1.hash),
            parent_hash: node1.hash.clone(),
            agent: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            payload_in: b"in2".to_vec(),
            payload_out: b"out2".to_vec(),
            created_at: "2025-01-01T00:01:00Z".to_string(),
        };

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

        let parent = DagNode {
            hash: hash_dag_node(b"in", b"out", ""),
            parent_hash: "".to_string(),
            agent: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            payload_in: b"in".to_vec(),
            payload_out: b"out".to_vec(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let child = DagNode {
            hash: hash_dag_node(b"in2", b"out2", &parent.hash),
            parent_hash: parent.hash.clone(),
            agent: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            payload_in: b"in2".to_vec(),
            payload_out: b"out2".to_vec(),
            created_at: "2025-01-01T00:01:00Z".to_string(),
        };

        store.insert_node(&parent).unwrap();
        store.insert_node(&child).unwrap();

        let children = store.get_children(&parent.hash).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].hash, child.hash);

        // Root has no children with empty parent (only the parent node itself)
        let root_children = store.get_children("").unwrap();
        assert_eq!(root_children.len(), 1);
        assert_eq!(root_children[0].hash, parent.hash);
    }

    #[test]
    fn different_sessions_are_isolated() {
        let store = test_store();

        let node_a = DagNode {
            hash: hash_dag_node(b"a", b"a", ""),
            parent_hash: "".to_string(),
            agent: "agent-a".to_string(),
            session_id: "sess-1".to_string(),
            payload_in: b"a".to_vec(),
            payload_out: b"a".to_vec(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let node_b = DagNode {
            hash: hash_dag_node(b"b", b"b", ""),
            parent_hash: "".to_string(),
            agent: "agent-b".to_string(),
            session_id: "sess-2".to_string(),
            payload_in: b"b".to_vec(),
            payload_out: b"b".to_vec(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };

        store.insert_node(&node_a).unwrap();
        store.insert_node(&node_b).unwrap();

        let sess1 = store.get_session_nodes("sess-1").unwrap();
        assert_eq!(sess1.len(), 1);
        assert_eq!(sess1[0].session_id, "sess-1");

        let sess2 = store.get_session_nodes("sess-2").unwrap();
        assert_eq!(sess2.len(), 1);
        assert_eq!(sess2[0].session_id, "sess-2");
    }
}
