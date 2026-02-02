//! Embedding capability domain types and traits.

// ============================================================================
// EmbeddingEngineType (available implementations)
// ============================================================================

/// Available embedding engine implementations.
///
/// Registered with the Registry to track what backends are available.
/// Follows the same pattern as `RuntimeType` and `InferenceEngineType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmbeddingEngineType {
    /// Nomic embedding via llama.cpp.
    Nomic,
    /// In-memory embedding (for testing).
    InMemory,
}

// ============================================================================
// EmbeddingEngine Trait
// ============================================================================

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

/// Configuration for Nomic embedding (via llama.cpp).
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
