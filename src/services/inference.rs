//! Inference service - text generation.

use std::sync::Arc;

use crate::domain::{Agent, InferenceEngine, ModelType};
use crate::inference::open_inference_engine;
use crate::loader;

/// Run inference using the model declared by the agent.
pub fn infer(agent: &Agent, model_name: &str, prompt: &str) -> Result<String, Error> {
    let (model, _) = resolve_model(agent, model_name)?;

    let engine = open_inference_engine(&model)
        .map_err(|e| Error::ModelLoad {
            model: model_name.to_string(),
            reason: e,
        })?;

    run_inference(&engine, prompt)
}

/// Run inference with a provided engine (for testing).
pub fn infer_with_engine(
    agent: &Agent,
    model_name: &str,
    prompt: &str,
    engine: Arc<dyn InferenceEngine>,
) -> Result<String, Error> {
    // Still validate model exists and has correct type
    let _ = resolve_model(agent, model_name)?;
    run_inference(&engine, prompt)
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
    if model.model_type != ModelType::Inference {
        return Err(Error::WrongModelType {
            model: model_name.to_string(),
            expected: "inference".to_string(),
            actual: format!("{:?}", model.model_type),
        });
    }

    Ok((model, model_uri.to_string()))
}

fn run_inference(engine: &Arc<dyn InferenceEngine>, prompt: &str) -> Result<String, Error> {
    engine.infer(prompt, 256)
        .map_err(Error::Inference)
}

/// Run inference with a pre-resolved engine (for service handlers).
///
/// This is the lower-level function for when the engine is already resolved.
/// Service handlers use this after looking up the engine by model name.
pub fn run_infer(engine: &dyn InferenceEngine, prompt: &str, max_tokens: u32) -> Result<String, Error> {
    engine.infer(prompt, max_tokens)
        .map_err(Error::Inference)
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    ModelNotDeclared(String),
    ModelLoad { model: String, reason: String },
    WrongModelType { model: String, expected: String, actual: String },
    Inference(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ModelNotDeclared(name) => write!(f, "model '{}' not declared by agent", name),
            Error::ModelLoad { model, reason } => write!(f, "failed to load '{}': {}", model, reason),
            Error::WrongModelType { model, expected, actual } => {
                write!(f, "model '{}' has type {} but {} was expected", model, actual, expected)
            }
            Error::Inference(e) => write!(f, "{}", e),
        }
    }
}
