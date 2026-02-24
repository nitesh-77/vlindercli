//! Integration tests that require a running Ollama server.

use vlinderd::catalog::OllamaCatalog;
use vlinderd::domain::{EmbeddingEngine, Provider, ModelCatalog};
use vlinderd::embedding::OllamaEmbeddingEngine;

#[test]
#[ignore] // Run via: just run-integration-tests
fn lists_models_from_ollama() {
    let catalog = OllamaCatalog::from_config();
    let models = catalog.list();
    assert!(models.is_ok());
}

#[test]
#[ignore] // Run via: just run-integration-tests
fn resolves_model_from_ollama() {
    let catalog = OllamaCatalog::from_config();
    let model = catalog.resolve("phi3");
    assert!(model.is_ok());
    let model = model.unwrap();
    assert_eq!(model.provider, Provider::Ollama);
    assert!(model.id.as_str().starts_with("pending-registration://"));
}

#[test]
#[ignore] // Run via: just run-integration-tests
fn embeds_with_ollama_server() {
    let engine = OllamaEmbeddingEngine::new("http://localhost:11434", "nomic-embed-text");
    let result = engine.embed("Hello world");
    assert!(result.is_ok());
    assert!(!result.unwrap().is_empty());
}
