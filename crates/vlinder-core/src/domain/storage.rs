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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
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
}

