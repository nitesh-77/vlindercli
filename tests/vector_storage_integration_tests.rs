//! Integration tests for SqliteVectorStorage.
//! Tests SQLite-specific behavior: persistence, sqlite-vec functionality.

use vlindercli::config::agent_dir;
use vlindercli::storage::{VectorStorage, SqliteVectorStorage};

/// Create a 768-dim test vector
fn make_test_vector(seed: f32) -> Vec<f32> {
    (0..768).map(|i| (i as f32 * seed).sin()).collect()
}

#[test]
fn sqlite_store_and_search() {
    let storage = SqliteVectorStorage::open("test-agent-vec").unwrap();

    let vec1 = make_test_vector(1.0);
    let vec2 = make_test_vector(2.0);
    let vec3 = make_test_vector(1.1); // Similar to vec1

    storage.store_embedding("doc1", &vec1, "first document").unwrap();
    storage.store_embedding("doc2", &vec2, "second document").unwrap();
    storage.store_embedding("doc3", &vec3, "third document").unwrap();

    // Search with vec1 - should find doc1 first (exact match), then doc3 (similar)
    let results = storage.search_by_vector(&vec1, 3).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].0, "doc1"); // Exact match first
    assert_eq!(results[0].1, "first document");

    drop(storage);
    let _ = std::fs::remove_dir_all(agent_dir("test-agent-vec"));
}

/// Verify data persists across close/reopen
#[test]
fn sqlite_persistence() {
    let agent_name = "test-agent-vec-persist";
    let vec1 = make_test_vector(1.0);

    // Store embedding
    {
        let storage = SqliteVectorStorage::open(agent_name).unwrap();
        storage.store_embedding("persistent", &vec1, "survives restart").unwrap();
    } // storage dropped, connection closed

    // Reopen and verify data persists
    {
        let storage = SqliteVectorStorage::open(agent_name).unwrap();
        let results = storage.search_by_vector(&vec1, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "persistent");
        assert_eq!(results[0].1, "survives restart");
    }

    let _ = std::fs::remove_dir_all(agent_dir(agent_name));
}

#[test]
fn sqlite_overwrite() {
    let storage = SqliteVectorStorage::open("test-agent-vec-overwrite").unwrap();

    let vec1 = make_test_vector(1.0);
    let vec2 = make_test_vector(2.0);

    storage.store_embedding("doc", &vec1, "original").unwrap();
    storage.store_embedding("doc", &vec2, "updated").unwrap();

    // Search with vec2 should find it
    let results = storage.search_by_vector(&vec2, 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "doc");
    assert_eq!(results[0].1, "updated");

    drop(storage);
    let _ = std::fs::remove_dir_all(agent_dir("test-agent-vec-overwrite"));
}

#[test]
fn sqlite_delete() {
    let storage = SqliteVectorStorage::open("test-agent-vec-del").unwrap();

    let vec1 = make_test_vector(1.0);
    storage.store_embedding("to-delete", &vec1, "temp").unwrap();

    let results = storage.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results.len(), 1);

    let deleted = storage.delete_embedding("to-delete").unwrap();
    assert!(deleted);

    let results = storage.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results.len(), 0);

    drop(storage);
    let _ = std::fs::remove_dir_all(agent_dir("test-agent-vec-del"));
}

/// Security test: Each agent's vector storage is isolated
#[test]
fn sqlite_agent_isolation() {
    let storage_a = SqliteVectorStorage::open("test-agent-vec-isolated-a").unwrap();
    let storage_b = SqliteVectorStorage::open("test-agent-vec-isolated-b").unwrap();

    let vec1 = make_test_vector(1.0);

    // Agent A stores an embedding
    storage_a.store_embedding("secret", &vec1, "agent-a-data").unwrap();

    // Agent B cannot find Agent A's embedding
    let results_b = storage_b.search_by_vector(&vec1, 10).unwrap();
    assert!(results_b.is_empty());

    // Agent B stores its own embedding with same key
    storage_b.store_embedding("secret", &vec1, "agent-b-data").unwrap();

    // Each agent sees only their own data
    let results_a = storage_a.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results_a[0].1, "agent-a-data");

    let results_b = storage_b.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results_b[0].1, "agent-b-data");

    drop(storage_a);
    drop(storage_b);
    let _ = std::fs::remove_dir_all(agent_dir("test-agent-vec-isolated-a"));
    let _ = std::fs::remove_dir_all(agent_dir("test-agent-vec-isolated-b"));
}
