//! Integration tests for Registry that require agent fixtures.

use std::path::{Path, PathBuf};

use vlindercli::domain::{Agent, Registry, ResourceId, RuntimeType};

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
    let mut registry = Registry::new();
    registry.register_runtime(RuntimeType::Wasm);
    let fake_id = ResourceId::new("file:///nonexistent/agent.wasm");

    // Agent not found initially
    assert!(registry.get_agent(&fake_id).is_none());

    // Register agent
    let agent = load_agent("echo-agent");
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
fn select_runtime_for_file_wasm() {
    let mut registry = Registry::new();
    registry.register_runtime(RuntimeType::Wasm);

    let agent = load_agent("echo-agent");

    // file:// + .wasm → Wasm runtime
    assert_eq!(registry.select_runtime(&agent), Some(RuntimeType::Wasm));
}

#[test]
fn select_runtime_returns_none_without_registered_runtime() {
    let registry = Registry::new(); // No runtimes registered

    let agent = load_agent("echo-agent");

    // No Wasm runtime available
    assert_eq!(registry.select_runtime(&agent), None);
}

#[test]
fn select_runtime_returns_none_for_non_file_scheme() {
    let mut registry = Registry::new();
    registry.register_runtime(RuntimeType::Wasm);

    // Load agent and modify its id to use http:// scheme
    let mut agent = load_agent("echo-agent");
    agent.id = ResourceId::new("http://example.com/agent.wasm");

    // http:// scheme → no runtime (only file:// supported)
    assert_eq!(registry.select_runtime(&agent), None);
}

#[test]
fn select_runtime_returns_none_for_non_wasm_extension() {
    let mut registry = Registry::new();
    registry.register_runtime(RuntimeType::Wasm);

    // Load agent and modify its id to use non-.wasm extension
    let mut agent = load_agent("echo-agent");
    agent.id = ResourceId::new("file:///path/to/agent.js");

    // file:// but not .wasm → no runtime
    assert_eq!(registry.select_runtime(&agent), None);
}
