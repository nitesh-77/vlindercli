use vlindercli::storage::VectorStorage;

/// Create a 768-dim test vector
fn make_test_vector(seed: f32) -> Vec<f32> {
    (0..768).map(|i| (i as f32 * seed).sin()).collect()
}

#[test]
fn store_and_search_embedding() {
    let storage = VectorStorage::open("test-agent-vec").unwrap();

    // Store some embeddings
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

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-vec");
}

#[test]
fn embedding_wrong_dimensions() {
    let storage = VectorStorage::open("test-agent-vec-dim").unwrap();

    // Try to store wrong dimension vector
    let wrong_vec: Vec<f32> = vec![1.0, 2.0, 3.0]; // Only 3 dims
    let result = storage.store_embedding("bad", &wrong_vec, "");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("768"));

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-vec-dim");
}

#[test]
fn delete_embedding() {
    let storage = VectorStorage::open("test-agent-vec-del").unwrap();

    let vec1 = make_test_vector(1.0);
    storage.store_embedding("to-delete", &vec1, "temp").unwrap();

    // Verify it exists via search
    let results = storage.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results.len(), 1);

    // Delete it
    let deleted = storage.delete_embedding("to-delete").unwrap();
    assert!(deleted);

    // Verify it's gone
    let results = storage.search_by_vector(&vec1, 1).unwrap();
    assert_eq!(results.len(), 0);

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-vec-del");
}

#[test]
fn overwrite_embedding() {
    let storage = VectorStorage::open("test-agent-vec-overwrite").unwrap();

    let vec1 = make_test_vector(1.0);
    let vec2 = make_test_vector(2.0);

    // Store initial
    storage.store_embedding("doc", &vec1, "original").unwrap();

    // Overwrite with different vector and metadata
    storage.store_embedding("doc", &vec2, "updated").unwrap();

    // Search with vec2 should find it
    let results = storage.search_by_vector(&vec2, 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "doc");
    assert_eq!(results[0].1, "updated");

    // Cleanup
    drop(storage);
    let _ = std::fs::remove_dir_all(".vlinder/agents/test-agent-vec-overwrite");
}
