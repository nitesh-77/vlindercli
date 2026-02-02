//! Storage dispatch - routes domain types to implementations.

use std::sync::Arc;

use crate::domain::{
    ObjectStorage, ObjectStorageManifest,
    Storage, StorageKind,
    VectorStorage, VectorStorageManifest,
};

use super::object::{InMemoryObjectStorage, SqliteObjectStorage};
use super::vector::{InMemoryVectorStorage, SqliteVectorStorage};

/// Open object storage for the given storage configuration.
pub(crate) fn open_object_storage(storage: &Storage) -> Result<Arc<dyn ObjectStorage>, DispatchError> {
    match &storage.backend.kind {
        StorageKind::Sqlite(_) => {
            SqliteObjectStorage::open(&storage.backend.agent_id)
                .map(|s| Arc::new(s) as Arc<dyn ObjectStorage>)
                .map_err(DispatchError::Sqlite)
        }
        StorageKind::InMemory => {
            Ok(Arc::new(InMemoryObjectStorage::new()))
        }
    }
}

/// Open vector storage for the given storage configuration.
pub(crate) fn open_vector_storage(storage: &Storage) -> Result<Arc<dyn VectorStorage>, DispatchError> {
    match &storage.backend.kind {
        StorageKind::Sqlite(_) => {
            SqliteVectorStorage::open(&storage.backend.agent_id)
                .map(|s| Arc::new(s) as Arc<dyn VectorStorage>)
                .map_err(DispatchError::Sqlite)
        }
        StorageKind::InMemory => {
            Ok(Arc::new(InMemoryVectorStorage::new()))
        }
    }
}

/// Create in-memory storage for testing.
pub(crate) fn in_memory_storage() -> Storage {
    Storage {
        backend: crate::domain::StorageBackend {
            agent_id: "test".to_string(),
            kind: StorageKind::InMemory,
        },
    }
}

// ============================================================================
// Manifest-based dispatch (new)
// ============================================================================

/// Open object storage from a manifest.
pub fn open_object_storage_from(manifest: &ObjectStorageManifest) -> Result<Arc<dyn ObjectStorage>, DispatchError> {
    match manifest {
        ObjectStorageManifest::Sqlite { path } => {
            SqliteObjectStorage::open_at(path)
                .map(|s| Arc::new(s) as Arc<dyn ObjectStorage>)
                .map_err(DispatchError::Sqlite)
        }
        ObjectStorageManifest::InMemory => {
            Ok(Arc::new(InMemoryObjectStorage::new()))
        }
    }
}

/// Open vector storage from a manifest.
pub fn open_vector_storage_from(manifest: &VectorStorageManifest) -> Result<Arc<dyn VectorStorage>, DispatchError> {
    match manifest {
        VectorStorageManifest::Sqlite { path } => {
            SqliteVectorStorage::open_at(path)
                .map(|s| Arc::new(s) as Arc<dyn VectorStorage>)
                .map_err(DispatchError::Sqlite)
        }
        VectorStorageManifest::InMemory => {
            Ok(Arc::new(InMemoryVectorStorage::new()))
        }
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Sqlite(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Sqlite(msg) => write!(f, "sqlite storage error: {}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_object_storage_works() {
        let storage = in_memory_storage();
        let obj = open_object_storage(&storage).unwrap();

        obj.put_file("/test.txt", b"hello").unwrap();
        assert_eq!(obj.get_file("/test.txt").unwrap(), Some(b"hello".to_vec()));
    }

    #[test]
    fn in_memory_vector_storage_works() {
        let storage = in_memory_storage();
        let vec = open_vector_storage(&storage).unwrap();

        let embedding: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        vec.store_embedding("doc1", &embedding, "test doc").unwrap();

        let results = vec.search_by_vector(&embedding, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "doc1");
    }

    // Tests for manifest-based dispatch

    #[test]
    fn object_storage_from_in_memory_manifest() {
        let manifest = ObjectStorageManifest::InMemory;
        let obj = open_object_storage_from(&manifest).unwrap();

        obj.put_file("/test.txt", b"hello").unwrap();
        assert_eq!(obj.get_file("/test.txt").unwrap(), Some(b"hello".to_vec()));
    }

    #[test]
    fn vector_storage_from_in_memory_manifest() {
        let manifest = VectorStorageManifest::InMemory;
        let vec = open_vector_storage_from(&manifest).unwrap();

        let embedding: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        vec.store_embedding("doc1", &embedding, "test doc").unwrap();

        let results = vec.search_by_vector(&embedding, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "doc1");
    }

    #[test]
    fn object_storage_from_sqlite_manifest() {
        let db_path = std::env::temp_dir().join("vlinder-test-obj.db");
        let _ = std::fs::remove_file(&db_path); // clean up from previous runs

        let manifest = ObjectStorageManifest::Sqlite { path: db_path.clone() };
        let obj = open_object_storage_from(&manifest).unwrap();

        obj.put_file("/test.txt", b"hello").unwrap();
        assert_eq!(obj.get_file("/test.txt").unwrap(), Some(b"hello".to_vec()));

        let _ = std::fs::remove_file(&db_path); // clean up
    }

    #[test]
    fn vector_storage_from_sqlite_manifest() {
        let db_path = std::env::temp_dir().join("vlinder-test-vec.db");
        let _ = std::fs::remove_file(&db_path); // clean up from previous runs

        let manifest = VectorStorageManifest::Sqlite { path: db_path.clone() };
        let vec = open_vector_storage_from(&manifest).unwrap();

        let embedding: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        vec.store_embedding("doc1", &embedding, "test doc").unwrap();

        let results = vec.search_by_vector(&embedding, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "doc1");

        let _ = std::fs::remove_file(&db_path); // clean up
    }
}
