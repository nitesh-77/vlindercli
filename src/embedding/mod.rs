//! Embedding engine implementations.
//!
//! The trait is defined in the domain module.

mod ollama;

pub use ollama::OllamaEmbeddingEngine;

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{EmbeddingEngine, EngineType, Model};

/// Open an embedding engine for the given model.
pub fn open_embedding_engine(model: &Model) -> Result<Arc<dyn EmbeddingEngine>, String> {
    match model.engine {
        EngineType::Ollama => {
            let endpoint = model.model_path.authority()
                .map(|a| format!("http://{}", a))
                .unwrap_or_else(|| Config::load().ollama.endpoint);
            Ok(Arc::new(OllamaEmbeddingEngine::new(endpoint, model.name.clone())))
        }
        EngineType::OpenRouter => {
            Err("OpenRouter does not support embeddings".to_string())
        }
        EngineType::InMemory => {
            Err("InMemory engine should be injected directly in tests".to_string())
        }
    }
}

// ============================================================================
// In-Memory Implementation (for testing)
// ============================================================================

/// In-memory embedding engine that returns canned responses.
pub struct InMemoryEmbedding {
    embedding: Vec<f32>,
}

impl InMemoryEmbedding {
    pub fn new(embedding: Vec<f32>) -> Self {
        Self { embedding }
    }
}

impl EmbeddingEngine for InMemoryEmbedding {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
        Ok(self.embedding.clone())
    }
}
