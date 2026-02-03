//! Model catalog trait for resolving models from various sources.

use super::Model;

/// A catalog that resolves model names to Model configurations.
///
/// Different implementations query different sources:
/// - OllamaCatalog: queries Ollama API
/// - HuggingFaceCatalog: downloads from HuggingFace Hub
/// - LocalCatalog: scans local directory
pub trait ModelCatalog: Send + Sync {
    /// Resolve a model name to a fully configured Model.
    ///
    /// The returned Model includes the engine type, which determines
    /// how the model will be executed (Ollama HTTP, llama.cpp local, etc.).
    fn resolve(&self, name: &str) -> Result<Model, CatalogError>;

    /// List available models in this catalog.
    fn list(&self) -> Result<Vec<ModelInfo>, CatalogError>;

    /// Check if a model is available without fully resolving it.
    fn available(&self, name: &str) -> bool;
}

/// Brief information about an available model.
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub name: String,
    pub size: Option<String>,
    pub modified: Option<String>,
}

/// Errors that can occur when interacting with a catalog.
#[derive(Debug)]
pub enum CatalogError {
    /// Model not found in catalog.
    NotFound(String),
    /// Network or API error.
    Network(String),
    /// Failed to parse response.
    Parse(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::NotFound(name) => write!(f, "model not found: {}", name),
            CatalogError::Network(msg) => write!(f, "network error: {}", msg),
            CatalogError::Parse(msg) => write!(f, "parse error: {}", msg),
        }
    }
}

impl std::error::Error for CatalogError {}
