//! Embedding service - vector representations.

use std::sync::Arc;

use crate::domain::{Agent, EmbeddingEngine, ModelType};
use crate::embedding::open_embedding_engine;
use crate::loader;

/// Generate embeddings using the model declared by the agent.
pub fn embed(agent: &Agent, model_name: &str, text: &str) -> Result<String, Error> {
    let (model, _) = resolve_model(agent, model_name)?;

    let engine = open_embedding_engine(&model)
        .map_err(|e| Error::ModelLoad {
            model: model_name.to_string(),
            reason: e,
        })?;

    run_embedding(&engine, text)
}

/// Generate embeddings with a provided engine (for testing).
pub fn embed_with_engine(
    agent: &Agent,
    model_name: &str,
    text: &str,
    engine: Arc<dyn EmbeddingEngine>,
) -> Result<String, Error> {
    // Still validate model exists and has correct type
    let _ = resolve_model(agent, model_name)?;
    run_embedding(&engine, text)
}

/// Resolve and validate model from agent requirements.
fn resolve_model(agent: &Agent, model_name: &str) -> Result<(crate::domain::Model, String), Error> {
    // Get model URI from agent's requirements (already resolved to absolute)
    let model_uri = agent.model_uri(model_name)
        .ok_or_else(|| Error::ModelNotDeclared(model_name.to_string()))?;

    // Load model manifest
    let model = loader::load_model(model_uri.as_str())
        .map_err(|e| Error::ModelLoad {
            model: model_name.to_string(),
            reason: e.to_string(),
        })?;

    // Validate model type
    if model.model_type != ModelType::Embedding {
        return Err(Error::WrongModelType {
            model: model_name.to_string(),
            expected: "embedding".to_string(),
            actual: format!("{:?}", model.model_type),
        });
    }

    Ok((model, model_uri.to_string()))
}

fn run_embedding(engine: &Arc<dyn EmbeddingEngine>, text: &str) -> Result<String, Error> {
    let vec = engine.embed(text)
        .map_err(Error::Embedding)?;

    serde_json::to_string(&vec)
        .map_err(|e| Error::Json(e.to_string()))
}

/// Run embedding with a pre-resolved engine (for service handlers).
///
/// This is the lower-level function for when the engine is already resolved.
/// Service handlers use this after looking up the engine by model name.
pub fn run_embed(engine: &dyn EmbeddingEngine, text: &str) -> Result<Vec<f32>, Error> {
    engine.embed(text)
        .map_err(Error::Embedding)
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    ModelNotDeclared(String),
    ModelLoad { model: String, reason: String },
    WrongModelType { model: String, expected: String, actual: String },
    Embedding(String),
    Json(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ModelNotDeclared(name) => write!(f, "model '{}' not declared by agent", name),
            Error::ModelLoad { model, reason } => write!(f, "failed to load '{}': {}", model, reason),
            Error::WrongModelType { model, expected, actual } => {
                write!(f, "model '{}' has type {} but {} was expected", model, actual, expected)
            }
            Error::Embedding(e) => write!(f, "{}", e),
            Error::Json(e) => write!(f, "{}", e),
        }
    }
}
