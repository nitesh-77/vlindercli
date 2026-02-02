//! Integration tests for WasmRuntime.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vlindercli::domain::{Agent, ResourceId, Runtime};
use vlindercli::queue::{InMemoryQueue, Message, MessageQueue};
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
    let mut runtime = WasmRuntime::new(&test_registry_id(), Arc::clone(&queue));

    // Register agent
    let agent = load_agent("reverse-agent");
    let agent_id = agent.id.clone();
    runtime.register(agent);

    // Send message to agent's queue (keyed by agent_id)
    let reply_queue = "test-reply";
    let request = Message::request(b"hello".to_vec(), reply_queue);
    let request_id = request.id.clone();
    queue.send(agent_id.as_str(), request).unwrap();

    // First tick spawns the WASM thread
    assert!(runtime.tick());

    // Keep ticking until the task completes and response is sent
    loop {
        if runtime.tick() {
            break; // Task completed
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // Response should be on reply queue
    let response = queue.receive(reply_queue).unwrap();
    assert_eq!(response.correlation_id, Some(request_id));
    assert_eq!(String::from_utf8(response.payload).unwrap(), "olleh");
}
