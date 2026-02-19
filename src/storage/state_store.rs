//! SqliteStateStore — SQLite-backed persistence for versioned agent state (ADR 055).
//!
//! Domain types (`StateCommit`, `StateStore` trait, `hash_value`, `hash_snapshot`,
//! `hash_state_commit`) live in `crate::domain`. This module provides the SQLite
//! implementation.
//!
//! Three tables mirror git's object model:
//! - `state_values`: Content blobs keyed by SHA-256 hash
//! - `state_snapshots`: Path → value_hash mappings (like git trees)
//! - `state_commits`: Snapshot + parent pointer (like git commits)

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::domain::{StateCommit, sorted_entries_json};

/// Content-addressed append-only SQLite store for versioned agent state.
pub struct SqliteStateStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStateStore {
    /// Open (or create) a state store at the given path.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create state store directory: {}", e))?;
        }

        let conn = Connection::open(path)
            .map_err(|e| format!("failed to open state store: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS state_values (
                 hash TEXT PRIMARY KEY,
                 content BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS state_snapshots (
                 hash TEXT PRIMARY KEY,
                 entries TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS state_commits (
                 hash TEXT PRIMARY KEY,
                 snapshot_hash TEXT NOT NULL,
                 parent_hash TEXT NOT NULL
             );"
        ).map_err(|e| format!("failed to initialize state store: {}", e))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

}

impl crate::domain::StateStore for SqliteStateStore {
    fn put_value(&self, hash: &str, content: &[u8]) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO state_values (hash, content) VALUES (?1, ?2)",
            rusqlite::params![hash, content],
        ).map_err(|e| format!("put_value failed: {}", e))?;
        Ok(())
    }

    fn get_value(&self, hash: &str) -> Result<Option<Vec<u8>>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT content FROM state_values WHERE hash = ?1")
            .map_err(|e| format!("get_value prepare failed: {}", e))?;
        let result = stmt.query_row(rusqlite::params![hash], |row| row.get(0))
            .optional()
            .map_err(|e| format!("get_value query failed: {}", e))?;
        Ok(result)
    }

    fn put_snapshot(&self, hash: &str, entries: &HashMap<String, String>) -> Result<(), String> {
        let json = sorted_entries_json(entries);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO state_snapshots (hash, entries) VALUES (?1, ?2)",
            rusqlite::params![hash, json],
        ).map_err(|e| format!("put_snapshot failed: {}", e))?;
        Ok(())
    }

    fn get_snapshot(&self, hash: &str) -> Result<Option<HashMap<String, String>>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT entries FROM state_snapshots WHERE hash = ?1")
            .map_err(|e| format!("get_snapshot prepare failed: {}", e))?;
        let result: Option<String> = stmt.query_row(rusqlite::params![hash], |row| row.get(0))
            .optional()
            .map_err(|e| format!("get_snapshot query failed: {}", e))?;

        match result {
            Some(json) => {
                let entries: HashMap<String, String> = serde_json::from_str(&json)
                    .map_err(|e| format!("get_snapshot parse failed: {}", e))?;
                Ok(Some(entries))
            }
            None => Ok(None),
        }
    }

    fn put_state_commit(
        &self,
        hash: &str,
        snapshot_hash: &str,
        parent_hash: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO state_commits (hash, snapshot_hash, parent_hash) VALUES (?1, ?2, ?3)",
            rusqlite::params![hash, snapshot_hash, parent_hash],
        ).map_err(|e| format!("put_state_commit failed: {}", e))?;
        Ok(())
    }

    fn get_state_commit(&self, hash: &str) -> Result<Option<StateCommit>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT hash, snapshot_hash, parent_hash FROM state_commits WHERE hash = ?1"
        ).map_err(|e| format!("get_state_commit prepare failed: {}", e))?;

        let result = stmt.query_row(rusqlite::params![hash], |row| {
            Ok(StateCommit {
                hash: row.get(0)?,
                snapshot_hash: row.get(1)?,
                parent_hash: row.get(2)?,
            })
        })
        .optional()
        .map_err(|e| format!("get_state_commit query failed: {}", e))?;

        Ok(result)
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
    use crate::domain::{hash_value, hash_snapshot, hash_state_commit, StateStore};

    fn test_store() -> SqliteStateStore {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        SqliteStateStore::open(tmp.path()).unwrap()
    }

    #[test]
    fn put_get_value_round_trip() {
        let store = test_store();
        let content = b"hello world";
        let hash = hash_value(content);

        store.put_value(&hash, content).unwrap();
        let retrieved = store.get_value(&hash).unwrap();

        assert_eq!(retrieved, Some(content.to_vec()));
    }

    #[test]
    fn get_value_returns_none_for_unknown() {
        let store = test_store();
        assert_eq!(store.get_value("nonexistent").unwrap(), None);
    }

    #[test]
    fn put_get_snapshot_round_trip() {
        let store = test_store();
        let mut entries = HashMap::new();
        entries.insert("/todos.json".to_string(), "abc123".to_string());
        entries.insert("/config.json".to_string(), "def456".to_string());

        let hash = hash_snapshot(&entries);
        store.put_snapshot(&hash, &entries).unwrap();

        let retrieved = store.get_snapshot(&hash).unwrap().unwrap();
        assert_eq!(retrieved, entries);
    }

    #[test]
    fn get_snapshot_returns_none_for_unknown() {
        let store = test_store();
        assert_eq!(store.get_snapshot("nonexistent").unwrap(), None);
    }

    #[test]
    fn put_get_state_commit_round_trip() {
        let store = test_store();
        let snapshot_hash = "snap123";
        let parent_hash = "";
        let hash = hash_state_commit(snapshot_hash, parent_hash);

        store.put_state_commit(&hash, snapshot_hash, parent_hash).unwrap();

        let commit = store.get_state_commit(&hash).unwrap().unwrap();
        assert_eq!(commit.snapshot_hash, snapshot_hash);
        assert_eq!(commit.parent_hash, parent_hash);
    }

    #[test]
    fn get_state_commit_returns_none_for_unknown() {
        let store = test_store();
        assert_eq!(store.get_state_commit("nonexistent").unwrap(), None);
    }

    #[test]
    fn duplicate_put_is_idempotent() {
        let store = test_store();
        let content = b"same content";
        let hash = hash_value(content);

        store.put_value(&hash, content).unwrap();
        store.put_value(&hash, content).unwrap(); // No error

        let retrieved = store.get_value(&hash).unwrap();
        assert_eq!(retrieved, Some(content.to_vec()));
    }
}
