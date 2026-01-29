//! Vector storage (embeddings)
//!
//! SQLite implementation of the VectorStorage trait using sqlite-vec
//! for efficient similarity search. The trait is defined in the domain module.

use crate::config;
use crate::domain::VectorStorage;
use rusqlite::{params, Connection};
use sqlite_vec::sqlite3_vec_init;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zerocopy::AsBytes;

// ============================================================================
// SQLite Implementation
// ============================================================================

/// SQLite-backed vector storage using sqlite-vec.
pub struct SqliteVectorStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteVectorStorage {
    /// Open vector storage for an agent
    pub fn open(agent_name: &str) -> Result<Self, String> {
        let db_path = config::agent_db_path(agent_name);

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create agent directory: {}", e))?;
        }

        // Register sqlite-vec extension
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite3_vec_init as *const (),
            )));
        }

        let conn = Connection::open(&db_path)
            .map_err(|e| format!("failed to open database: {}", e))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("failed to set WAL mode: {}", e))?;

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_items USING vec0(
                key TEXT PRIMARY KEY,
                embedding float[768],
                metadata TEXT
            )",
            [],
        ).map_err(|e| format!("failed to create vec_items table: {}", e))?;

        Ok(SqliteVectorStorage {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl VectorStorage for SqliteVectorStorage {
    fn store_embedding(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String> {
        if vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", vector.len()));
        }

        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // sqlite-vec virtual tables don't support INSERT OR REPLACE
        // Delete first if exists, then insert
        let _ = conn.execute("DELETE FROM vec_items WHERE key = ?", params![key]);

        conn.execute(
            "INSERT INTO vec_items (key, embedding, metadata) VALUES (?, ?, ?)",
            params![key, vector.as_bytes(), metadata],
        ).map_err(|e| format!("failed to store embedding: {}", e))?;
        Ok(())
    }

    fn search_by_vector(&self, query_vector: &[f32], limit: u32) -> Result<Vec<(String, String, f64)>, String> {
        if query_vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", query_vector.len()));
        }

        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT key, metadata, distance
             FROM vec_items
             WHERE embedding MATCH ?
             ORDER BY distance
             LIMIT ?"
        ).map_err(|e| format!("failed to prepare search query: {}", e))?;

        let rows = stmt.query_map(params![query_vector.as_bytes(), limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        }).map_err(|e| format!("failed to search: {}", e))?;

        let mut results = Vec::new();
        for result in rows {
            results.push(result.map_err(|e| format!("failed to get result: {}", e))?);
        }
        Ok(results)
    }

    fn delete_embedding(&self, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let rows_affected = conn.execute("DELETE FROM vec_items WHERE key = ?", params![key])
            .map_err(|e| format!("failed to delete embedding: {}", e))?;
        Ok(rows_affected > 0)
    }
}

// ============================================================================
// In-Memory Implementation
// ============================================================================

/// In-memory vector storage. Useful for testing.
/// Uses brute-force euclidean distance for similarity search.
pub struct InMemoryVectorStorage {
    items: Mutex<HashMap<String, (Vec<f32>, String)>>,
}

impl InMemoryVectorStorage {
    pub fn new() -> Self {
        InMemoryVectorStorage {
            items: Mutex::new(HashMap::new()),
        }
    }
}

impl VectorStorage for InMemoryVectorStorage {
    fn store_embedding(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String> {
        if vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", vector.len()));
        }

        let mut items = self.items.lock().map_err(|e| e.to_string())?;
        items.insert(key.to_string(), (vector.to_vec(), metadata.to_string()));
        Ok(())
    }

    fn search_by_vector(&self, query_vector: &[f32], limit: u32) -> Result<Vec<(String, String, f64)>, String> {
        if query_vector.len() != 768 {
            return Err(format!("expected 768 dimensions, got {}", query_vector.len()));
        }

        let items = self.items.lock().map_err(|e| e.to_string())?;

        // Calculate distances and sort
        let mut results: Vec<(String, String, f64)> = items.iter()
            .map(|(key, (vector, metadata))| {
                let distance = euclidean_distance(query_vector, vector);
                (key.clone(), metadata.clone(), distance)
            })
            .collect();

        results.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit as usize);

        Ok(results)
    }

    fn delete_embedding(&self, key: &str) -> Result<bool, String> {
        let mut items = self.items.lock().map_err(|e| e.to_string())?;
        Ok(items.remove(key).is_some())
    }
}

fn euclidean_distance(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2) as f64)
        .sum::<f64>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::VectorStorage;

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
}
