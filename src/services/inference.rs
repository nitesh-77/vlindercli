//! Inference service - text generation.

use crate::domain::InferenceEngine;

/// Run inference with a pre-resolved engine.
///
/// This is the production function called by service workers
/// after looking up the engine by model name.
pub fn run_infer(engine: &dyn InferenceEngine, prompt: &str, max_tokens: u32) -> Result<String, Error> {
    engine.infer(prompt, max_tokens)
        .map_err(Error::Inference)
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    Inference(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Inference(e) => write!(f, "{}", e),
        }
    }
}
