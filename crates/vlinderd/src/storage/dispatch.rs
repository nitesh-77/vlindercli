//! Storage dispatch - routes domain types to implementations.

use std::sync::Arc;

use crate::domain::{ObjectStorage, ObjectStorageManifest};

use super::object::SqliteObjectStorage;

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
    }
}

// ============================================================================
// URI-based dispatch (for lazy opening)
// ============================================================================

use crate::domain::ResourceId;
use std::path::Path;

/// Open object storage from a URI.
///
/// Dispatches based on URI scheme:
/// - `sqlite://path` → SQLite storage at path
pub fn open_object_storage_from_uri(uri: &ResourceId) -> Result<Arc<dyn ObjectStorage>, DispatchError> {
    match uri.scheme() {
        Some("sqlite") => {
            let path = uri.path().ok_or_else(|| {
                DispatchError::InvalidUri("sqlite URI missing path".to_string())
            })?;
            SqliteObjectStorage::open_at(Path::new(path))
                .map(|s| Arc::new(s) as Arc<dyn ObjectStorage>)
                .map_err(DispatchError::Sqlite)
        }
        Some(scheme) => Err(DispatchError::UnknownScheme(scheme.to_string())),
        None => Err(DispatchError::InvalidUri("missing scheme".to_string())),
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Sqlite(String),
    UnknownScheme(String),
    InvalidUri(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Sqlite(msg) => write!(f, "sqlite storage error: {}", msg),
            DispatchError::UnknownScheme(scheme) => write!(f, "unknown storage scheme: {}", scheme),
            DispatchError::InvalidUri(msg) => write!(f, "invalid storage URI: {}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_storage_from_sqlite_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("obj.db");

        let manifest = ObjectStorageManifest::Sqlite { path: db_path };
        let obj = open_object_storage_from(&manifest).unwrap();

        obj.put_file("/test.txt", b"hello").unwrap();
        assert_eq!(obj.get_file("/test.txt").unwrap(), Some(b"hello".to_vec()));
    }
}
