//! Storage domain types and traits.
//!
//! Storage provides object (file) and vector (embedding) persistence.
//! This module defines both the configuration types (WHAT storage to use)
//! and the capability traits (the abstract interface).

use std::path::PathBuf;

use serde::Deserialize;

// ============================================================================
// ObjectStorageType (available implementations)
// ============================================================================

/// Available object storage implementations.
///
/// Registered with the Registry to track what backends are available.
/// Follows the same pattern as `RuntimeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectStorageType {
    /// SQLite-backed storage.
    Sqlite,
    /// In-memory storage (for testing).
    InMemory,
}

impl ObjectStorageType {
    /// String representation for queue routing.
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectStorageType::Sqlite => "sqlite",
            ObjectStorageType::InMemory => "memory",
        }
    }

    /// Determine storage type from URI scheme.
    pub fn from_scheme(scheme: Option<&str>) -> Option<Self> {
        match scheme {
            Some("sqlite") => Some(ObjectStorageType::Sqlite),
            Some("memory") => Some(ObjectStorageType::InMemory),
            _ => None,
        }
    }
}

// ============================================================================
// VectorStorageType (available implementations)
// ============================================================================

/// Available vector storage implementations.
///
/// Registered with the Registry to track what backends are available.
/// Follows the same pattern as `RuntimeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VectorStorageType {
    /// SQLite-backed storage with sqlite-vec extension.
    SqliteVec,
    /// In-memory storage (for testing).
    InMemory,
}

impl VectorStorageType {
    /// String representation for queue routing.
    pub fn as_str(&self) -> &'static str {
        match self {
            VectorStorageType::SqliteVec => "sqlite-vec",
            VectorStorageType::InMemory => "memory",
        }
    }

    /// Determine storage type from URI scheme.
    pub fn from_scheme(scheme: Option<&str>) -> Option<Self> {
        match scheme {
            Some("sqlite") => Some(VectorStorageType::SqliteVec),
            Some("memory") => Some(VectorStorageType::InMemory),
            _ => None,
        }
    }
}

// ============================================================================
// ObjectStorage Trait
// ============================================================================

/// Object storage for file operations.
///
/// Implementations provide a virtual filesystem for agents. Paths are keys,
/// not real filesystem paths - this isolates agents from the host system.
pub trait ObjectStorage: Send + Sync {
    /// Store a file at the given path.
    fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String>;

    /// Retrieve a file from the given path. Returns None if not found.
    fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String>;

    /// Delete a file at the given path. Returns true if the file existed.
    fn delete_file(&self, path: &str) -> Result<bool, String>;

    /// List all files under the given directory path.
    fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String>;
}

// ============================================================================
// VectorStorage Trait
// ============================================================================

/// Vector storage for embedding operations.
///
/// Implementations provide similarity search over embedded vectors.
/// Each embedding has a key, a 768-dimensional vector, and metadata.
pub trait VectorStorage: Send + Sync {
    /// Store an embedding with the given key.
    fn store_embedding(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String>;

    /// Search for similar embeddings. Returns (key, metadata, distance) tuples.
    fn search_by_vector(&self, query_vector: &[f32], limit: u32) -> Result<Vec<(String, String, f64)>, String>;

    /// Delete an embedding by key. Returns true if it existed.
    fn delete_embedding(&self, key: &str) -> Result<bool, String>;
}

// ============================================================================
// ObjectStorageManifest
// ============================================================================

/// Manifest for object storage configuration.
///
/// Object storage is configured independently from vector storage.
/// All paths are resolved at manifest creation time.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ObjectStorageManifest {
    /// SQLite-backed storage.
    Sqlite { path: PathBuf },
}

// ============================================================================
// VectorStorageManifest
// ============================================================================

/// Manifest for vector storage configuration.
///
/// Vector storage is configured independently from object storage.
/// All paths are resolved at manifest creation time.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VectorStorageManifest {
    /// SQLite-backed storage with sqlite-vec.
    Sqlite { path: PathBuf },
}

// ============================================================================
// In-Memory test doubles
// ============================================================================

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

/// In-memory object storage for tests.
#[cfg(test)]
pub struct InMemoryObjectStorage {
    files: Mutex<HashMap<String, Vec<u8>>>,
}

