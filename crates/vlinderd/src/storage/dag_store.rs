//! SqliteDagStore — SQLite-backed persistence for the Merkle DAG (ADR 067).
//!
//! Domain types (`DagNode`, `DagStore`, `MessageType`, `hash_dag_node`) live
//! in `crate::domain`. This module provides the SQLite implementation.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::domain::{DagNode, DagStore, MessageType, Timeline};

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
                 created_at TEXT NOT NULL,
                 state TEXT,
                 protocol_version TEXT NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS idx_dag_nodes_session
                 ON dag_nodes (session_id, created_at);
             CREATE INDEX IF NOT EXISTS idx_dag_nodes_parent
                 ON dag_nodes (parent_hash);
             CREATE TABLE IF NOT EXISTS checkout_state (
                 agent_name TEXT PRIMARY KEY,
                 state_hash TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS timelines (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 branch_name TEXT NOT NULL UNIQUE,
                 parent_timeline_id INTEGER,
                 fork_point TEXT,
                 created_at TEXT NOT NULL,
                 broken_at TEXT,
                 FOREIGN KEY (parent_timeline_id) REFERENCES timelines(id)
             );"
        ).map_err(|e| format!("failed to initialize dag store: {}", e))?;

        // Migrate existing databases: add diagnostics and stderr columns (ADR 071).
        // ALTER TABLE ADD COLUMN is idempotent-safe: we check PRAGMA table_info first.
        Self::migrate_071(&conn)?;
        Self::migrate_state(&conn)?;
        Self::migrate_protocol_version(&conn)?;
        Self::migrate_timelines(&conn)?;

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

    /// Migrate existing databases to include the state column (ADR 055).
    fn migrate_state(conn: &Connection) -> Result<(), String> {
        let has_state = conn
            .prepare("PRAGMA table_info(dag_nodes)")
            .and_then(|mut stmt| {
                let mut found = false;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let name: String = row.get(1)?;
                    if name == "state" {
                        found = true;
                        break;
                    }
                }
                Ok(found)
            })
            .map_err(|e| format!("migration check failed: {}", e))?;

        if !has_state {
            conn.execute_batch(
                "ALTER TABLE dag_nodes ADD COLUMN state TEXT;"
            ).map_err(|e| format!("state migration failed: {}", e))?;
        }

        Ok(())
    }

    /// Migrate existing databases to include the timelines table (ADR 093).
    fn migrate_timelines(conn: &Connection) -> Result<(), String> {
        // Check if table exists
        let exists: bool = conn
            .prepare("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='timelines'")
            .and_then(|mut stmt| stmt.query_row([], |row| row.get::<_, i64>(0)))
            .map(|count| count > 0)
            .map_err(|e| format!("timelines migration check failed: {}", e))?;

        if !exists {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS timelines (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     branch_name TEXT NOT NULL UNIQUE,
                     parent_timeline_id INTEGER,
                     fork_point TEXT,
                     created_at TEXT NOT NULL,
                     broken_at TEXT,
                     FOREIGN KEY (parent_timeline_id) REFERENCES timelines(id)
                 );"
            ).map_err(|e| format!("timelines migration failed: {}", e))?;
        }

        Ok(())
    }

    /// Migrate existing databases to include the protocol_version column.
    fn migrate_protocol_version(conn: &Connection) -> Result<(), String> {
        let has_col = conn
            .prepare("PRAGMA table_info(dag_nodes)")
            .and_then(|mut stmt| {
                let mut found = false;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let name: String = row.get(1)?;
                    if name == "protocol_version" {
                        found = true;
                        break;
                    }
                }
                Ok(found)
            })
            .map_err(|e| format!("migration check failed: {}", e))?;

        if !has_col {
            conn.execute_batch(
                "ALTER TABLE dag_nodes ADD COLUMN protocol_version TEXT NOT NULL DEFAULT '';"
            ).map_err(|e| format!("protocol_version migration failed: {}", e))?;
        }

        Ok(())
    }
}

