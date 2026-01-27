//! Embedding dispatch - routes domain types to implementations.

use std::path::Path;
use std::sync::Arc;

use crate::domain::{Embedding, EmbeddingEngine, EmbeddingKind, NomicConfig};

use super::LlamaEmbeddingEngine;

/// Open an embedding engine for the given embedding configuration.
pub(crate) fn open_embedding(embedding: &Embedding) -> Result<Arc<dyn EmbeddingEngine>, DispatchError> {
    match &embedding.backend.kind {
        EmbeddingKind::Nomic(config) => {
            let path = config.model_path.strip_prefix("file://")
                .unwrap_or(&config.model_path);
            LlamaEmbeddingEngine::load(Path::new(path))
                .map(|e| Arc::new(e) as Arc<dyn EmbeddingEngine>)
                .map_err(DispatchError::Embedding)
        }
    }
}

/// Run embedding with the given text.
pub(crate) fn run_embedding(embedding: &Embedding, text: &str) -> Result<Vec<f32>, DispatchError> {
    let engine = open_embedding(embedding)?;
    engine.embed(text).map_err(DispatchError::Embedding)
}

/// Create an Embedding domain type from a model file path.
pub(crate) fn embedding_from_model_path(model_path: &str) -> Embedding {
    Embedding {
        backend: crate::domain::EmbeddingBackend {
            dimensions: 768,
            kind: EmbeddingKind::Nomic(NomicConfig {
                model_path: model_path.to_string(),
            }),
        },
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Embedding(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Embedding(msg) => write!(f, "embedding error: {}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_from_model_path_creates_nomic_config() {
        let embedding = embedding_from_model_path("/models/nomic.gguf");

        assert_eq!(embedding.backend.dimensions, 768);
        match &embedding.backend.kind {
            EmbeddingKind::Nomic(config) => {
                assert_eq!(config.model_path, "/models/nomic.gguf");
            }
        }
    }
}
