//! Shared request/response types for service message payloads.
//!
//! These types are the single source of truth for the wire format between
//! QueueBridge (producer) and service workers (consumers). Both sides
//! import from here, eliminating schema drift.
//!
//! `RequestPayload` and `ResponsePayload` are typed enums carried on
//! `RequestMessage` and `ResponseMessage`. The `Legacy` variant wraps
//! raw bytes for the existing JSON-over-bytes protocol. Protocol-specific
//! variants (Anthropic, OpenAI, etc.) will replace `Legacy` once each
//! service migrates to SDK types.

use serde::{Deserialize, Serialize};

// ============================================================================
// Typed Payload Enums
// ============================================================================

/// Typed payload for service requests.
///
/// Each variant represents a wire protocol. `Legacy` is transitional —
/// it wraps the existing JSON-serialized bytes and will be removed once
/// all services migrate to protocol-specific SDK types.
#[derive(Clone, Debug)]
pub enum RequestPayload {
    /// Raw bytes (JSON-serialized service payloads like InferRequest, KvGetRequest).
    Legacy(Vec<u8>),
}

impl RequestPayload {
    /// Access the payload as raw bytes.
    ///
    /// Returns the inner bytes regardless of variant. When protocol-specific
    /// variants are added, this method will serialize them to bytes on demand.
    pub fn legacy_bytes(&self) -> &[u8] {
        match self {
            RequestPayload::Legacy(b) => b,
        }
    }
}

/// Typed payload for service responses.
///
/// Mirrors `RequestPayload` — each variant represents a wire protocol.
/// `Legacy` wraps raw bytes and will be removed during migration.
#[derive(Clone, Debug)]
pub enum ResponsePayload {
    /// Raw bytes (JSON-serialized service responses).
    Legacy(Vec<u8>),
}

impl ResponsePayload {
    /// Access the payload as raw bytes.
    pub fn legacy_bytes(&self) -> &[u8] {
        match self {
            ResponsePayload::Legacy(b) => b,
        }
    }
}

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
// Embedding Service
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct EmbedRequest {
    pub model: String,
    pub text: String,
}
