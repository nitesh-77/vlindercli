//! Integration tests for Agent + Model interaction (ADR 094).
//!
//! Model values are registry names (plain strings).
//! The registry owns models; agents reference them by name.

use std::path::{Path, PathBuf};
use vlinder_core::domain::Agent;

const AGENT_FIXTURES: &str = "tests/fixtures/agents";

fn agent_fixture(name: &str) -> PathBuf {
    Path::new(AGENT_FIXTURES).join(name)
}

// ============================================================================
// Agent model name tests (ADR 094)
// ============================================================================

#[test]
fn agent_has_model_by_alias() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert!(agent.has_model("phi3"));
    assert!(agent.has_model("nomic-embed"));
    assert!(!agent.has_model("llama3"));
}

#[test]
fn model_name_returns_registry_name() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert_eq!(agent.model_name("phi3"), Some("phi3"));
    assert_eq!(agent.model_name("nomic-embed"), Some("nomic-embed"));
}

#[test]
fn model_name_returns_none_for_undeclared() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    assert_eq!(agent.model_name("llama3"), None);
    assert_eq!(agent.model_name("gpt-4"), None);
}

#[test]
fn model_values_are_plain_strings() {
    let agent = Agent::load(&agent_fixture("model-test-agent")).unwrap();

    for (_alias, name) in &agent.requirements.models {
        // Values are registry names, not URIs
        assert!(
            !name.contains("://"),
            "model value should be a name, not a URI: {}",
            name
        );
    }
}
