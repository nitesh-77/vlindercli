//! Embedding service - vector representations.

use std::path::Path;
use std::sync::Arc;

use crate::domain::{Agent, ModelType};
use crate::embedding::{open_embedding_engine, EmbeddingEngine};
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
    // Get model URI from agent's requirements
    let model_uri = agent.model_uri(model_name)
        .ok_or_else(|| Error::ModelNotDeclared(model_name.to_string()))?;

    // Resolve relative URI and load model manifest
    let resolved_uri = resolve_uri(model_uri, &agent.agent_dir);
    let model = loader::load_model(&resolved_uri)
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

    Ok((model, resolved_uri))
}

fn run_embedding(engine: &Arc<dyn EmbeddingEngine>, text: &str) -> Result<String, Error> {
    let vec = engine.embed(text)
        .map_err(Error::Embedding)?;

    serde_json::to_string(&vec)
        .map_err(|e| Error::Json(e.to_string()))
}

fn resolve_uri(uri: &str, base_dir: &Path) -> String {
    if let Some(path) = uri.strip_prefix("file://") {
        if path.starts_with("./") || !Path::new(path).is_absolute() {
            let clean_path = path.strip_prefix("./").unwrap_or(path);
            let resolved = base_dir.join(clean_path);
            return format!("file://{}", resolved.display());
        }
    }
    uri.to_string()
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
