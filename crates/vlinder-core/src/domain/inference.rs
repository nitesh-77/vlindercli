//! Inference capability domain types and traits.

/// Result of an inference call, carrying the generated text and token counts.
pub struct InferenceResult {
    pub text: String,
    pub tokens_input: u32,
    pub tokens_output: u32,
}

/// Inference engine for text generation.
pub trait InferenceEngine: Send + Sync {
    fn infer(&self, prompt: &str, max_tokens: u32) -> Result<InferenceResult, String>;
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

// ============================================================================
// In-Memory Implementation (for testing)
// ============================================================================

/// In-memory inference engine that returns canned responses.
pub struct InMemoryInference {
    response: String,
}

impl InMemoryInference {
    pub fn new(response: impl Into<String>) -> Self {
        Self { response: response.into() }
    }
}

impl InferenceEngine for InMemoryInference {
    fn infer(&self, _prompt: &str, _max_tokens: u32) -> Result<InferenceResult, String> {
        Ok(InferenceResult {
            text: self.response.clone(),
            tokens_input: 0,
            tokens_output: 0,
        })
    }
}
