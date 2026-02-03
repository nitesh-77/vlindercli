//! Embedding service - vector representations.

use crate::domain::EmbeddingEngine;

/// Run embedding with a pre-resolved engine.
///
/// This is the production function called by service workers
/// after looking up the engine by model name.
pub fn run_embed(engine: &dyn EmbeddingEngine, text: &str) -> Result<Vec<f32>, Error> {
    engine.embed(text)
        .map_err(Error::Embedding)
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    Embedding(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Embedding(e) => write!(f, "{}", e),
        }
    }
}
