//! Inference engine implementations.
//!
//! The trait is defined in the domain module.

mod ollama;
mod openrouter;

pub use ollama::OllamaInferenceEngine;
pub use openrouter::OpenRouterInferenceEngine;

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{EngineType, InferenceEngine, InferenceResult, Model};

/// Open an inference engine for the given model.
pub fn open_inference_engine(model: &Model) -> Result<Arc<dyn InferenceEngine>, String> {
    match model.engine {
        EngineType::Ollama => {
            let endpoint = model.model_path.authority()
                .map(|a| format!("http://{}", a))
                .unwrap_or_else(|| Config::load().ollama.endpoint);
            Ok(Arc::new(OllamaInferenceEngine::new(endpoint, model.name.clone())))
        }
        EngineType::OpenRouter => {
            let config = Config::load();
            let endpoint = config.openrouter.endpoint;
            let api_key = config.openrouter.api_key;
            Ok(Arc::new(OpenRouterInferenceEngine::new(endpoint, api_key, model.name.clone())))
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
    fn infer(&self, _prompt: &str, _max_tokens: u32) -> Result<InferenceResult, String> {
        Ok(InferenceResult {
            text: self.response.clone(),
            tokens_input: 0,
            tokens_output: 0,
        })
    }
}
