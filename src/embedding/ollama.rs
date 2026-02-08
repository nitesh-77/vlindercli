//! Ollama embedding engine - HTTP client for Ollama API.

use serde::{Deserialize, Serialize};

use crate::domain::EmbeddingEngine;

/// Embedding engine that calls Ollama's HTTP API.
pub struct OllamaEmbeddingEngine {
    endpoint: String,
    model: String,
}

impl OllamaEmbeddingEngine {
    /// Create a new Ollama embedding engine.
    ///
    /// - `endpoint`: Ollama server URL (e.g., "http://localhost:11434")
    /// - `model`: Model name (e.g., "nomic-embed-text")
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
        }
    }
}

impl EmbeddingEngine for OllamaEmbeddingEngine {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let url = format!("{}/api/embed", self.endpoint);

        let request = EmbedRequest {
            model: &self.model,
            input: text,
        };

        let response = ureq::post(&url)
            .send_json(&request)
            .map_err(|e| format!("ollama request failed: {}", e))?;

        let body: EmbedResponse = response
            .into_json()
            .map_err(|e| format!("failed to parse ollama response: {}", e))?;

        body.embeddings
            .into_iter()
            .next()
            .ok_or_else(|| "ollama returned empty embeddings array".to_string())
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_engine_with_endpoint_and_model() {
        let engine = OllamaEmbeddingEngine::new("http://localhost:11434", "nomic-embed-text");
        assert_eq!(engine.endpoint, "http://localhost:11434");
        assert_eq!(engine.model, "nomic-embed-text");
    }

    #[test]
    #[ignore] // Requires running Ollama server
    fn embeds_with_ollama_server() {
        let engine = OllamaEmbeddingEngine::new("http://localhost:11434", "nomic-embed-text");
        let result = engine.embed("Hello world");
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }
}
