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
    /// In-memory storage for testing.
    InMemory,
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
    /// In-memory storage for testing.
    InMemory,
}

