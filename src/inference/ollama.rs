//! Ollama inference engine - HTTP client for Ollama API.

use serde::{Deserialize, Serialize};

use crate::domain::InferenceEngine;

/// Inference engine that calls Ollama's HTTP API.
pub struct OllamaInferenceEngine {
    endpoint: String,
    model: String,
}

impl OllamaInferenceEngine {
    /// Create a new Ollama inference engine.
    ///
    /// - `endpoint`: Ollama server URL (e.g., "http://localhost:11434")
    /// - `model`: Model name (e.g., "phi3")
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
        }
    }
}

impl InferenceEngine for OllamaInferenceEngine {
    fn infer(&self, prompt: &str, _max_tokens: u32) -> Result<String, String> {
        let url = format!("{}/api/generate", self.endpoint);

        let request = GenerateRequest {
            model: &self.model,
            prompt,
            stream: false,
        };

        let response = ureq::post(&url)
            .send_json(&request)
            .map_err(|e| format!("ollama request failed: {}", e))?;

        let body: GenerateResponse = response
            .into_json()
            .map_err(|e| format!("failed to parse ollama response: {}", e))?;

        Ok(body.response)
    }
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_engine_with_endpoint_and_model() {
        let engine = OllamaInferenceEngine::new("http://localhost:11434", "phi3");
        assert_eq!(engine.endpoint, "http://localhost:11434");
        assert_eq!(engine.model, "phi3");
    }
}
