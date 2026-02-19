//! Embedding capability domain types and traits.

/// Embedding engine for vector generation.
pub trait EmbeddingEngine: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

/// An embedding capability for vector representations.
#[derive(Clone, Debug)]
pub struct Embedding {
    pub backend: EmbeddingBackend,
}

/// Backend configuration for embedding.
#[derive(Clone, Debug)]
pub struct EmbeddingBackend {
    pub dimensions: u32,
    pub kind: EmbeddingKind,
}

impl Default for EmbeddingBackend {
    fn default() -> Self {
        EmbeddingBackend {
            dimensions: 768,
            kind: EmbeddingKind::Nomic(NomicConfig::default()),
        }
    }
}

/// The specific embedding implementation.
#[derive(Clone, Debug)]
pub enum EmbeddingKind {
    Nomic(NomicConfig),
}

/// Configuration for Nomic embedding (via Ollama).
#[derive(Clone, Debug)]
pub struct NomicConfig {
    pub model_path: String,
}

impl Default for NomicConfig {
    fn default() -> Self {
        NomicConfig {
            model_path: String::new(),
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
