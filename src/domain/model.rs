//! Model domain type with resolved paths.

use std::path::Path;

use super::model_manifest::{ModelManifest, ModelTypeConfig, ModelEngineConfig, ParseError};
use super::resource_id::ResourceId;

/// A model with resolved paths, ready for use.
#[derive(Clone, Debug)]
pub struct Model {
    /// Registry-assigned identity: `<registry_id>/models/<name>`.
    /// Set by the registry during registration.
    pub id: ResourceId,
    pub name: String,
    pub model_type: ModelType,
    pub engine: EngineType,
    /// Path to the model file (e.g., GGUF).
    pub model_path: ResourceId,
    /// Content digest for reproducibility (e.g., sha256:a80c4f17...).
    /// Ollama models get this from the API. Empty for local models.
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelType {
    Inference,
    Embedding,
}

/// Engine type for running models.
///
/// Used by both model manifests (what the model requires) and
/// the registry (what implementations are available).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EngineType {
    /// Llama.cpp-based engine (local GGUF files).
    Llama,
    /// Ollama HTTP API.
    Ollama,
    /// OpenRouter API (cloud LLMs via OpenAI-compatible endpoint).
    OpenRouter,
    /// In-memory engine (for testing).
    InMemory,
}

impl EngineType {
    /// Backend string used for queue routing subjects.
    pub fn as_backend_str(&self) -> &str {
        match self {
            EngineType::Llama => "llama",
            EngineType::Ollama => "ollama",
            EngineType::OpenRouter => "openrouter",
            EngineType::InMemory => "memory",
        }
    }
}

impl Model {
    /// Create a placeholder ID for models not yet registered.
    ///
    /// Registry replaces this with `<registry_id>/models/<name>` during registration.
    pub fn placeholder_id(name: &str) -> ResourceId {
        ResourceId::new(format!("pending-registration://models/{}", name))
    }

    /// Create a model from a manifest with a pre-computed digest.
    ///
    /// The `id` field is set to a placeholder. The registry assigns the real
    /// id (`<registry_id>/models/<name>`) during registration.
    pub fn from_manifest(manifest: ModelManifest, digest: String) -> Model {
        let name = manifest.name.clone();
        Model {
            id: Self::placeholder_id(&name),
            name: manifest.name,
            model_type: manifest.model_type.into(),
            engine: manifest.engine.into(),
            model_path: ResourceId::new(manifest.model_path),
            digest,
        }
    }

    /// Load model from a manifest file path.
    pub fn load(path: &Path) -> Result<Model, LoadError> {
        let manifest = ModelManifest::load(path)?;
        Ok(Self::from_manifest(manifest, String::new()))
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

impl From<ModelEngineConfig> for EngineType {
    fn from(config: ModelEngineConfig) -> Self {
        match config {
            ModelEngineConfig::Llama => EngineType::Llama,
            ModelEngineConfig::Ollama => EngineType::Ollama,
            ModelEngineConfig::OpenRouter => EngineType::OpenRouter,
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
            ParseError::ModelPathNotFound(s) => LoadError::Parse(s),
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
