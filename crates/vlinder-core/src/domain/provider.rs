//! Provider — the service that hosts and serves a model.
//!
//! Determines queue routing for inference and embedding requests.
//! Used by Model (who serves it), ServiceConfig (what the agent declares),
//! and the provider server (where to route requests).

use serde::{Deserialize, Serialize};

use super::operation::Operation;
use super::routing_key::ServiceBackend;

/// Model provider — the service that hosts and serves the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Ollama HTTP API (local models).
    Ollama,
    /// OpenRouter API (cloud LLMs via OpenAI-compatible endpoint).
    OpenRouter,
}

// ============================================================================
// Provider server contract types (pure Rust — no HTTP framework dependency)
// ============================================================================

/// HTTP method for provider routes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

/// Payload validation failure.
#[derive(Debug)]
pub enum PayloadError {
    /// Bytes could not be deserialized as the expected type.
    InvalidPayload(String),
}

impl std::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PayloadError::InvalidPayload(msg) => write!(f, "invalid payload: {msg}"),
        }
    }
}

/// Validates that raw bytes can be deserialized as the expected type.
pub type TypeValidator = fn(&[u8]) -> Result<(), PayloadError>;

/// A single route: method + path + request/response type validators + queue routing.
pub struct ProviderRoute {
    pub method: HttpMethod,
    pub path: String,
    pub validate_request: TypeValidator,
    pub validate_response: TypeValidator,
    pub service_backend: ServiceBackend,
    pub operation: Operation,
}

/// A provider host: hostname + its routes.
pub struct ProviderHost {
    pub hostname: String,
    pub routes: Vec<ProviderRoute>,
}

impl ProviderRoute {
    pub fn new<Req, Resp>(
        method: HttpMethod,
        path: impl Into<String>,
        service_backend: ServiceBackend,
        operation: Operation,
    ) -> Self
    where
        Req: serde::de::DeserializeOwned,
        Resp: serde::de::DeserializeOwned,
    {
        Self {
            method,
            path: path.into(),
            validate_request: |bytes| {
                serde_json::from_slice::<Req>(bytes)
                    .map(|_| ())
                    .map_err(|e| PayloadError::InvalidPayload(e.to_string()))
            },
            validate_response: |bytes| {
                serde_json::from_slice::<Resp>(bytes)
                    .map(|_| ())
                    .map_err(|e| PayloadError::InvalidPayload(e.to_string()))
            },
            service_backend,
            operation,
        }
    }
}

impl ProviderHost {
    pub fn new(hostname: impl Into<String>, routes: Vec<ProviderRoute>) -> Self {
        Self {
            hostname: hostname.into(),
            routes,
        }
    }
}
