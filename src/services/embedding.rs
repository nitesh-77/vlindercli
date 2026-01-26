//! Embedding service - vector representations.

use crate::domain::Agent;
use crate::embedding::load_embedding_engine;

pub fn embed(agent: &Agent, model: &str, text: &str) -> Result<String, Error> {
    if !agent.has_model(model) {
        return Err(Error::ModelNotDeclared(model.to_string()));
    }

    let engine = load_embedding_engine(model)
        .map_err(|e| Error::ModelLoad {
            model: model.to_string(),
            reason: e,
        })?;

    let vec = engine.embed(text)
        .map_err(Error::Embedding)?;

    serde_json::to_string(&vec)
        .map_err(|e| Error::Json(e.to_string()))
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    ModelNotDeclared(String),
    ModelLoad { model: String, reason: String },
    Embedding(String),
    Json(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ModelNotDeclared(name) => write!(f, "model '{}' not declared by agent", name),
            Error::ModelLoad { model, reason } => write!(f, "failed to load '{}': {}", model, reason),
            Error::Embedding(e) => write!(f, "{}", e),
            Error::Json(e) => write!(f, "{}", e),
        }
    }
}