/// Construct a Timeline from a SQLite row.
///
/// Expects columns in order: id, branch_name, parent_timeline_id, fork_point,
/// created_at, broken_at.
fn row_to_timeline(row: &rusqlite::Row) -> Result<Timeline, rusqlite::Error> {
    let created_at_str: String = row.get(4)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_default();
    let broken_at_str: Option<String> = row.get(5)?;
    let broken_at = broken_at_str.and_then(|s| {
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
    });
    Ok(Timeline {
        id: row.get(0)?,
        branch_name: row.get(1)?,
        parent_timeline_id: row.get(2)?,
        fork_point: row.get(3)?,
        created_at,
        broken_at,
    })
}

/// Construct a DagNode from a SQLite row.
///
/// Expects columns in order: hash, parent_hash, message_type, sender, receiver,
/// session_id, submission_id, payload, diagnostics, stderr, created_at, state,
/// protocol_version.
fn row_to_dag_node(row: &rusqlite::Row) -> Result<DagNode, rusqlite::Error> {
    let mt_str: String = row.get(2)?;
    let message_type = MessageType::from_str(&mt_str)
        .ok_or_else(|| rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            format!("unknown message type: {}", mt_str).into(),
        ))?;
    let created_at_str: String = row.get(10)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_default();
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
        created_at,
        state: row.get(11)?,
        protocol_version: row.get(12)?,
    })
}

