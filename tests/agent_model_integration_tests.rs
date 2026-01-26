//! Integration tests for Agent + Model interaction.
//!
//! Demonstrates the pattern: Agent stores model URIs, runtime resolves them.

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
    // Load agent - stores name→URI map in requirements.models
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Agent has model by name (key in the map)
    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3")); // Not declared

    // Can get the URI for a model
    assert_eq!(
        agent.model_uri("phi3"),
        Some("file://./models/inference.toml")
    );
}

#[test]
fn runtime_resolves_model_uris() {
    use vlindercli::loader;

    // Load agent
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Runtime resolves model URIs when needed (name → uri map)
    for (model_name, model_uri) in &agent.requirements.models {
        // Resolve relative URI against agent_dir
        let resolved_uri = resolve_model_uri(model_uri, &agent.agent_dir);
        let model = loader::load_model(&resolved_uri).unwrap();

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

    assert_eq!(
        agent.model_uri("phi3"),
        Some("file://./models/inference.toml")
    );
    assert_eq!(
        agent.model_uri("nomic-embed"),
        Some("file://./models/embedding.toml")
    );
}

#[test]
fn model_uri_returns_none_for_undeclared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert_eq!(agent.model_uri("llama3"), None);
    assert_eq!(agent.model_uri("gpt-4"), None);
}

// ============================================================================
// URI Resolution Tests
// ============================================================================

/// Helper to resolve file:// URIs with relative paths against a base directory.
/// In production, this lives in the services module.
fn resolve_model_uri(uri: &str, base_dir: &Path) -> String {
    if let Some(path) = uri.strip_prefix("file://") {
        if path.starts_with("./") || !Path::new(path).is_absolute() {
            // Relative path - resolve against base_dir
            let resolved = base_dir.join(path.strip_prefix("./").unwrap_or(path));
            return format!("file://{}", resolved.display());
        }
    }
    uri.to_string()
}

#[test]
fn resolve_model_uri_absolute_path() {
    let base = Path::new("/some/agent/dir");

    // Absolute URI should pass through unchanged
    let uri = "file:///absolute/path/to/model.toml";
    assert_eq!(resolve_model_uri(uri, base), uri);
}

#[test]
fn resolve_model_uri_relative_with_dot_slash() {
    let base = Path::new("/agent/dir");

    let uri = "file://./models/phi3.toml";
    let resolved = resolve_model_uri(uri, base);
    assert_eq!(resolved, "file:///agent/dir/models/phi3.toml");
}

#[test]
fn resolve_model_uri_relative_without_dot_slash() {
    let base = Path::new("/agent/dir");

    let uri = "file://models/phi3.toml";
    let resolved = resolve_model_uri(uri, base);
    assert_eq!(resolved, "file:///agent/dir/models/phi3.toml");
}

#[test]
fn resolve_model_uri_non_file_scheme_unchanged() {
    let base = Path::new("/agent/dir");

    // Non-file:// URIs pass through (even if invalid for our loader)
    let uri = "ollama://phi3";
    assert_eq!(resolve_model_uri(uri, base), uri);
}
