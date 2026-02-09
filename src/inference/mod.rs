//! Inference engine implementations.
//!
//! The trait is defined in the domain module.

mod ollama;
mod openrouter;

pub use ollama::OllamaInferenceEngine;
pub use openrouter::OpenRouterInferenceEngine;

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{EngineType, InferenceEngine, Model};

/// Open an inference engine for the given model.
pub fn open_inference_engine(model: &Model) -> Result<Arc<dyn InferenceEngine>, String> {
    match model.engine {
        EngineType::Ollama => {
            let endpoint = model.model_path.authority()
                .map(|a| format!("http://{}", a))
                .unwrap_or_else(|| Config::load().ollama.endpoint);
            Ok(Arc::new(OllamaInferenceEngine::new(endpoint, model.bare_name())))
        }
        EngineType::OpenRouter => {
            let config = Config::load();
            let endpoint = config.openrouter.endpoint;
            let api_key = config.openrouter.api_key;
            // model_path is "openrouter://anthropic/claude-sonnet-4-20250514"
            // Strip the scheme to get the OpenRouter model identifier
            let model_id = model.model_path.as_str()
                .strip_prefix("openrouter://")
                .unwrap_or(model.model_path.as_str())
                .to_string();
            Ok(Arc::new(OpenRouterInferenceEngine::new(endpoint, api_key, model_id)))
        }
        EngineType::InMemory => {
            Err("InMemory engine should be injected directly in tests".to_string())
        }
    }
}

// ============================================================================
// In-Memory Implementation (for testing)
// ============================================================================

/// In-memory inference engine that returns canned responses.
pub struct InMemoryInference {
    response: String,
}

impl InMemoryInference {
    pub fn new(response: impl Into<String>) -> Self {
        Self { response: response.into() }
    }
}

impl InferenceEngine for InMemoryInference {
    fn infer(&self, _prompt: &str, _max_tokens: u32) -> Result<String, String> {
        Ok(self.response.clone())
    }
}