impl DagStore for SqliteDagStore {
    fn insert_node(&self, node: &DagNode) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO dag_nodes (hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at, state, protocol_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                node.created_at.to_rfc3339(),
                node.state,
                node.protocol_version,
            ],
        ).map_err(|e| format!("insert_node failed: {}", e))?;

        // Clear checkout override when a Complete with state is recorded.
        // The agent is the sender on Complete messages.
        if node.message_type == MessageType::Complete && node.state.is_some() {
            conn.execute(
                "DELETE FROM checkout_state WHERE agent_name = ?1",
                rusqlite::params![node.from],
            ).map_err(|e| format!("clear checkout_state failed: {}", e))?;
        }

        Ok(())
    }

    fn get_node(&self, hash: &str) -> Result<Option<DagNode>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at, state, protocol_version
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
            "SELECT hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at, state, protocol_version
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
            "SELECT hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at, state, protocol_version
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

    fn latest_node_hash(&self, session_id: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash FROM dag_nodes
             WHERE session_id = ?1
             ORDER BY created_at DESC
             LIMIT 1"
        ).map_err(|e| format!("latest_node_hash prepare failed: {}", e))?;

        let result: Option<String> = stmt.query_row(rusqlite::params![session_id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| format!("latest_node_hash query failed: {}", e))?;

        Ok(result)
    }

    fn latest_state(&self, agent_name: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().unwrap();

        // Check checkout override first (ADR 081).
        let mut stmt = conn.prepare(
            "SELECT state_hash FROM checkout_state WHERE agent_name = ?1"
        ).map_err(|e| format!("checkout_state prepare failed: {}", e))?;

        let override_state: Option<String> = stmt.query_row(rusqlite::params![agent_name], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| format!("checkout_state query failed: {}", e))?;

        if override_state.is_some() {
            return Ok(override_state);
        }

        // Fall back to latest state from dag_nodes.
        let mut stmt = conn.prepare(
            "SELECT state FROM dag_nodes
             WHERE state IS NOT NULL AND state != ''
               AND (sender = ?1 OR receiver = ?1)
             ORDER BY created_at DESC
             LIMIT 1"
        ).map_err(|e| format!("latest_state prepare failed: {}", e))?;

        let result: Option<String> = stmt.query_row(rusqlite::params![agent_name], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| format!("latest_state query failed: {}", e))?;

        Ok(result)
    }

    fn set_checkout_state(&self, agent_name: &str, state: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO checkout_state (agent_name, state_hash) VALUES (?1, ?2)",
            rusqlite::params![agent_name, state],
        ).map_err(|e| format!("set_checkout_state failed: {}", e))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Timeline methods (ADR 093)
    // -------------------------------------------------------------------------

    fn ensure_main_timeline(&self) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO timelines (id, branch_name, created_at) VALUES (1, 'main', ?1)",
            rusqlite::params![Utc::now().to_rfc3339()],
        ).map_err(|e| format!("ensure_main_timeline failed: {}", e))?;
        Ok(1)
    }

    fn create_timeline(
        &self,
        branch_name: &str,
        parent_id: Option<i64>,
        fork_point: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO timelines (branch_name, parent_timeline_id, fork_point, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                branch_name,
                parent_id,
                fork_point,
                Utc::now().to_rfc3339(),
            ],
        ).map_err(|e| format!("create_timeline failed: {}", e))?;
        Ok(conn.last_insert_rowid())
    }

    fn get_timeline_by_branch(&self, branch_name: &str) -> Result<Option<Timeline>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, branch_name, parent_timeline_id, fork_point, created_at, broken_at
             FROM timelines WHERE branch_name = ?1"
        ).map_err(|e| format!("get_timeline_by_branch prepare failed: {}", e))?;

        stmt.query_row(rusqlite::params![branch_name], |row| row_to_timeline(row))
            .optional()
            .map_err(|e| format!("get_timeline_by_branch query failed: {}", e))
    }

    fn get_timeline(&self, id: i64) -> Result<Option<Timeline>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, branch_name, parent_timeline_id, fork_point, created_at, broken_at
             FROM timelines WHERE id = ?1"
        ).map_err(|e| format!("get_timeline prepare failed: {}", e))?;

        stmt.query_row(rusqlite::params![id], |row| row_to_timeline(row))
            .optional()
            .map_err(|e| format!("get_timeline query failed: {}", e))
    }

    fn seal_timeline(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE timelines SET broken_at = ?1 WHERE id = ?2",
            rusqlite::params![Utc::now().to_rfc3339(), id],
        ).map_err(|e| format!("seal_timeline failed: {}", e))?;
        Ok(())
    }

    fn rename_timeline(&self, id: i64, new_name: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE timelines SET branch_name = ?1 WHERE id = ?2",
            rusqlite::params![new_name, id],
        ).map_err(|e| format!("rename_timeline failed: {}", e))?;
        Ok(())
    }

    fn is_timeline_sealed(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT broken_at FROM timelines WHERE id = ?1"
        ).map_err(|e| format!("is_timeline_sealed prepare failed: {}", e))?;

        let broken_at: Option<String> = stmt.query_row(rusqlite::params![id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| format!("is_timeline_sealed query failed: {}", e))?
        .flatten();

        Ok(broken_at.is_some())
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
    use chrono::TimeZone;
    use crate::domain::hash_dag_node;

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
            created_at: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            state: None,
            protocol_version: String::new(),
        }
    }

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
            created_at: Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap(),
            state: Some("abc123".to_string()),
            protocol_version: "0.1.0".to_string(),
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
        node1.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

        let mut node2 = test_node(b"second", &node1.hash, MessageType::Request);
        node2.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 1, 0).unwrap();

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
        child.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 1, 0).unwrap();

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
    fn latest_state_returns_most_recent() {
        let store = test_store();

        // Insert two nodes with state — different timestamps
        let mut node1 = test_node(b"first", "", MessageType::Response);
        node1.state = Some("old-state".to_string());
        node1.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

        let mut node2 = test_node(b"second", &node1.hash, MessageType::Response);
        node2.state = Some("new-state".to_string());
        node2.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 1, 0).unwrap();

        store.insert_node(&node1).unwrap();
        store.insert_node(&node2).unwrap();

        // agent-a is the `to` field on test nodes
        let state = store.latest_state("agent-a").unwrap();
        assert_eq!(state, Some("new-state".to_string()));
    }

    #[test]
    fn latest_state_returns_none_when_no_state() {
        let store = test_store();

        // Insert nodes without state
        let node = test_node(b"payload", "", MessageType::Invoke);
        store.insert_node(&node).unwrap();

        let state = store.latest_state("agent-a").unwrap();
        assert_eq!(state, None);
    }

    #[test]
    fn latest_state_filters_by_agent() {
        let store = test_store();

        // Node for agent-a
        let mut node_a = test_node(b"a-data", "", MessageType::Response);
        node_a.to = "agent-a".to_string();
        node_a.state = Some("state-a".to_string());
        node_a.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

        // Node for agent-b
        let mut node_b = test_node(b"b-data", "", MessageType::Response);
        node_b.to = "agent-b".to_string();
        node_b.from = "cli".to_string();
        node_b.state = Some("state-b".to_string());
        node_b.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 1, 0).unwrap();

        store.insert_node(&node_a).unwrap();
        store.insert_node(&node_b).unwrap();

        assert_eq!(store.latest_state("agent-a").unwrap(), Some("state-a".to_string()));
        assert_eq!(store.latest_state("agent-b").unwrap(), Some("state-b".to_string()));
        assert_eq!(store.latest_state("agent-c").unwrap(), None);
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
            node.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, i as u32, 0).unwrap();
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

    #[test]
    fn latest_node_hash_returns_most_recent() {
        let store = test_store();

        let mut node1 = test_node(b"first", "", MessageType::Invoke);
        node1.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();

        let mut node2 = test_node(b"second", &node1.hash, MessageType::Request);
        node2.created_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 1, 0).unwrap();

        store.insert_node(&node1).unwrap();
        store.insert_node(&node2).unwrap();

        let hash = store.latest_node_hash("sess-1").unwrap();
        assert_eq!(hash, Some(node2.hash));
    }

    #[test]
    fn latest_node_hash_returns_none_for_empty_session() {
        let store = test_store();
        let hash = store.latest_node_hash("nonexistent").unwrap();
        assert_eq!(hash, None);
    }

    #[test]
    fn checkout_state_overrides_latest_state() {
        let store = test_store();

        // Insert a node with state
        let mut node = test_node(b"complete", "", MessageType::Complete);
        node.from = "todoapp".to_string();
        node.to = "cli".to_string();
        node.state = Some("real-state".to_string());
        store.insert_node(&node).unwrap();

        assert_eq!(store.latest_state("todoapp").unwrap(), Some("real-state".to_string()));

        // Set checkout override
        store.set_checkout_state("todoapp", "checked-out-state").unwrap();
        assert_eq!(store.latest_state("todoapp").unwrap(), Some("checked-out-state".to_string()));
    }

    #[test]
    fn insert_complete_clears_checkout_state() {
        let store = test_store();

        // Set checkout override
        store.set_checkout_state("todoapp", "old-state").unwrap();
        assert_eq!(store.latest_state("todoapp").unwrap(), Some("old-state".to_string()));

        // Insert a Complete with new state — should clear override
        let mut node = test_node(b"new-complete", "", MessageType::Complete);
        node.from = "todoapp".to_string();
        node.to = "cli".to_string();
        node.state = Some("new-state".to_string());
        node.created_at = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
        store.insert_node(&node).unwrap();

        // Override cleared — latest_state returns the real state
        assert_eq!(store.latest_state("todoapp").unwrap(), Some("new-state".to_string()));
    }

    #[test]
    fn insert_non_complete_does_not_clear_checkout_state() {
        let store = test_store();

        store.set_checkout_state("agent-a", "checkout-state").unwrap();

        // Insert a Response with state — should NOT clear override
        let mut node = test_node(b"response", "", MessageType::Response);
        node.state = Some("response-state".to_string());
        store.insert_node(&node).unwrap();

        assert_eq!(store.latest_state("agent-a").unwrap(), Some("checkout-state".to_string()));
    }

    // ========================================================================
    // Timeline tests (ADR 093)
    // ========================================================================

    #[test]
    fn ensure_main_timeline_creates_row_1() {
        let store = test_store();
        let id = store.ensure_main_timeline().unwrap();
        assert_eq!(id, 1);

        let tl = store.get_timeline(1).unwrap().unwrap();
        assert_eq!(tl.branch_name, "main");
        assert!(tl.parent_timeline_id.is_none());
        assert!(tl.fork_point.is_none());
        assert!(tl.broken_at.is_none());
    }

    #[test]
    fn ensure_main_timeline_is_idempotent() {
        let store = test_store();
        store.ensure_main_timeline().unwrap();
        store.ensure_main_timeline().unwrap();

        let tl = store.get_timeline(1).unwrap().unwrap();
        assert_eq!(tl.branch_name, "main");
    }

    #[test]
    fn create_timeline_returns_auto_id() {
        let store = test_store();
        store.ensure_main_timeline().unwrap();

        let id = store.create_timeline("repair-2026-01-01-1", Some(1), Some("abc123")).unwrap();
        assert!(id > 1);

        let tl = store.get_timeline(id).unwrap().unwrap();
        assert_eq!(tl.branch_name, "repair-2026-01-01-1");
        assert_eq!(tl.parent_timeline_id, Some(1));
        assert_eq!(tl.fork_point, Some("abc123".to_string()));
        assert!(tl.broken_at.is_none());
    }

    #[test]
    fn get_timeline_by_branch() {
        let store = test_store();
        store.ensure_main_timeline().unwrap();

        let tl = store.get_timeline_by_branch("main").unwrap().unwrap();
        assert_eq!(tl.id, 1);

        assert!(store.get_timeline_by_branch("nonexistent").unwrap().is_none());
    }

    #[test]
    fn seal_timeline_sets_broken_at() {
        let store = test_store();
        store.ensure_main_timeline().unwrap();

        assert!(!store.is_timeline_sealed(1).unwrap());

        store.seal_timeline(1).unwrap();
        assert!(store.is_timeline_sealed(1).unwrap());

        let tl = store.get_timeline(1).unwrap().unwrap();
        assert!(tl.broken_at.is_some());
    }

    #[test]
    fn rename_timeline_updates_branch_name() {
        let store = test_store();
        store.ensure_main_timeline().unwrap();

        store.rename_timeline(1, "broken-main-2026-01-01").unwrap();

        assert!(store.get_timeline_by_branch("main").unwrap().is_none());
        let tl = store.get_timeline_by_branch("broken-main-2026-01-01").unwrap().unwrap();
        assert_eq!(tl.id, 1);
    }

    #[test]
    fn is_timeline_sealed_returns_false_for_nonexistent() {
        let store = test_store();
        // Non-existent timeline → not sealed (no row → broken_at is None)
        assert!(!store.is_timeline_sealed(999).unwrap());
    }

    #[test]
    fn unknown_message_type_returns_error() {
        let store = test_store();
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO dag_nodes (hash, parent_hash, message_type, sender, receiver, session_id, submission_id, payload, diagnostics, stderr, created_at, state, protocol_version)
             VALUES ('h1', '', 'bogus', 'cli', 'agent-a', 'sess-1', 'sub-1', x'', x'', x'', '2025-01-01T00:00:00Z', NULL, '')",
            [],
        ).unwrap();
        drop(conn);

        let result = store.get_node("h1");
        assert!(result.is_err(), "unknown message type should error, not silently default");
        let err = result.unwrap_err();
        assert!(err.contains("unknown message type: bogus"), "error should name the bad value, got: {}", err);
    }
}
