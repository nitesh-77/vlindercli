//! Inference service - text generation.

use std::path::Path;

use crate::domain::{Agent, ModelType};
use crate::inference::{InferenceEngine, LlamaEngine};
use crate::loader;

pub fn infer(agent: &Agent, model_name: &str, prompt: &str) -> Result<String, Error> {
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
    if model.model_type != ModelType::Inference {
        return Err(Error::WrongModelType {
            model: model_name.to_string(),
            expected: "inference".to_string(),
            actual: format!("{:?}", model.model_type),
        });
    }

    // Get model file path from URI
    let model_path = uri_to_path(&model.model)
        .map_err(|e| Error::ModelLoad {
            model: model_name.to_string(),
            reason: e,
        })?;

    // Load and run inference
    let engine = LlamaEngine::load(&model_path)
        .map_err(|e| Error::ModelLoad {
            model: model_name.to_string(),
            reason: e,
        })?;

    engine.infer(prompt, 256)
        .map_err(Error::Inference)
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

fn uri_to_path(uri: &str) -> Result<std::path::PathBuf, String> {
    uri.strip_prefix("file://")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| format!("unsupported URI scheme: {}", uri))
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