#[cfg(test)]
impl InMemoryObjectStorage {
    pub fn new() -> Self {
        InMemoryObjectStorage {
            files: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
impl ObjectStorage for InMemoryObjectStorage {
    fn put_file(&self, path: &str, content: &[u8]) -> Result<(), String> {
        let mut files = self.files.lock().map_err(|e| e.to_string())?;
        files.insert(path.to_string(), content.to_vec());
        Ok(())
    }

    fn get_file(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let files = self.files.lock().map_err(|e| e.to_string())?;
        Ok(files.get(path).cloned())
    }

    fn delete_file(&self, path: &str) -> Result<bool, String> {
        let mut files = self.files.lock().map_err(|e| e.to_string())?;
        Ok(files.remove(path).is_some())
    }

    fn list_files(&self, dir_path: &str) -> Result<Vec<String>, String> {
        let files = self.files.lock().map_err(|e| e.to_string())?;
        let prefix = if dir_path == "/" || dir_path.is_empty() {
            "/".to_string()
        } else {
            format!("{}/", dir_path.trim_end_matches('/'))
        };

        Ok(files.keys()
            .filter(|p| p.starts_with(&prefix))
            .cloned()
            .collect())
    }
}

/// In-memory vector storage for tests.
/// Uses brute-force euclidean distance for similarity search.
#[cfg(test)]
pub struct InMemoryVectorStorage {
    items: Mutex<HashMap<String, (Vec<f32>, String)>>,
}

#[cfg(test)]
impl InMemoryVectorStorage {
    pub fn new() -> Self {
        InMemoryVectorStorage {
            items: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
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

#[cfg(test)]
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

    // ========================================================================
    // ObjectStorage trait contract
    // ========================================================================

    #[test]
    fn object_put_and_get_file() {
        let storage = InMemoryObjectStorage::new();
        storage.put_file("/docs/readme.txt", b"Hello, AgentFS!").unwrap();
        let retrieved = storage.get_file("/docs/readme.txt").unwrap();
        assert_eq!(retrieved, Some(b"Hello, AgentFS!".to_vec()));
    }

    #[test]
    fn object_get_nonexistent_file() {
        let storage = InMemoryObjectStorage::new();
        let result = storage.get_file("/does/not/exist.txt").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn object_delete_file() {
        let storage = InMemoryObjectStorage::new();
        storage.put_file("/temp/data.bin", b"temporary").unwrap();
        assert!(storage.get_file("/temp/data.bin").unwrap().is_some());

        let deleted = storage.delete_file("/temp/data.bin").unwrap();
        assert!(deleted);
        assert!(storage.get_file("/temp/data.bin").unwrap().is_none());

        let deleted_again = storage.delete_file("/temp/data.bin").unwrap();
        assert!(!deleted_again);
    }

    #[test]
    fn object_list_files() {
        let storage = InMemoryObjectStorage::new();
        storage.put_file("/notes/day1.md", b"day 1").unwrap();
        storage.put_file("/notes/day2.md", b"day 2").unwrap();
        storage.put_file("/data/config.json", b"{}").unwrap();

        let notes = storage.list_files("/notes").unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes.contains(&"/notes/day1.md".to_string()));
        assert!(notes.contains(&"/notes/day2.md".to_string()));
    }

    #[test]
    fn object_overwrite_file() {
        let storage = InMemoryObjectStorage::new();
        storage.put_file("/config.yaml", b"version: 1").unwrap();
        storage.put_file("/config.yaml", b"version: 2").unwrap();
        let content = storage.get_file("/config.yaml").unwrap().unwrap();
        assert_eq!(content, b"version: 2");
    }

    #[test]
    fn object_binary_content() {
        let storage = InMemoryObjectStorage::new();
        let binary: Vec<u8> = (0..=255).collect();
        storage.put_file("/binary.dat", &binary).unwrap();
        let retrieved = storage.get_file("/binary.dat").unwrap().unwrap();
        assert_eq!(retrieved, binary);
    }

    #[test]
    fn object_list_files_empty_dir() {
        let storage = InMemoryObjectStorage::new();
        storage.put_file("/data/file.txt", b"content").unwrap();
        let empty = storage.list_files("/nonexistent").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn object_list_files_root() {
        let storage = InMemoryObjectStorage::new();
        storage.put_file("/a.txt", b"a").unwrap();
        storage.put_file("/dir/b.txt", b"b").unwrap();
        storage.put_file("/dir/sub/c.txt", b"c").unwrap();

        let all = storage.list_files("/").unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn object_unicode_paths_and_content() {
        let storage = InMemoryObjectStorage::new();
        storage.put_file("/文档/日记.txt", "今日の天気は晴れです".as_bytes()).unwrap();
        let content = storage.get_file("/文档/日记.txt").unwrap().unwrap();
        assert_eq!(String::from_utf8(content).unwrap(), "今日の天気は晴れです");
    }

    // ========================================================================
    // VectorStorage trait contract
    // ========================================================================

    fn make_test_vector(seed: f32) -> Vec<f32> {
        (0..768).map(|i| (i as f32 * seed).sin()).collect()
    }

    #[test]
    fn vector_store_and_search() {
        let storage = InMemoryVectorStorage::new();

        let vec1 = make_test_vector(1.0);
        let vec2 = make_test_vector(2.0);
        let vec3 = make_test_vector(1.1);

        storage.store_embedding("doc1", &vec1, "first document").unwrap();
        storage.store_embedding("doc2", &vec2, "second document").unwrap();
        storage.store_embedding("doc3", &vec3, "third document").unwrap();

        let results = storage.search_by_vector(&vec1, 3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "doc1");
        assert_eq!(results[0].1, "first document");
        assert!(results[0].2 < 0.0001);
    }

    #[test]
    fn vector_wrong_dimensions() {
        let storage = InMemoryVectorStorage::new();

        let wrong_vec: Vec<f32> = vec![1.0, 2.0, 3.0];
        let result = storage.store_embedding("bad", &wrong_vec, "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("768"));

        let search_result = storage.search_by_vector(&wrong_vec, 10);
        assert!(search_result.is_err());
    }

    #[test]
    fn vector_delete() {
        let storage = InMemoryVectorStorage::new();

        let vec1 = make_test_vector(1.0);
        storage.store_embedding("to-delete", &vec1, "temp").unwrap();

        let results = storage.search_by_vector(&vec1, 1).unwrap();
        assert_eq!(results.len(), 1);

        let deleted = storage.delete_embedding("to-delete").unwrap();
        assert!(deleted);

        let results = storage.search_by_vector(&vec1, 1).unwrap();
        assert_eq!(results.len(), 0);

        let deleted_again = storage.delete_embedding("to-delete").unwrap();
        assert!(!deleted_again);
    }

    #[test]
    fn vector_overwrite() {
        let storage = InMemoryVectorStorage::new();

        let vec1 = make_test_vector(1.0);
        let vec2 = make_test_vector(2.0);

        storage.store_embedding("doc", &vec1, "original").unwrap();
        storage.store_embedding("doc", &vec2, "updated").unwrap();

        let results = storage.search_by_vector(&vec2, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "doc");
        assert_eq!(results[0].1, "updated");
        assert!(results[0].2 < 0.0001);
    }

    #[test]
    fn vector_search_empty() {
        let storage = InMemoryVectorStorage::new();

        let vec1 = make_test_vector(1.0);
        let results = storage.search_by_vector(&vec1, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn vector_search_respects_limit() {
        let storage = InMemoryVectorStorage::new();

        for i in 0..5 {
            let vec = make_test_vector(i as f32);
            storage.store_embedding(&format!("doc{}", i), &vec, "").unwrap();
        }

        let query = make_test_vector(0.0);

        let results = storage.search_by_vector(&query, 2).unwrap();
        assert_eq!(results.len(), 2);

        let results = storage.search_by_vector(&query, 10).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn vector_search_orders_by_distance() {
        let storage = InMemoryVectorStorage::new();

        let base: Vec<f32> = vec![1.0; 768];
        let close: Vec<f32> = vec![1.1; 768];
        let medium: Vec<f32> = vec![2.0; 768];
        let far: Vec<f32> = vec![10.0; 768];

        storage.store_embedding("far", &far, "").unwrap();
        storage.store_embedding("close", &close, "").unwrap();
        storage.store_embedding("medium", &medium, "").unwrap();

        let results = storage.search_by_vector(&base, 3).unwrap();
        assert_eq!(results[0].0, "close");
        assert_eq!(results[1].0, "medium");
        assert_eq!(results[2].0, "far");

        assert!(results[0].2 < results[1].2);
        assert!(results[1].2 < results[2].2);
    }

    #[test]
    fn vector_unicode_metadata() {
        let storage = InMemoryVectorStorage::new();

        let vec1 = make_test_vector(1.0);

        storage.store_embedding("doc1", &vec1, "文档摘要：这是一个测试").unwrap();
        storage.store_embedding("doc2", &make_test_vector(2.0), "Party notes").unwrap();

        let results = storage.search_by_vector(&vec1, 2).unwrap();
        assert_eq!(results[0].1, "文档摘要：这是一个测试");
    }

    #[test]
    fn vector_search_limit_zero() {
        let storage = InMemoryVectorStorage::new();

        let vec1 = make_test_vector(1.0);
        storage.store_embedding("doc1", &vec1, "test").unwrap();

        let results = storage.search_by_vector(&vec1, 0).unwrap();
        assert!(results.is_empty());
    }
}

