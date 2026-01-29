//! Storage dispatch - routes domain types to implementations.

use std::sync::Arc;

use crate::domain::{ObjectStorage, Storage, StorageKind, VectorStorage};

use super::object::{InMemoryObjectStorage, SqliteObjectStorage};
use super::vector::{InMemoryVectorStorage, SqliteVectorStorage};

/// Open object storage for the given storage configuration.
pub(crate) fn open_object_storage(storage: &Storage) -> Result<Arc<dyn ObjectStorage>, DispatchError> {
    match &storage.backend.kind {
        StorageKind::Sqlite(_) => {
            SqliteObjectStorage::open(&storage.backend.namespace)
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
            SqliteVectorStorage::open(&storage.backend.namespace)
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
            namespace: "test".to_string(),
            kind: StorageKind::InMemory,
        },
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
}
