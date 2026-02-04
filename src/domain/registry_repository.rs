//! Repository trait for Registry persistence.

use super::{Model, EngineType, ModelType, ResourceId};

/// Repository for persisting Registry state.
///
/// Implementations handle storage (SQLite, Postgres, etc.).
/// Registry uses this for save/load operations.
pub trait RegistryRepository: Send + Sync {
    /// Save a model to the repository.
    fn save_model(&self, model: &Model) -> Result<(), RepositoryError>;

    /// Load all models from the repository.
    fn load_models(&self) -> Result<Vec<Model>, RepositoryError>;

    /// Delete a model by name.
    fn delete_model(&self, name: &str) -> Result<bool, RepositoryError>;

    /// Check if a model exists.
    fn model_exists(&self, name: &str) -> Result<bool, RepositoryError>;
}

#[derive(Debug)]
pub enum RepositoryError {
    /// Database connection or query error.
    Database(String),
    /// Serialization/deserialization error.
    Serialization(String),
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepositoryError::Database(msg) => write!(f, "database error: {}", msg),
            RepositoryError::Serialization(msg) => write!(f, "serialization error: {}", msg),
        }
    }
}

impl std::error::Error for RepositoryError {}

/// Stored model representation for persistence.
#[derive(Debug)]
pub struct StoredModel {
    pub name: String,
    pub model_type: String,
    pub engine: String,
    pub model_path: String,
    pub digest: String,
}

impl StoredModel {
    pub fn from_model(model: &Model) -> Self {
        Self {
            name: model.name.clone(),
            model_type: match model.model_type {
                ModelType::Inference => "inference".to_string(),
                ModelType::Embedding => "embedding".to_string(),
            },
            engine: match model.engine {
                EngineType::Llama => "llama".to_string(),
                EngineType::Ollama => "ollama".to_string(),
                EngineType::InMemory => "inmemory".to_string(),
            },
            model_path: model.model_path.as_str().to_string(),
            digest: model.digest.clone(),
        }
    }

    pub fn to_model(&self) -> Result<Model, RepositoryError> {
        let model_type = match self.model_type.as_str() {
            "inference" => ModelType::Inference,
            "embedding" => ModelType::Embedding,
            other => return Err(RepositoryError::Serialization(
                format!("unknown model type: {}", other)
            )),
        };

        let engine = match self.engine.as_str() {
            "llama" => EngineType::Llama,
            "ollama" => EngineType::Ollama,
            "inmemory" => EngineType::InMemory,
            other => return Err(RepositoryError::Serialization(
                format!("unknown engine type: {}", other)
            )),
        };

        Ok(Model {
            id: Model::placeholder_id(&self.name),
            name: self.name.clone(),
            model_type,
            engine,
            model_path: ResourceId::new(&self.model_path),
            digest: self.digest.clone(),
        })
    }
}
