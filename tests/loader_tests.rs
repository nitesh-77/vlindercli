//! Integration tests for loader functions that require fixtures.

use std::path::PathBuf;

use vlindercli::domain::ModelType;
use vlindercli::loader::{load_agent, load_fleet, load_model};

fn fixture_uri(name: &str) -> String {
    let path = PathBuf::from("tests/fixtures/agents")
        .join(name)
        .canonicalize()
        .unwrap();
    format!("file://{}", path.display())
}

fn fleet_fixture_uri(name: &str) -> String {
    let path = PathBuf::from("tests/fixtures/fleets")
        .join(name)
        .canonicalize()
        .unwrap();
    format!("file://{}", path.display())
}

fn model_fixture_uri(agent: &str, model_file: &str) -> String {
    let path = PathBuf::from("tests/fixtures/agents")
        .join(agent)
        .join("models")
        .join(model_file)
        .canonicalize()
        .unwrap();
    format!("file://{}", path.display())
}

#[test]
fn load_agent_with_file_uri() {
    let agent = load_agent(&fixture_uri("echo-agent")).unwrap();
    assert_eq!(agent.name, "echo-agent");
}

#[test]
fn load_fleet_with_file_uri() {
    let fleet = load_fleet(&fleet_fixture_uri("test-fleet")).unwrap();
    assert_eq!(fleet.name, "test-fleet");
}

#[test]
fn load_model_with_file_uri() {
    let model = load_model(&model_fixture_uri("model-test-agent", "inference.toml")).unwrap();
    assert_eq!(model.name, "phi3");
}

#[test]
fn load_model_returns_correct_type() {
    let inference = load_model(&model_fixture_uri("model-test-agent", "inference.toml")).unwrap();
    assert_eq!(inference.model_type, ModelType::Inference);

    let embedding = load_model(&model_fixture_uri("model-test-agent", "embedding.toml")).unwrap();
    assert_eq!(embedding.model_type, ModelType::Embedding);
}
