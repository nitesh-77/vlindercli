//! Inference capability domain types and traits.

// ============================================================================
// InferenceEngineType (available implementations)
// ============================================================================

/// Available inference engine implementations.
///
/// Registered with the Registry to track what backends are available.
/// Follows the same pattern as `RuntimeType` and `ObjectStorageType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InferenceEngineType {
    /// Llama.cpp-based inference.
    Llama,
    /// In-memory inference (for testing).
    InMemory,
}

// ============================================================================
// InferenceEngine Trait
// ============================================================================

/// Inference engine for text generation.
pub trait InferenceEngine: Send + Sync {
    fn infer(&self, prompt: &str, max_tokens: u32) -> Result<String, String>;
}

/// An inference capability for text generation.
#[derive(Clone, Debug)]
pub struct Inference {
    pub backend: InferenceBackend,
}

/// Backend configuration for inference.
#[derive(Clone, Debug)]
pub struct InferenceBackend {
    pub max_tokens: u32,
    pub temperature: f32,
    pub kind: InferenceKind,
}

impl Default for InferenceBackend {
    fn default() -> Self {
        InferenceBackend {
            max_tokens: 1024,
            temperature: 0.7,
            kind: InferenceKind::Llama(LlamaConfig::default()),
        }
    }
}

/// The specific inference implementation.
#[derive(Clone, Debug)]
pub enum InferenceKind {
    Llama(LlamaConfig),
}

/// Configuration for local Llama inference.
#[derive(Clone, Debug)]
pub struct LlamaConfig {
    pub model_path: String,
    pub context_size: u32,
}

impl Default for LlamaConfig {
    fn default() -> Self {
        LlamaConfig {
            model_path: String::new(),
            context_size: 8192,
        }
    }
}
