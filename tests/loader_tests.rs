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

fn vlinder_model_uri(model_name: &str) -> String {
    let path = PathBuf::from(".vlinder/models")
        .join(model_name)
        .join("model.toml")
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
    let model = load_model(&vlinder_model_uri("phi3")).unwrap();
    assert_eq!(model.name, "phi3");
}

#[test]
fn load_model_returns_correct_type() {
    let inference = load_model(&vlinder_model_uri("phi3")).unwrap();
    assert_eq!(inference.model_type, ModelType::Inference);

    let embedding = load_model(&vlinder_model_uri("nomic-embed")).unwrap();
    assert_eq!(embedding.model_type, ModelType::Embedding);
}
