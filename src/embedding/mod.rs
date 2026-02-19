//! Embedding engine implementations.
//!
//! The trait is defined in the domain module.

mod ollama;

pub use ollama::OllamaEmbeddingEngine;

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{EmbeddingEngine, Provider, Model};

/// Open an embedding engine for the given model.
pub fn open_embedding_engine(model: &Model) -> Result<Arc<dyn EmbeddingEngine>, String> {
    match model.provider {
        Provider::Ollama => {
            let endpoint = model.model_path.authority()
                .map(|a| format!("http://{}", a))
                .unwrap_or_else(|| Config::load().ollama.endpoint);
            Ok(Arc::new(OllamaEmbeddingEngine::new(endpoint, model.pfname())))
        }
        other => {
            Err(format!("{:?} does not support embeddings", other))
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
