//! Shared request/response types for service message payloads.
//!
//! `RequestPayload` and `ResponsePayload` are typed enums carried on
//! `RequestMessage` and `ResponseMessage`. The `Legacy` variant wraps
//! raw bytes for the existing JSON-over-bytes protocol. Protocol-specific
//! variants (Anthropic, OpenAI, etc.) will replace `Legacy` once each
//! service migrates to SDK types.
//!
//! KV types have moved to the vlinder-sqlite-kv provider crate.

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
