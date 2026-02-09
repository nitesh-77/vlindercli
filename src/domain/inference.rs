//! Inference capability domain types and traits.

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
            kind: InferenceKind::Ollama,
        }
    }
}

/// The specific inference implementation.
#[derive(Clone, Debug)]
pub enum InferenceKind {
    Ollama,
}
