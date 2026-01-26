//! Vector storage services - embedding storage and similarity search.

use crate::storage::Storage;

pub fn store_embedding(storage: &Storage, key: &str, vector_json: &str, metadata: &str) -> Result<String, Error> {
    let vector: Vec<f32> = serde_json::from_str(vector_json)
        .map_err(|e| Error::Json(format!("invalid vector JSON: {}", e)))?;

    storage.store_embedding(key, &vector, metadata)
        .map_err(|e| Error::Storage(e.to_string()))?;

    Ok("ok".to_string())
}

pub fn search_by_vector(storage: &Storage, query_json: &str, limit: u32) -> Result<String, Error> {
    let query_vector: Vec<f32> = serde_json::from_str(query_json)
        .map_err(|e| Error::Json(format!("invalid vector JSON: {}", e)))?;

    let results = storage.search_by_vector(&query_vector, limit)
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
