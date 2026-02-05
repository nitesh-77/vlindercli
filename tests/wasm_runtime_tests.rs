//! Integration tests for WasmRuntime.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vlindercli::domain::{Agent, InMemoryRegistry, Registry, ResourceId, Runtime, RuntimeType};
use vlindercli::queue::{InMemoryQueue, InvokeMessage, MessageQueue, HarnessType, SubmissionId};
use vlindercli::runtime::WasmRuntime;

const FIXTURES: &str = "tests/fixtures/agents";

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn load_agent(name: &str) -> Agent {
    Agent::load(&fixture(name)).unwrap()
}

fn test_registry_id() -> ResourceId {
    ResourceId::new("http://test:9000")
}

#[test]
fn runtime_executes_agent_and_returns_response() {
    let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());

    // Create registry with agent registered
    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Wasm);
    let agent = load_agent("reverse-agent");
    let agent_id = agent.id.clone();
    registry.register_agent(agent).unwrap();
    let registry: Arc<dyn Registry> = Arc::new(registry);

    let mut runtime = WasmRuntime::new(&test_registry_id(), Arc::clone(&queue), registry);

    // Send typed InvokeMessage to start submission
    let invoke = InvokeMessage::new(
        SubmissionId::new(),
        HarnessType::Cli,
        RuntimeType::Wasm,
        agent_id.clone(),
        b"hello".to_vec(),
    );
    let invoke_id = invoke.id.clone();
    queue.send_invoke(invoke).unwrap();

    // First tick spawns the WASM thread
    assert!(runtime.tick());

    // Keep ticking until the task completes and response is sent
    loop {
        if runtime.tick() {
            break; // Task completed
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // CompleteMessage should be on harness queue
    let (complete, ack) = queue.receive_complete("cli").unwrap();
    assert_eq!(complete.correlation_id, invoke_id);
    assert_eq!(String::from_utf8(complete.payload.clone()).unwrap(), "olleh");
    ack().unwrap();
}
