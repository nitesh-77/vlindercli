//! Model domain type with resolved paths.

use std::path::Path;

use super::model_manifest::{ModelManifest, ModelTypeConfig, ModelEngineConfig, ParseError};
use super::resource_id::ResourceId;

/// A model with resolved paths, ready for use.
#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
    pub model_type: ModelType,
    pub engine: ModelEngine,
    /// Resource URI pointing to model weights (e.g., GGUF file, Ollama model)
    pub id: ResourceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelType {
    Inference,
    Embedding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelEngine {
    Llama,
}

impl Model {
    /// Create a model from a manifest.
    ///
    /// The manifest's `id` field is already a resolved URI.
    pub fn from_manifest(manifest: ModelManifest) -> Model {
        Model {
            name: manifest.name,
            model_type: manifest.model_type.into(),
            engine: manifest.engine.into(),
            id: ResourceId::new(manifest.id),
        }
    }

    /// Convenience: load model from a manifest file path.
    pub fn load(path: &Path) -> Result<Model, LoadError> {
        let manifest = ModelManifest::load(path)?;
        Ok(Self::from_manifest(manifest))
    }
}

impl From<ModelTypeConfig> for ModelType {
    fn from(config: ModelTypeConfig) -> Self {
        match config {
            ModelTypeConfig::Inference => ModelType::Inference,
            ModelTypeConfig::Embedding => ModelType::Embedding,
        }
    }
}

impl From<ModelEngineConfig> for ModelEngine {
    fn from(config: ModelEngineConfig) -> Self {
        match config {
            ModelEngineConfig::Llama => ModelEngine::Llama,
        }
    }
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
}

impl From<ParseError> for LoadError {
    fn from(e: ParseError) -> Self {
        match e {
            ParseError::Io(e) => LoadError::Io(e),
            ParseError::Toml(s) => LoadError::Parse(s),
            ParseError::IdNotFound(s) => LoadError::Parse(s),
        }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{}", e),
            LoadError::Parse(e) => write!(f, "{}", e),
        }
    }
}
