//! Integration tests that require an OpenRouter API key.
//!
//! These tests skip gracefully if VLINDER_OPENROUTER_API_KEY is not set.

use vlinderd::catalog::OpenRouterCatalog;
use vlinderd::domain::{Provider, ModelCatalog};

/// Return the API key if set, or print a skip message and return None.
fn openrouter_key_or_skip() -> Option<String> {
    match std::env::var("VLINDER_OPENROUTER_API_KEY") {
        Ok(key) if !key.is_empty() => Some(key),
        _ => {
            eprintln!("VLINDER_OPENROUTER_API_KEY not set — skipping");
            None
        }
    }
}

#[test]
#[ignore] // Run via: just run-integration-tests
fn lists_models_from_openrouter() {
    if openrouter_key_or_skip().is_none() {
        return;
    }
    let catalog = OpenRouterCatalog::from_config();
    let models = catalog.list();
    assert!(models.is_ok());
    assert!(!models.unwrap().is_empty());
}

#[test]
#[ignore] // Run via: just run-integration-tests
fn resolves_model_from_openrouter() {
    if openrouter_key_or_skip().is_none() {
        return;
    }
    let catalog = OpenRouterCatalog::from_config();
    let model = catalog.resolve("anthropic/claude-sonnet-4");
    assert!(model.is_ok());
    let model = model.unwrap();
    assert_eq!(model.provider, Provider::OpenRouter);
    assert!(model.id.as_str().starts_with("pending-registration://"));
}
