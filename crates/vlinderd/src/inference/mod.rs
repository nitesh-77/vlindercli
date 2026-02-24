//! Inference engine implementations.
//!
//! The trait is defined in the domain module.

mod ollama;

pub use ollama::OllamaInferenceEngine;

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{InferenceEngine, Model};

/// Open an inference engine for the given model.
///
/// Only handles Ollama models. OpenRouter inference is handled by the
/// dedicated vlinder-infer-openrouter crate.
pub fn open_inference_engine(model: &Model) -> Result<Arc<dyn InferenceEngine>, String> {
    let endpoint = model.model_path.authority()
        .map(|a| format!("http://{}", a))
        .unwrap_or_else(|| Config::load().ollama.endpoint);
    Ok(Arc::new(OllamaInferenceEngine::new(endpoint, model.pfname())))
}

// Re-export from domain (canonical location) for backward compatibility
pub use crate::domain::InMemoryInference;
