//! Service-level tests for inference and embedding.

use std::path::{Path, PathBuf};
use vlindercli::domain::Agent;
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
fn infer_rejects_embedding_model() {
    let agent = Agent::load(&agent_fixture("type-mismatch-agent")).unwrap();

    // "inference-model" is mapped to an embedding-type model
    let result = inference::infer(&agent, "inference-model", "test prompt");

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Error: "model 'inference-model' has type Embedding but inference was expected"
    assert!(
        err.to_string().contains("Embedding") && err.to_string().contains("inference"),
        "Expected type mismatch error, got: {}",
        err
    );
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
fn embed_rejects_inference_model() {
    let agent = Agent::load(&agent_fixture("type-mismatch-agent")).unwrap();

    // "embedding-model" is mapped to an inference-type model
    let result = embedding::embed(&agent, "embedding-model", "test text");

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Error: "model 'embedding-model' has type Inference but embedding was expected"
    assert!(
        err.to_string().contains("Inference") && err.to_string().contains("embedding"),
        "Expected type mismatch error, got: {}",
        err
    );
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
        code = "agent.wasm"

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
