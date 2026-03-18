//! Model catalog trait for resolving models from various sources.

use std::collections::HashMap;
use std::sync::Arc;

use super::Model;

/// A catalog that resolves model names to Model configurations.
///
/// Different implementations query different sources:
/// - `OllamaCatalog`: queries Ollama API
/// - `HuggingFaceCatalog`: downloads from `HuggingFace` Hub
/// - `LocalCatalog`: scans local directory
pub trait ModelCatalog: Send + Sync {
    /// Resolve a model name to a fully configured Model.
    ///
    /// The returned Model includes the engine type, which determines
    /// how the model will be executed (Ollama HTTP, `OpenRouter` API, etc.).
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
    /// Content digest (e.g., sha256:a80c4f17...).
    pub digest: Option<String>,
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
    /// Catalog name not registered.
    UnknownCatalog(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::NotFound(name) => write!(f, "model not found: {name}"),
            CatalogError::Network(msg) => write!(f, "network error: {msg}"),
            CatalogError::Parse(msg) => write!(f, "parse error: {msg}"),
            CatalogError::UnknownCatalog(name) => write!(f, "unknown catalog: {name}"),
        }
    }
}

impl std::error::Error for CatalogError {}

// ============================================================================
// CatalogService — aggregation over multiple ModelCatalog sources
// ============================================================================

/// A service that dispatches catalog operations to named sources.
///
/// Where `ModelCatalog` represents a single source (Ollama, `OpenRouter`),
/// `CatalogService` aggregates multiple sources and routes by catalog name.
pub trait CatalogService: Send + Sync {
    /// List available catalog names (e.g. `["ollama", "openrouter"]`).
    fn catalogs(&self) -> Vec<String>;

    /// Resolve a model by name from a specific catalog.
    fn resolve(&self, catalog: &str, name: &str) -> Result<Model, CatalogError>;

    /// List models available in a specific catalog.
    fn list(&self, catalog: &str) -> Result<Vec<ModelInfo>, CatalogError>;

    /// Check if a model is available in a specific catalog.
    fn available(&self, catalog: &str, name: &str) -> bool;
}

/// In-process composite that dispatches to registered `ModelCatalog` backends.
///
/// Named "Composite" (not "`InMemory`") because the backends themselves make
/// real HTTP calls — this is just the dispatch layer.
#[derive(Default)]
pub struct CompositeCatalog {
    sources: HashMap<String, Arc<dyn ModelCatalog>>,
}

impl CompositeCatalog {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
        }
    }

    pub fn add(&mut self, name: String, catalog: Arc<dyn ModelCatalog>) {
        self.sources.insert(name, catalog);
    }
}

impl CatalogService for CompositeCatalog {
    fn catalogs(&self) -> Vec<String> {
        let mut names: Vec<String> = self.sources.keys().cloned().collect();
        names.sort();
        names
    }

    fn resolve(&self, catalog: &str, name: &str) -> Result<Model, CatalogError> {
        let source = self
            .sources
            .get(catalog)
            .ok_or_else(|| CatalogError::UnknownCatalog(catalog.to_string()))?;
        source.resolve(name)
    }

    fn list(&self, catalog: &str) -> Result<Vec<ModelInfo>, CatalogError> {
        let source = self
            .sources
            .get(catalog)
            .ok_or_else(|| CatalogError::UnknownCatalog(catalog.to_string()))?;
        source.list()
    }

    fn available(&self, catalog: &str, name: &str) -> bool {
        match self.sources.get(catalog) {
            Some(source) => source.available(name),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ModelType, Provider, ResourceId};

    /// A test-only catalog that returns hardcoded data.
    struct MockCatalog {
        models: Vec<(&'static str, Model)>,
    }

    impl MockCatalog {
        fn new(models: Vec<(&'static str, Model)>) -> Self {
            Self { models }
        }
    }

    impl ModelCatalog for MockCatalog {
        fn resolve(&self, name: &str) -> Result<Model, CatalogError> {
            self.models
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, m)| m.clone())
                .ok_or_else(|| CatalogError::NotFound(name.to_string()))
        }

        fn list(&self) -> Result<Vec<ModelInfo>, CatalogError> {
            Ok(self
                .models
                .iter()
                .map(|(n, _)| ModelInfo {
                    name: n.to_string(),
                    size: None,
                    modified: None,
                    digest: None,
                })
                .collect())
        }

        fn available(&self, name: &str) -> bool {
            self.models.iter().any(|(n, _)| *n == name)
        }
    }

    fn test_model(name: &str) -> Model {
        Model {
            id: ResourceId::new(format!("test://models/{name}")),
            name: name.to_string(),
            model_type: ModelType::Inference,
            provider: Provider::Ollama,
            model_path: ResourceId::new(format!("ollama://localhost:11434/{name}")),
            digest: String::new(),
        }
    }

    fn composite_with_mock() -> CompositeCatalog {
        let mock = Arc::new(MockCatalog::new(vec![
            ("m1", test_model("m1")),
            ("m2", test_model("m2")),
        ]));
        let mut composite = CompositeCatalog::new();
        composite.add("mock".to_string(), mock);
        composite
    }

    #[test]
    fn composite_catalogs_lists_names() {
        let mut composite = composite_with_mock();
        composite.add("another".to_string(), Arc::new(MockCatalog::new(vec![])));
        let names = composite.catalogs();
        assert_eq!(names, vec!["another", "mock"]);
    }

    #[test]
    fn composite_resolve_dispatches() {
        let composite = composite_with_mock();
        let model = composite.resolve("mock", "m1").unwrap();
        assert_eq!(model.name, "m1");
    }

    #[test]
    fn composite_unknown_catalog() {
        let composite = composite_with_mock();
        let err = composite.resolve("bogus", "m1").unwrap_err();
        assert!(
            matches!(err, CatalogError::UnknownCatalog(ref name) if name == "bogus"),
            "expected UnknownCatalog, got: {err}"
        );
    }

    #[test]
    fn composite_list_dispatches() {
        let composite = composite_with_mock();
        let models = composite.list("mock").unwrap();
        let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["m1", "m2"]);
    }

    #[test]
    fn composite_available_dispatches() {
        let composite = composite_with_mock();
        assert!(composite.available("mock", "m1"));
        assert!(!composite.available("mock", "missing"));
    }

    #[test]
    fn composite_available_unknown_catalog() {
        let composite = composite_with_mock();
        assert!(!composite.available("bogus", "m1"));
    }
}
