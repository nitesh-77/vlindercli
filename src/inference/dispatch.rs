//! Inference dispatch - routes domain types to implementations.

use std::path::Path;
use std::sync::Arc;

use crate::domain::{Inference, InferenceEngine, InferenceKind, LlamaConfig};

use super::LlamaEngine;

/// Open an inference engine for the given inference configuration.
pub(crate) fn open_inference(inference: &Inference) -> Result<Arc<dyn InferenceEngine>, DispatchError> {
    match &inference.backend.kind {
        InferenceKind::Llama(config) => {
            let path = config.model_path.strip_prefix("file://")
                .unwrap_or(&config.model_path);
            LlamaEngine::load(Path::new(path))
                .map(|e| Arc::new(e) as Arc<dyn InferenceEngine>)
                .map_err(DispatchError::Llama)
        }
    }
}

/// Run inference with the given prompt.
pub(crate) fn run_inference(inference: &Inference, prompt: &str) -> Result<String, DispatchError> {
    let engine = open_inference(inference)?;
    engine.infer(prompt, inference.backend.max_tokens)
        .map_err(DispatchError::Llama)
}

/// Create an Inference domain type from a model file path.
pub(crate) fn inference_from_model_path(model_path: &str) -> Inference {
    Inference {
        backend: crate::domain::InferenceBackend {
            max_tokens: 1024,
            temperature: 0.7,
            kind: InferenceKind::Llama(LlamaConfig {
                model_path: model_path.to_string(),
                context_size: 8192,
            }),
        },
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Llama(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Llama(msg) => write!(f, "llama inference error: {}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_from_model_path_creates_llama_config() {
        let inference = inference_from_model_path("/models/phi3.gguf");

        assert_eq!(inference.backend.max_tokens, 1024);
        match &inference.backend.kind {
            InferenceKind::Llama(config) => {
                assert_eq!(config.model_path, "/models/phi3.gguf");
                assert_eq!(config.context_size, 8192);
            }
        }
    }
}
