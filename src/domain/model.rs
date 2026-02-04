//! Model domain type with resolved paths.

use std::path::Path;

use sha2::{Sha256, Digest};

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
    /// - Ollama models: from API
    /// - Local GGUF: sha256 of file
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
    /// In-memory engine (for testing).
    InMemory,
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

    /// Load model from a manifest file path, calculating digest from model file.
    pub fn load(path: &Path) -> Result<Model, LoadError> {
        let manifest = ModelManifest::load(path)?;

        // Extract filesystem path from file:// URI
        let model_file_path = manifest.model_path
            .strip_prefix("file://")
            .ok_or_else(|| LoadError::Parse(format!(
                "expected file:// URI, got: {}",
                manifest.model_path
            )))?;
        let digest = Self::calculate_file_digest(Path::new(model_file_path))?;

        Ok(Self::from_manifest(manifest, digest))
    }

    /// Calculate sha256 digest of a file.
    fn calculate_file_digest(path: &Path) -> Result<String, LoadError> {
        use std::io::Read;

        let mut file = std::fs::File::open(path)
            .map_err(|e| LoadError::Io(e))?;

        // Use a simple hash - read in chunks for large files
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let bytes_read = file.read(&mut buffer)
                .map_err(|e| LoadError::Io(e))?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        let hash = hasher.finalize();
        Ok(format!("sha256:{}", hex::encode(hash)))
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
