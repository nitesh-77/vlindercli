//! Model domain type with resolved paths.

use std::path::Path;

use serde::Serialize;

use super::model_manifest::{ModelManifest, ModelTypeConfig, ModelEngineConfig, ParseError};
use super::resource_id::ResourceId;

/// A model with resolved paths, ready for use.
#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ModelType {
    Inference,
    Embedding,
}

/// Engine type for running models.
///
/// Used by both model manifests (what the model requires) and
/// the registry (what implementations are available).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum EngineType {
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
        let engine: EngineType = manifest.engine.into();
        let name = name_from_model_path(&manifest.model_path, engine);
        Model {
            id: Self::placeholder_id(&name),
            name,
            model_type: manifest.model_type.into(),
            engine,
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

/// Extract the backend-native model name from a `model_path` URI.
///
/// The `model_path` is the canonical source of the model's name.
///
/// Examples:
/// - `ollama://localhost:11434/nomic-embed-text:latest` → `"nomic-embed-text:latest"`
/// - `openrouter://anthropic/claude-sonnet-4` → `"anthropic/claude-sonnet-4"`
/// - `memory://test/my-model` → `"my-model"`
fn name_from_model_path(model_path: &str, engine: EngineType) -> String {
    if let Some(after_scheme) = model_path.split("://").nth(1) {
        // For openrouter:// the entire after-scheme part IS the model id
        if engine == EngineType::OpenRouter {
            return after_scheme.to_string();
        }
        // For ollama:// and others, strip authority (host:port) to get /model-name
        if let Some(slash_idx) = after_scheme.find('/') {
            let path = &after_scheme[slash_idx + 1..];
            if !path.is_empty() {
                return path.to_string();
            }
        }
    }
    // Fallback: use the full model_path
    model_path.to_string()
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
