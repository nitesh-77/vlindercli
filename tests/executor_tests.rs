//! Executor dispatcher tests.

use std::path::{Path, PathBuf};
use vlindercli::domain::Agent;
use vlindercli::executor::open_executor;

const AGENT_FIXTURES: &str = "tests/fixtures/agents";

fn agent_fixture(name: &str) -> PathBuf {
    Path::new(AGENT_FIXTURES).join(name)
}

// ============================================================================
// open_executor Tests
// ============================================================================

#[test]
fn open_executor_returns_wasm_executor_for_wasm_code() {
    let agent = Agent::load(&agent_fixture("echo-agent")).unwrap();

    // Agent has .wasm code
    assert!(agent.code.ends_with(".wasm"));

    // Should return an executor without error
    let result = open_executor(&agent);
    assert!(result.is_ok(), "Expected WasmExecutor for .wasm code");
}

#[test]
fn open_executor_fails_for_unsupported_code_type() {
    // Create a temp agent with non-wasm code
    let temp_dir = std::env::temp_dir().join("vlinder-test-unsupported-executor");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Create a dummy "code" file with unsupported extension
    std::fs::write(temp_dir.join("agent.py"), b"# python agent").unwrap();

    let manifest = r#"
        name = "python-agent"
        description = "Agent with unsupported code type"
        code = "agent.py"

        [requirements]
        services = []
    "#;
    std::fs::write(temp_dir.join("agent.toml"), manifest).unwrap();

    let agent = Agent::load(&temp_dir).unwrap();
    let result = open_executor(&agent);

    match result {
        Ok(_) => panic!("Expected error for unsupported code type"),
        Err(e) => assert!(
            e.contains("unsupported code type"),
            "Expected 'unsupported code type' error, got: {}",
            e
        ),
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
