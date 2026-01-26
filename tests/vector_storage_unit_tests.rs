//! Unit tests for VectorStorage trait behavior.
//! Uses in-memory implementation for speed.

use vlindercli::storage::{VectorStorage, InMemoryVectorStorage};

/// Create a 768-dim test vector
fn make_test_vector(seed: f32) -> Vec<f32> {
    (0..768).map(|i| (i as f32 * seed).sin()).collect()
}

#[test]
fn store_and_search_embedding() {
    let storage = InMemoryVectorStorage::new();

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
    assert!(results[0].2 < 0.0001); // Distance should be ~0 for exact match
}

#[test]
fn embedding_wrong_dimensions() {
    let storage = InMemoryVectorStorage::new();

    let wrong_vec: Vec<f32> = vec![1.0, 2.0, 3.0]; // Only 3 dims
    let result = storage.store_embedding("bad", &wrong_vec, "");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("768"));

    // Also test search with wrong dimensions
    let search_result = storage.search_by_vector(&wrong_vec, 10);
    assert!(search_result.is_err());
}

#[test]
fn delete_embedding() {
    let storage = InMemoryVectorStorage::new();

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

    // Delete non-existent returns false
    let deleted_again = storage.delete_embedding("to-delete").unwrap();
    assert!(!deleted_again);
}

#[test]
fn overwrite_embedding() {
    let storage = InMemoryVectorStorage::new();

    let vec1 = make_test_vector(1.0);
    let vec2 = make_test_vector(2.0);

    // Store initial
    storage.store_embedding("doc", &vec1, "original").unwrap();

    // Overwrite with different vector and metadata
    storage.store_embedding("doc", &vec2, "updated").unwrap();

    // Search with vec2 should find it with distance ~0
    let results = storage.search_by_vector(&vec2, 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "doc");
    assert_eq!(results[0].1, "updated");
    assert!(results[0].2 < 0.0001);
}

#[test]
fn search_empty_storage() {
    let storage = InMemoryVectorStorage::new();

    let vec1 = make_test_vector(1.0);
    let results = storage.search_by_vector(&vec1, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_respects_limit() {
    let storage = InMemoryVectorStorage::new();

    // Store 5 embeddings
    for i in 0..5 {
        let vec = make_test_vector(i as f32);
        storage.store_embedding(&format!("doc{}", i), &vec, "").unwrap();
    }

    let query = make_test_vector(0.0);

    // Limit 2 should return only 2
    let results = storage.search_by_vector(&query, 2).unwrap();
    assert_eq!(results.len(), 2);

    // Limit 10 should return all 5
    let results = storage.search_by_vector(&query, 10).unwrap();
    assert_eq!(results.len(), 5);
}

#[test]
fn search_orders_by_distance() {
    let storage = InMemoryVectorStorage::new();

    // Create base vector of all 1.0s
    let base: Vec<f32> = vec![1.0; 768];

    // Create vectors with known distances from base
    let close: Vec<f32> = vec![1.1; 768];   // Small offset
    let medium: Vec<f32> = vec![2.0; 768];  // Larger offset
    let far: Vec<f32> = vec![10.0; 768];    // Much larger offset

    storage.store_embedding("far", &far, "").unwrap();
    storage.store_embedding("close", &close, "").unwrap();
    storage.store_embedding("medium", &medium, "").unwrap();

    // Search with base vector - should order by distance
    let results = storage.search_by_vector(&base, 3).unwrap();
    assert_eq!(results[0].0, "close");
    assert_eq!(results[1].0, "medium");
    assert_eq!(results[2].0, "far");

    // Verify distances are actually increasing
    assert!(results[0].2 < results[1].2);
    assert!(results[1].2 < results[2].2);
}

#[test]
fn unicode_metadata() {
    let storage = InMemoryVectorStorage::new();

    let vec1 = make_test_vector(1.0);

    // Store with unicode metadata
    storage.store_embedding("doc1", &vec1, "文档摘要：这是一个测试").unwrap();
    storage.store_embedding("doc2", &make_test_vector(2.0), "🎉 Party notes 🎊").unwrap();

    let results = storage.search_by_vector(&vec1, 2).unwrap();
    assert_eq!(results[0].1, "文档摘要：这是一个测试");
}

#[test]
fn search_limit_zero() {
    let storage = InMemoryVectorStorage::new();

    let vec1 = make_test_vector(1.0);
    storage.store_embedding("doc1", &vec1, "test").unwrap();

    // Limit 0 should return empty
    let results = storage.search_by_vector(&vec1, 0).unwrap();
    assert!(results.is_empty());
}
