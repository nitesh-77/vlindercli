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
    /// In-memory provider (for testing).
    #[cfg(test)]
    InMemory,
}
