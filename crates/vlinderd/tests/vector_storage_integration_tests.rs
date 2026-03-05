#![cfg(feature = "sqlite-vec")]
//! Integration tests for SqliteVectorStorage.
//! Tests SQLite-specific behavior: persistence, sqlite-vec functionality.

use tempfile::TempDir;
use vlinder_sqlite_vec::SqliteVectorStorage;

/// Create a 768-dim test vector
fn make_test_vector(seed: f32) -> Vec<f32> {
    (0..768).map(|i| (i as f32 * seed).sin()).collect()
}

#[test]
fn sqlite_store_and_search() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("agent.db");
    let storage = SqliteVectorStorage::open_at(&db_path).unwrap();

    let vec1 = make_test_vector(1.0);
    let vec2 = make_test_vector(2.0);
    let vec3 = make_test_vector(1.1); // Similar to vec1

    storage
        .store_embedding("doc1", &vec1, "first document")
        .unwrap();
    storage
        .store_embedding("doc2", &vec2, "second document")
        .unwrap();
    storage
        .store_embedding("doc3", &vec3, "third document")
        .unwrap();

    // Search with vec1 - should find doc1 first (exact match), then doc3 (similar)
    let results = storage.search_by_vector(&vec1, 3).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].0, "doc1"); // Exact match first
    assert_eq!(results[0].1, "first document");
}

/// Verify data persists across close/reopen
#[test]
fn sqlite_persistence() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("agent.db");
    let vec1 = make_test_vector(1.0);

    // Store embedding
    {
        let storage = SqliteVectorStorage::open_at(&db_path).unwrap();
        storage
            .store_embedding("persistent", &vec1, "survives restart")
            .unwrap();
    } // storage dropped, connection closed

    // Reopen and verify data persists
    {
        let storage = SqliteVectorStorage::open_at(&db_path).unwrap();
        let results = storage.search_by_vector(&vec1, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "persistent");
        assert_eq!(results[0].1, "survives restart");
    }
}

#[test]
fn sqlite_overwrite() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("agent.db");
    let storage = SqliteVectorStorage::open_at(&db_path).unwrap();

    let vec1 = make_test_vector(1.0);
    let vec2 = make_test_vector(2.0);

    storage.store_embedding("doc", &vec1, "original").unwrap();
    storage.store_embedding("doc", &vec2, "updated").unwrap();

    // Search with vec2 should find it
    let results = storage.search_by_vector(&vec2, 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "doc");
    assert_eq!(results[0].1, "updated");
}

#[test]
fn sqlite_delete() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("agent.db");
    let storage = SqliteVectorStorage::open_at(&db_path).unwrap();

    let vec1 = make_test_vector(1.0);
    storage.store_embedding("to-delete", &vec1, "temp").unwrap();

    let results = storage.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results.len(), 1);

    let deleted = storage.delete_embedding("to-delete").unwrap();
    assert!(deleted);

    let results = storage.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results.len(), 0);
}

/// Security test: Each agent's vector storage is isolated
#[test]
fn sqlite_agent_isolation() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    let storage_a = SqliteVectorStorage::open_at(&dir_a.path().join("agent.db")).unwrap();
    let storage_b = SqliteVectorStorage::open_at(&dir_b.path().join("agent.db")).unwrap();

    let vec1 = make_test_vector(1.0);

    // Agent A stores an embedding
    storage_a
        .store_embedding("secret", &vec1, "agent-a-data")
        .unwrap();

    // Agent B cannot find Agent A's embedding
    let results_b = storage_b.search_by_vector(&vec1, 10).unwrap();
    assert!(results_b.is_empty());

    // Agent B stores its own embedding with same key
    storage_b
        .store_embedding("secret", &vec1, "agent-b-data")
        .unwrap();

    // Each agent sees only their own data
    let results_a = storage_a.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results_a[0].1, "agent-a-data");

    let results_b = storage_b.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results_b[0].1, "agent-b-data");
}
