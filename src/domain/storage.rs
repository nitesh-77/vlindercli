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

