//! Inference engine implementations.
//!
//! The trait is defined in the domain module.

mod ollama;
mod openrouter;

pub use ollama::OllamaInferenceEngine;
pub use openrouter::OpenRouterInferenceEngine;

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{Provider, InferenceEngine, Model};

/// Open an inference engine for the given model.
pub fn open_inference_engine(model: &Model) -> Result<Arc<dyn InferenceEngine>, String> {
    match model.provider {
        Provider::Ollama => {
            let endpoint = model.model_path.authority()
                .map(|a| format!("http://{}", a))
                .unwrap_or_else(|| Config::load().ollama.endpoint);
            Ok(Arc::new(OllamaInferenceEngine::new(endpoint, model.pfname())))
        }
        Provider::OpenRouter => {
            let config = Config::load();
            let endpoint = config.openrouter.endpoint;
            let api_key = config.openrouter.api_key;
            Ok(Arc::new(OpenRouterInferenceEngine::new(endpoint, api_key, model.pfname())))
        }
    }
}

// Re-export from domain (canonical location) for backward compatibility
pub use crate::domain::InMemoryInference;
