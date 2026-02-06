//! Integration tests for ContainerRuntime.
//!
//! Requires: podman installed + `just build-echo-container`

use std::sync::Arc;

use vlindercli::domain::{Agent, InMemoryRegistry, Registry, ResourceId, Runtime, RuntimeType};
use vlindercli::queue::{InMemoryQueue, InvokeMessage, MessageQueue, HarnessType, SubmissionId};
use vlindercli::runtime::ContainerRuntime;

#[test]
#[ignore] // Requires: podman + just build-echo-container
fn container_runtime_executes_echo_agent() {
    let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());

    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);

    // Register container agent directly from TOML
    let agent = Agent::from_toml(r#"
        name = "echo-container"
        description = "Echo container agent"
        id = "container://localhost/echo-container:latest"
        [requirements]
        services = []
    "#).unwrap();
    let agent_id = agent.id.clone();
    registry.register_agent(agent).unwrap();
    let registry: Arc<dyn Registry> = Arc::new(registry);

    let mut runtime = ContainerRuntime::new(
        &ResourceId::new("http://test:9000"),
        Arc::clone(&queue),
        registry,
    );

    // Send InvokeMessage
    let invoke = InvokeMessage::new(
        SubmissionId::new(),
        HarnessType::Cli,
        RuntimeType::Container,
        agent_id,
        b"hello from container".to_vec(),
    );
    let invoke_id = invoke.id.clone();
    queue.send_invoke(invoke).unwrap();

    // First tick spawns the container
    assert!(runtime.tick());

    // Keep ticking until complete (containers take a few seconds to start)
    let start = std::time::Instant::now();
    loop {
        if runtime.tick() {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(30) {
            panic!("container did not complete within 30 seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Verify CompleteMessage
    let (complete, ack) = queue.receive_complete("cli").unwrap();
    assert_eq!(complete.correlation_id, invoke_id);
    assert_eq!(
        String::from_utf8(complete.payload).unwrap(),
        "hello from container"
    );
    ack().unwrap();
}
