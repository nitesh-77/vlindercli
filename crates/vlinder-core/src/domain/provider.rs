//! Provider — the service that hosts and serves a model.
//!
//! Determines queue routing for inference and embedding requests.
//! Used by Model (who serves it), ServiceConfig (what the agent declares),
//! and QueueBridge (where to route requests).

use serde::{Deserialize, Serialize};

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

/// A single route: method + path.
pub struct ProviderRoute {
    pub method: HttpMethod,
    pub path: String,
}

/// A provider host: hostname + its routes.
pub struct ProviderHost {
    pub hostname: String,
    pub routes: Vec<ProviderRoute>,
}

impl ProviderRoute {
    pub fn new(method: HttpMethod, path: impl Into<String>) -> Self {
        Self { method, path: path.into() }
    }
}

impl ProviderHost {
    pub fn new(hostname: impl Into<String>, routes: Vec<ProviderRoute>) -> Self {
        Self { hostname: hostname.into(), routes }
    }
}
