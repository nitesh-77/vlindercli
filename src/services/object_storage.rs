//! Object storage services - file operations.

use crate::storage::ObjectStorage;

pub fn put_file(storage: &ObjectStorage, path: &str, content: &[u8]) -> Result<String, Error> {
    storage.put_file(path, content)
        .map_err(|e| Error::Storage(e.to_string()))?;
    Ok("ok".to_string())
}

pub fn get_file(storage: &ObjectStorage, path: &str) -> Result<Vec<u8>, Error> {
    match storage.get_file(path) {
        Ok(Some(content)) => Ok(content),
        Ok(None) => Err(Error::FileNotFound),
        Err(e) => Err(Error::Storage(e.to_string())),
    }
}

pub fn delete_file(storage: &ObjectStorage, path: &str) -> Result<String, Error> {
    match storage.delete_file(path) {
        Ok(true) => Ok("ok".to_string()),
        Ok(false) => Ok("not_found".to_string()),
        Err(e) => Err(Error::Storage(e.to_string())),
    }
}

pub fn list_files(storage: &ObjectStorage, dir_path: &str) -> Result<String, Error> {
    let files = storage.list_files(dir_path)
        .map_err(|e| Error::Storage(e.to_string()))?;

    serde_json::to_string(&files)
        .map_err(|e| Error::Json(e.to_string()))
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    Storage(String),
    Json(String),
    FileNotFound,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Storage(e) => write!(f, "{}", e),
            Error::Json(e) => write!(f, "{}", e),
            Error::FileNotFound => write!(f, "file not found"),
        }
    }
}
