//! Request types for the sqlite-vec provider routes.
//!
//! These mirror the service payload types in vlinder-core but are
//! specific to the provider route validation. The provider server
//! validates incoming JSON against these types before forwarding
//! to the queue.

use serde::{Deserialize, Serialize};

/// Store a vector with its key and metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct SqliteVecStoreRequest {
    pub key: String,
    pub vector: Vec<f32>,
    pub metadata: String,
}

/// Search for nearest vectors.
#[derive(Debug, Serialize, Deserialize)]
pub struct SqliteVecSearchRequest {
    pub vector: Vec<f32>,
    pub limit: u32,
}

/// Delete a vector by key.
#[derive(Debug, Serialize, Deserialize)]
pub struct SqliteVecDeleteRequest {
    pub key: String,
}
