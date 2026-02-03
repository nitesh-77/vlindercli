//! Integration tests for Agent + Model interaction.
//!
//! Model URIs are registry resource identifiers (http://...).
//! The registry owns models; agents reference them by URI.

use std::path::{Path, PathBuf};
use vlindercli::domain::Agent;

const AGENT_FIXTURES: &str = "tests/fixtures/agents";

fn agent_fixture(name: &str) -> PathBuf {
    Path::new(AGENT_FIXTURES).join(name)
}

// ============================================================================
// Agent + Model URI Tests
// ============================================================================

#[test]
fn agent_with_model_uris() {
    // Load agent - model URIs are registry resource identifiers
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Agent has model by name (key in the map)
    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3")); // Not declared

    // Model URIs point to registry resources
    let phi3_uri = agent.model_uri("phi3").unwrap();
    assert!(phi3_uri.as_str().starts_with("http://"));
    assert!(phi3_uri.as_str().contains("/models/phi3"));
}

#[test]
fn model_uris_are_registry_resources() {
    // Load agent - URIs are registry resource identifiers
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // Verify URI format: http://<registry>/models/<name>
    for (model_name, model_uri) in &agent.requirements.models {
        assert!(model_uri.as_str().starts_with("http://"),
            "model URI should be http scheme: {}", model_uri);
        assert!(model_uri.as_str().contains("/models/"),
            "model URI should contain /models/: {}", model_uri);

        // The URI path ends with the model name that matches the key
        // (though they could differ - the key is the alias, URI contains the registry model name)
        let expected_suffix = format!("/models/{}", model_name);
        assert!(model_uri.as_str().ends_with(&expected_suffix),
            "model URI {} should end with {}", model_uri, expected_suffix);
    }
}

// ============================================================================
// Agent.model_uri() Tests
// ============================================================================

#[test]
fn model_uri_returns_some_for_declared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    // URIs are registry resource identifiers
    let phi3_uri = agent.model_uri("phi3").unwrap();
    assert!(phi3_uri.as_str().starts_with("http://"));
    assert!(phi3_uri.as_str().ends_with("/models/phi3"));

    let nomic_uri = agent.model_uri("nomic-embed").unwrap();
    assert!(nomic_uri.as_str().starts_with("http://"));
    assert!(nomic_uri.as_str().ends_with("/models/nomic-embed"));
}

#[test]
fn model_uri_returns_none_for_undeclared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert_eq!(agent.model_uri("llama3"), None);
    assert_eq!(agent.model_uri("gpt-4"), None);
}
