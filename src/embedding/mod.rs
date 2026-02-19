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

// Re-export from domain (canonical location) for backward compatibility
pub use crate::domain::InMemoryEmbedding;
