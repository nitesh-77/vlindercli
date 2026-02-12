//! Ollama inference engine - HTTP client for Ollama API.

use serde::{Deserialize, Serialize};

use crate::domain::{InferenceEngine, InferenceResult};

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
    fn infer(&self, prompt: &str, _max_tokens: u32) -> Result<InferenceResult, String> {
        let url = format!("{}/api/generate", self.endpoint);

        let request = GenerateRequest {
            model: &self.model,
            prompt,
            stream: false,
        };

        let mut response = ureq::post(&url)
            .send_json(&request)
            .map_err(|e| format!("ollama request failed: {}", e))?;

        let body: GenerateResponse = response
            .body_mut()
            .read_json()
            .map_err(|e| format!("failed to parse ollama response: {}", e))?;

        Ok(InferenceResult {
            text: body.response,
            tokens_input: body.prompt_eval_count,
            tokens_output: body.eval_count,
        })
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
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
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
