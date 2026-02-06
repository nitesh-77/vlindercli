//! Integration tests for Registry that require agent fixtures.

use std::path::{Path, PathBuf};

use vlindercli::domain::{Agent, InMemoryRegistry, Registry, ResourceId, RuntimeType};

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
    let fake_id = ResourceId::new("container://localhost/nonexistent-agent");

    // Agent not found initially
    assert!(registry.get_agent(&fake_id).is_none());

    // Register agent (override id to container scheme so runtime matches)
    let mut agent = load_agent("echo-agent");
    agent.id = ResourceId::new("container://localhost/echo-agent");
    let agent_id = agent.id.clone();
    registry.register_agent(agent).unwrap();

    // Now found by id
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

    let mut agent = load_agent("echo-agent");
    agent.id = ResourceId::new("container://localhost/echo-agent");

    // container:// scheme → Container runtime
    assert_eq!(registry.select_runtime(&agent), Some(RuntimeType::Container));
}

#[test]
fn select_runtime_returns_none_without_registered_runtime() {
    let registry = InMemoryRegistry::new(); // No runtimes registered

    let mut agent = load_agent("echo-agent");
    agent.id = ResourceId::new("container://localhost/echo-agent");

    // No Container runtime available
    assert_eq!(registry.select_runtime(&agent), None);
}

#[test]
fn select_runtime_returns_none_for_http_scheme() {
    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);

    // Load agent and modify its id to use http:// scheme
    let mut agent = load_agent("echo-agent");
    agent.id = ResourceId::new("http://example.com/agent");

    // http:// scheme → no runtime (only container:// supported)
    assert_eq!(registry.select_runtime(&agent), None);
}

#[test]
fn select_runtime_returns_none_for_file_scheme() {
    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);

    // Load agent and modify its id to use file:// scheme
    let mut agent = load_agent("echo-agent");
    agent.id = ResourceId::new("file:///path/to/agent.wasm");

    // file:// scheme → no runtime (only container:// supported)
    assert_eq!(registry.select_runtime(&agent), None);
}
