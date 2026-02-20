//! State domain types — content-addressed versioned state (ADR 055).
//!
//! Three concepts mirror git's object model:
//! - Values: Content blobs keyed by SHA-256 hash
//! - Snapshots: Path → value_hash mappings (like git trees)
//! - State commits: Snapshot + parent pointer (like git commits)
//!
//! Hash computation:
//! - Value: SHA256(content)
//! - Snapshot: SHA256(sorted JSON of entries)
//! - State commit: SHA256(snapshot_hash + ":" + parent_hash)
//!
//! Root state: empty string "" — the parent of the first commit.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

/// A state commit links a snapshot to its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct StateCommit {
    pub hash: String,
    pub snapshot_hash: String,
    pub parent_hash: String,
}

// ============================================================================
// StateStore trait
// ============================================================================

/// Persistence layer for versioned agent state (ADR 055).
///
/// Mirrors git's object model: values (blobs), snapshots (trees),
/// and state commits (commit objects with parent pointers).
///
/// Implementations must be thread-safe. All writes are idempotent
/// (content-addressed, insert-or-ignore).
pub trait StateStore: Send + Sync {
    /// Store a value blob. Idempotent (content-addressed).
    fn put_value(&self, hash: &str, content: &[u8]) -> Result<(), String>;

    /// Retrieve a value blob by hash.
    fn get_value(&self, hash: &str) -> Result<Option<Vec<u8>>, String>;

    /// Store a snapshot (set of path → value_hash entries). Idempotent.
    fn put_snapshot(&self, hash: &str, entries: &HashMap<String, String>) -> Result<(), String>;

    /// Retrieve a snapshot's entries by hash.
    fn get_snapshot(&self, hash: &str) -> Result<Option<HashMap<String, String>>, String>;

    /// Store a state commit. Idempotent.
    fn put_state_commit(&self, hash: &str, snapshot_hash: &str, parent_hash: &str) -> Result<(), String>;

    /// Retrieve a state commit by hash.
    fn get_state_commit(&self, hash: &str) -> Result<Option<StateCommit>, String>;
}

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
pub fn sorted_entries_json(entries: &HashMap<String, String>) -> String {
    let mut sorted: Vec<(&String, &String)> = entries.iter().collect();
    sorted.sort_by_key(|(k, _)| k.as_str());
    let map: serde_json::Map<String, serde_json::Value> = sorted.into_iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
}

// ============================================================================
// In-Memory test double
// ============================================================================

#[cfg(test)]
use std::sync::RwLock;

/// Ephemeral in-memory state store for tests.
#[cfg(test)]
pub struct InMemoryStateStore {
    values: RwLock<HashMap<String, Vec<u8>>>,
    snapshots: RwLock<HashMap<String, HashMap<String, String>>>,
    commits: RwLock<HashMap<String, StateCommit>>,
}

#[cfg(test)]
impl InMemoryStateStore {
    pub fn new() -> Self {
        Self {
            values: RwLock::new(HashMap::new()),
            snapshots: RwLock::new(HashMap::new()),
            commits: RwLock::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
impl StateStore for InMemoryStateStore {
    fn put_value(&self, hash: &str, content: &[u8]) -> Result<(), String> {
        self.values.write().unwrap().entry(hash.to_string()).or_insert_with(|| content.to_vec());
        Ok(())
    }

    fn get_value(&self, hash: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.values.read().unwrap().get(hash).cloned())
    }

    fn put_snapshot(&self, hash: &str, entries: &HashMap<String, String>) -> Result<(), String> {
        self.snapshots.write().unwrap().entry(hash.to_string()).or_insert_with(|| entries.clone());
        Ok(())
    }

    fn get_snapshot(&self, hash: &str) -> Result<Option<HashMap<String, String>>, String> {
        Ok(self.snapshots.read().unwrap().get(hash).cloned())
    }

    fn put_state_commit(&self, hash: &str, snapshot_hash: &str, parent_hash: &str) -> Result<(), String> {
        self.commits.write().unwrap().entry(hash.to_string()).or_insert_with(|| StateCommit {
            hash: hash.to_string(),
            snapshot_hash: snapshot_hash.to_string(),
            parent_hash: parent_hash.to_string(),
        });
        Ok(())
    }

    fn get_state_commit(&self, hash: &str) -> Result<Option<StateCommit>, String> {
        Ok(self.commits.read().unwrap().get(hash).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
