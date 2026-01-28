//! Integration tests for Agent + Model interaction.
//!
//! Model URIs are resolved to absolute paths at agent load time.

use std::path::{Path, PathBuf};
use vlindercli::domain::{Agent, ModelType};

const AGENT_FIXTURES: &str = "tests/fixtures/agents";

fn agent_fixture(name: &str) -> PathBuf {
    Path::new(AGENT_FIXTURES).join(name)
}

// ============================================================================
// Agent + Model URI Tests
// ============================================================================

#[test]
fn agent_with_model_uris() {
    // Load agent - model URIs are resolved to absolute at load time
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Agent has model by name (key in the map)
    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3")); // Not declared

    // Model URIs are now absolute (resolved at load time)
    let phi3_uri = agent.model_uri("phi3").unwrap();
    assert!(phi3_uri.as_str().starts_with("file://"));
    assert!(phi3_uri.as_str().contains("/models/inference.toml"));
}

#[test]
fn model_uris_already_resolved() {
    use vlindercli::loader;

    // Load agent - URIs are already absolute
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Can load models directly without further resolution
    for (model_name, model_uri) in &agent.requirements.models {
        let model = loader::load_model(model_uri.as_str()).unwrap();

        // Verify model name matches key and type is correct
        assert_eq!(&model.name, model_name);
        match model.name.as_str() {
            "phi3" => assert_eq!(model.model_type, ModelType::Inference),
            "nomic-embed" => assert_eq!(model.model_type, ModelType::Embedding),
            name => panic!("unexpected model: {}", name),
        }
    }
}

// ============================================================================
// Agent.model_uri() Tests
// ============================================================================

#[test]
fn model_uri_returns_some_for_declared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // URIs are absolute after loading
    let phi3_uri = agent.model_uri("phi3").unwrap();
    assert!(phi3_uri.as_str().starts_with("file://"));
    assert!(phi3_uri.as_str().ends_with("/models/inference.toml"));

    let nomic_uri = agent.model_uri("nomic-embed").unwrap();
    assert!(nomic_uri.as_str().starts_with("file://"));
    assert!(nomic_uri.as_str().ends_with("/models/embedding.toml"));
}

#[test]
fn model_uri_returns_none_for_undeclared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert_eq!(agent.model_uri("llama3"), None);
    assert_eq!(agent.model_uri("gpt-4"), None);
}
