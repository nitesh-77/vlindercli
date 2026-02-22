//! Shared SDK types used across the platform boundary.

use serde::{Deserialize, Serialize};

/// A single vector search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMatch {
    pub key: String,
    pub metadata: String,
    pub distance: f32,
}
