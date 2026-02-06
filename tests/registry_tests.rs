//! Integration tests for Registry that require agent fixtures.

use std::path::{Path, PathBuf};

use vlindercli::domain::{Agent, InMemoryRegistry, Registry, RuntimeType};

const FIXTURES: &str = "tests/fixtures/agents";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn load_agent(name: &str) -> Agent {
    Agent::load(&fixture(name)).unwrap()
}

// ============================================================================
// Agent Registration Tests
// ============================================================================

#[test]
fn agent_registration() {
    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);

    // Agent not found initially
    let agent_id = registry.agent_id("echo-agent");
    assert!(registry.get_agent(&agent_id).is_none());

    // Register agent — registry assigns identity
    let agent = load_agent("echo-agent");
    registry.register_agent(agent).unwrap();

    // Now found by registry-assigned id
    let agent = registry.get_agent(&agent_id).unwrap();
    assert_eq!(agent.name, "echo-agent");
}

// ============================================================================
// Runtime Selection Tests
// ============================================================================

#[test]
fn select_runtime_identifies_container_agent() {
    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);

    let agent = load_agent("echo-agent");

    // Agent declares runtime = "container" → Container runtime selected
    assert_eq!(registry.select_runtime(&agent), Some(RuntimeType::Container));
}

#[test]
fn select_runtime_returns_none_without_registered_runtime() {
    let registry = InMemoryRegistry::new(); // No runtimes registered

    let agent = load_agent("echo-agent");

    // No Container runtime available
    assert_eq!(registry.select_runtime(&agent), None);
}
