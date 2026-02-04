//! Ollama model catalog - queries Ollama API for available models.

use serde::Deserialize;

use crate::config::Config;
use crate::domain::{
    CatalogError, EngineType, Model, ModelCatalog, ModelInfo, ModelType, ResourceId,
};

/// Catalog that queries Ollama's API for available models.
pub struct OllamaCatalog {
    endpoint: String,
}

impl OllamaCatalog {
    /// Create a new Ollama catalog.
    ///
    /// - `endpoint`: Ollama server URL (e.g., "http://localhost:11434")
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    /// Create catalog using endpoint from config.
    pub fn from_config() -> Self {
        Self::new(Config::load().ollama.endpoint)
    }
}

impl ModelCatalog for OllamaCatalog {
    fn resolve(&self, name: &str) -> Result<Model, CatalogError> {
        // Verify model exists by listing and checking
        let models = self.list()?;
        let info = models
            .iter()
            .find(|m| m.name == name || m.name.starts_with(&format!("{}:", name)))
            .ok_or_else(|| CatalogError::NotFound(name.to_string()))?;

        // Determine model type from name heuristics
        let model_type = if name.contains("embed") {
            ModelType::Embedding
        } else {
            ModelType::Inference
        };

        let host = self.endpoint.trim_start_matches("http://");
        let digest = info.digest.clone()
            .ok_or_else(|| CatalogError::Parse("missing digest from Ollama API".to_string()))?;

        Ok(Model {
            id: Model::placeholder_id(&info.name),
            name: info.name.clone(),
            model_type,
            engine: EngineType::Ollama,
            model_path: ResourceId::new(format!("ollama://{}/{}", host, info.name)),
            digest,
        })
    }

    fn list(&self) -> Result<Vec<ModelInfo>, CatalogError> {
        let url = format!("{}/api/tags", self.endpoint);

        let response = ureq::get(&url)
            .call()
            .map_err(|e| CatalogError::Network(e.to_string()))?;

        let body: TagsResponse = response
            .into_json()
            .map_err(|e| CatalogError::Parse(e.to_string()))?;

        Ok(body
            .models
            .into_iter()
            .map(|m| ModelInfo {
                name: m.name,
                size: Some(format_size(m.size)),
                modified: Some(m.modified_at),
                digest: Some(format!("sha256:{}", m.digest)),
            })
            .collect())
    }

    fn available(&self, name: &str) -> bool {
        self.list()
            .map(|models| {
                models
                    .iter()
                    .any(|m| m.name == name || m.name.starts_with(&format!("{}:", name)))
            })
            .unwrap_or(false)
    }
}

fn format_size(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    }
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
    size: u64,
    modified_at: String,
    digest: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_catalog_with_endpoint() {
        let catalog = OllamaCatalog::new("http://localhost:11434");
        assert_eq!(catalog.endpoint, "http://localhost:11434");
    }

    #[test]
    #[ignore] // Requires running Ollama server
    fn lists_models_from_ollama() {
        let catalog = OllamaCatalog::from_config();
        let models = catalog.list();
        assert!(models.is_ok());
    }

    #[test]
    #[ignore] // Requires running Ollama server with model pulled
    fn resolves_model_from_ollama() {
        let catalog = OllamaCatalog::from_config();
        let model = catalog.resolve("phi3");
        assert!(model.is_ok());
        let model = model.unwrap();
        assert_eq!(model.engine, EngineType::Ollama);
        assert!(model.id.as_str().starts_with("pending-registration://"));
    }
}
