//! Integration tests that require an OpenRouter API key.

use vlindercli::catalog::OpenRouterCatalog;
use vlindercli::domain::{EngineType, InferenceEngine, ModelCatalog};
use vlindercli::inference::OpenRouterInferenceEngine;

#[test]
#[ignore] // Requires OpenRouter API key
fn lists_models_from_openrouter() {
    let catalog = OpenRouterCatalog::from_config();
    let models = catalog.list();
    assert!(models.is_ok());
    assert!(!models.unwrap().is_empty());
}

#[test]
#[ignore] // Requires OpenRouter API key
fn resolves_model_from_openrouter() {
    let catalog = OpenRouterCatalog::from_config();
    let model = catalog.resolve("anthropic/claude-sonnet-4");
    assert!(model.is_ok());
    let model = model.unwrap();
    assert_eq!(model.engine, EngineType::OpenRouter);
    assert!(model.id.as_str().starts_with("pending-registration://"));
}

#[test]
#[ignore] // Requires valid API key and network access
fn infers_with_openrouter_api() {
    let api_key = std::env::var("VLINDER_OPENROUTER_API_KEY")
        .expect("VLINDER_OPENROUTER_API_KEY must be set");
    let engine = OpenRouterInferenceEngine::new(
        "https://openrouter.ai/api/v1",
        api_key,
        "anthropic/claude-sonnet-4-20250514",
    );
    let result = engine.infer("Say hello in one word", 10);
    assert!(result.is_ok());
    assert!(!result.unwrap().is_empty());
}
