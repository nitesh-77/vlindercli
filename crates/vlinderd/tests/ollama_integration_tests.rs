//! Integration tests that require a running Ollama server.

use vlinderd::catalog::OllamaCatalog;
use vlinderd::domain::{Provider, ModelCatalog};

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
