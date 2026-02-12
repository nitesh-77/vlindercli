//! Shared request/response types for service message payloads.
//!
//! These types are the single source of truth for the wire format between
//! QueueBridge (producer) and service workers (consumers). Both sides
//! import from here, eliminating schema drift.

use serde::{Deserialize, Serialize};

// ============================================================================
// KV Service (object storage)
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct KvGetRequest {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KvPutRequest {
    pub path: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KvListRequest {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KvDeleteRequest {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KvPutResponse {
    pub state: String,
}

// ============================================================================
// Vector Service
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct VectorStoreRequest {
    pub key: String,
    pub vector: Vec<f32>,
    pub metadata: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VectorSearchRequest {
    pub vector: Vec<f32>,
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VectorDeleteRequest {
    pub key: String,
}

// ============================================================================
// Inference Service
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct InferRequest {
    pub model: String,
    pub prompt: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    256
}

// ============================================================================
// Embedding Service
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct EmbedRequest {
    pub model: String,
    pub text: String,
}
