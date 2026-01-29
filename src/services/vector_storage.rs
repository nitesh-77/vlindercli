//! Vector storage services - embedding storage and similarity search.

use crate::storage::VectorStorage;

pub fn store_embedding(storage: &dyn VectorStorage, key: &str, vector_json: &str, metadata: &str) -> Result<String, Error> {
    let vector: Vec<f32> = serde_json::from_str(vector_json)
        .map_err(|e| Error::Json(format!("invalid vector JSON: {}", e)))?;

    storage.store_embedding(key, &vector, metadata)
        .map_err(|e| Error::Storage(e.to_string()))?;

    Ok("ok".to_string())
}

pub fn search_by_vector(storage: &dyn VectorStorage, query_json: &str, limit: u32) -> Result<String, Error> {
    let query_vector: Vec<f32> = serde_json::from_str(query_json)
        .map_err(|e| Error::Json(format!("invalid vector JSON: {}", e)))?;

    search_by_vec(storage, &query_vector, limit)
}

// ============================================================================
// Vec-based variants (for service handlers with pre-parsed vectors)
// ============================================================================

/// Store embedding with a pre-parsed vector (for service handlers).
pub fn store_embedding_vec(storage: &dyn VectorStorage, key: &str, vector: &[f32], metadata: &str) -> Result<String, Error> {
    storage.store_embedding(key, vector, metadata)
        .map_err(|e| Error::Storage(e.to_string()))?;
    Ok("ok".to_string())
}

/// Search by vector with a pre-parsed query (for service handlers).
pub fn search_by_vec(storage: &dyn VectorStorage, query: &[f32], limit: u32) -> Result<String, Error> {
    let results = storage.search_by_vector(query, limit)
        .map_err(|e| Error::Storage(e.to_string()))?;

    let formatted: Vec<serde_json::Value> = results.iter()
        .map(|(key, metadata, distance)| {
            serde_json::json!({
                "key": key,
                "metadata": metadata,
                "distance": distance
            })
        })
        .collect();

    serde_json::to_string(&formatted)
        .map_err(|e| Error::Json(e.to_string()))
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    Storage(String),
    Json(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Storage(e) => write!(f, "{}", e),
            Error::Json(e) => write!(f, "{}", e),
        }
    }
}
