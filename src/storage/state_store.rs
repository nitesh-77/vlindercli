//! StateStore — content-addressed SQLite storage for versioned agent state (ADR 055).
//!
//! Three tables mirror git's object model:
//! - `state_values`: Content blobs keyed by SHA-256 hash
//! - `state_snapshots`: Path → value_hash mappings (like git trees)
//! - `state_commits`: Snapshot + parent pointer (like git commits)
//!
//! Hash computation:
//! - Value: SHA256(content)
//! - Snapshot: SHA256(sorted JSON of entries)
//! - State commit: SHA256(snapshot_hash + ":" + parent_hash)
//!
//! Root state: empty string "" — the parent of the first commit.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// A state commit links a snapshot to its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct StateCommit {
    pub hash: String,
    pub snapshot_hash: String,
    pub parent_hash: String,
}

/// Content-addressed append-only SQLite store for versioned agent state.
pub struct StateStore {
    conn: Arc<Mutex<Connection>>,
}

impl StateStore {
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

    // --- Values (blobs) ---

    /// Store a value blob. Idempotent (content-addressed).
    pub fn put_value(&self, hash: &str, content: &[u8]) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO state_values (hash, content) VALUES (?1, ?2)",
            rusqlite::params![hash, content],
        ).map_err(|e| format!("put_value failed: {}", e))?;
        Ok(())
    }

    /// Retrieve a value blob by hash.
    pub fn get_value(&self, hash: &str) -> Result<Option<Vec<u8>>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT content FROM state_values WHERE hash = ?1")
            .map_err(|e| format!("get_value prepare failed: {}", e))?;
        let result = stmt.query_row(rusqlite::params![hash], |row| row.get(0))
            .optional()
            .map_err(|e| format!("get_value query failed: {}", e))?;
        Ok(result)
    }

    // --- Snapshots (path → value_hash mappings) ---

    /// Store a snapshot (set of path → value_hash entries). Idempotent.
    pub fn put_snapshot(&self, hash: &str, entries: &HashMap<String, String>) -> Result<(), String> {
        let json = sorted_entries_json(entries);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO state_snapshots (hash, entries) VALUES (?1, ?2)",
            rusqlite::params![hash, json],
        ).map_err(|e| format!("put_snapshot failed: {}", e))?;
        Ok(())
    }

    /// Retrieve a snapshot's entries by hash.
    pub fn get_snapshot(&self, hash: &str) -> Result<Option<HashMap<String, String>>, String> {
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

    // --- State commits (snapshot + parent) ---

    /// Store a state commit. Idempotent.
    pub fn put_state_commit(
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

    /// Retrieve a state commit by hash.
    pub fn get_state_commit(&self, hash: &str) -> Result<Option<StateCommit>, String> {
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

// --- Hash computation ---

/// Compute SHA-256 hash of content bytes.
pub fn hash_value(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

/// Compute SHA-256 hash of a snapshot (sorted JSON of entries).
pub fn hash_snapshot(entries: &HashMap<String, String>) -> String {
    let json = sorted_entries_json(entries);
    hash_value(json.as_bytes())
}

/// Compute SHA-256 hash of a state commit (snapshot_hash + ":" + parent_hash).
pub fn hash_state_commit(snapshot_hash: &str, parent_hash: &str) -> String {
    let input = format!("{}:{}", snapshot_hash, parent_hash);
    hash_value(input.as_bytes())
}

/// Produce deterministic JSON from entries by sorting keys.
fn sorted_entries_json(entries: &HashMap<String, String>) -> String {
    let mut sorted: Vec<(&String, &String)> = entries.iter().collect();
    sorted.sort_by_key(|(k, _)| k.as_str());
    let map: serde_json::Map<String, serde_json::Value> = sorted.into_iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
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

    fn test_store() -> StateStore {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        StateStore::open(tmp.path()).unwrap()
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

    #[test]
    fn snapshot_hash_is_deterministic() {
        let mut entries1 = HashMap::new();
        entries1.insert("b".to_string(), "2".to_string());
        entries1.insert("a".to_string(), "1".to_string());

        let mut entries2 = HashMap::new();
        entries2.insert("a".to_string(), "1".to_string());
        entries2.insert("b".to_string(), "2".to_string());

        assert_eq!(hash_snapshot(&entries1), hash_snapshot(&entries2));
    }

    #[test]
    fn state_commit_hash_depends_on_parent() {
        let snapshot = "snap123";
        let hash1 = hash_state_commit(snapshot, "");
        let hash2 = hash_state_commit(snapshot, "parent1");

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn value_hash_is_sha256() {
        // Known SHA-256 for "hello world"
        let hash = hash_value(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
