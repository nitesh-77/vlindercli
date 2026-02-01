//! Service-level tests for inference and embedding.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use vlindercli::domain::Agent;
use vlindercli::embedding::InMemoryEmbedding;
use vlindercli::inference::InMemoryInference;
use vlindercli::services::{inference, embedding};

const AGENT_FIXTURES: &str = "tests/fixtures/agents";

fn agent_fixture(name: &str) -> PathBuf {
    Path::new(AGENT_FIXTURES).join(name)
}

// ============================================================================
// Inference Service Tests
// ============================================================================

#[test]
fn infer_fails_for_undeclared_model() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    let result = inference::infer(&agent, "nonexistent-model", "test prompt");

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not declared"),
        "Expected 'not declared' error, got: {}",
        err
    );
}

#[test]
fn infer_with_engine_returns_response() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();
    let engine = Arc::new(InMemoryInference::new("Hello from the model!"));

    let result = inference::infer_with_engine(&agent, "phi3", "What is 2+2?", engine);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Hello from the model!");
}

#[test]
fn infer_with_engine_still_validates_model_exists() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();
    let engine = Arc::new(InMemoryInference::new("response"));

    // Model doesn't exist - should fail even with injected engine
    let result = inference::infer_with_engine(&agent, "nonexistent", "prompt", engine);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not declared"));
}

// ============================================================================
// Embedding Service Tests
// ============================================================================

#[test]
fn embed_fails_for_undeclared_model() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    let result = embedding::embed(&agent, "nonexistent-model", "test text");

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not declared"),
        "Expected 'not declared' error, got: {}",
        err
    );
}

#[test]
fn embed_with_engine_returns_json() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();
    let engine = Arc::new(InMemoryEmbedding::new(vec![0.1, 0.2, 0.3]));

    let result = embedding::embed_with_engine(&agent, "nomic-embed", "test text", engine);

    assert!(result.is_ok());
    let json = result.unwrap();
    assert_eq!(json, "[0.1,0.2,0.3]");
}

#[test]
fn embed_with_engine_still_validates_model_exists() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();
    let engine = Arc::new(InMemoryEmbedding::new(vec![0.1]));

    // Model doesn't exist - should fail even with injected engine
    let result = embedding::embed_with_engine(&agent, "nonexistent", "text", engine);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not declared"));
}

// ============================================================================
// Model Manifest Loading Tests
// ============================================================================

#[test]
fn infer_fails_for_missing_manifest() {
    // Create a temp agent with a model URI pointing to nonexistent manifest
    let temp_dir = std::env::temp_dir().join("vlinder-test-missing-manifest");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Copy wasm
    std::fs::copy(
        agent_fixture("echo-agent").join("agent.wasm"),
        temp_dir.join("agent.wasm"),
    ).unwrap();

    // Create manifest with nonexistent model
    let manifest = r#"
        name = "missing-manifest-agent"
        description = "Test agent"
        id = "agent.wasm"

        [requirements]
        services = ["infer"]

        [requirements.models]
        test-model = "file://./models/nonexistent.toml"
    "#;
    std::fs::write(temp_dir.join("agent.toml"), manifest).unwrap();

    let agent = Agent::load(&temp_dir).unwrap();
    let result = inference::infer(&agent, "test-model", "test prompt");

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("failed to load"),
        "Expected load failure, got: {}",
        err
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
