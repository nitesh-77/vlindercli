//! Integration tests for ContainerRuntime (long-running model).
//!
//! Requires: podman installed + `just build-echo-container`

use std::sync::Arc;

use vlindercli::domain::{
    Agent, Registry, ResourceId, Runtime, RuntimeType, SecretStore,
    InvokeDiagnostics, InvokeMessage, MessageQueue, HarnessType, SessionId, SubmissionId,
};
use vlindercli::registry::InMemoryRegistry;
use vlindercli::queue::InMemoryQueue;
use vlindercli::runtime::ContainerRuntime;
use vlindercli::secret_store::InMemorySecretStore;

fn test_secret_store() -> Arc<dyn SecretStore> {
    Arc::new(InMemorySecretStore::new())
}

#[test]
#[ignore] // Run via: just run-integration-tests
fn container_runtime_executes_echo_agent() {
    let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());

    let registry = InMemoryRegistry::new(test_secret_store());
    registry.register_runtime(RuntimeType::Container);

    let agent = Agent::from_toml(r#"
        name = "echo-container"
        description = "Echo container agent"
        runtime = "container"
        executable = "localhost/echo-container:latest"
        [requirements]

    "#).unwrap();
    registry.register_agent(agent).unwrap();
    let agent_id = registry.agent_id("echo-container");
    let registry: Arc<dyn Registry> = Arc::new(registry);

    let mut runtime = ContainerRuntime::new(
        &ResourceId::new("http://test:9000"),
        Arc::clone(&queue),
        registry,
        vlindercli::runtime::ImagePolicy::Mutable,
        "auto",
    );

    // Send InvokeMessage
    let submission = SubmissionId::new();
    let invoke = InvokeMessage::new(
        submission.clone(),
        SessionId::new(),
        HarnessType::Cli,
        RuntimeType::Container,
        agent_id,
        b"hello from container".to_vec(),
        None,
        InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
    );
    queue.send_invoke(invoke).unwrap();

    // First tick starts container (lazy) and dispatches work
    assert!(runtime.tick());

    // Keep ticking until complete
    let start = std::time::Instant::now();
    loop {
        if runtime.tick() {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(60) {
            panic!("container did not complete within 60 seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Verify CompleteMessage (submission-scoped consumer, ADR 052)
    let (complete, ack) = queue.receive_complete(&submission, "cli").unwrap();
    assert_eq!(
        String::from_utf8(complete.payload).unwrap(),
        "hello from container"
    );
    ack().unwrap();

    // Explicit shutdown (stops containers)
    runtime.shutdown();
}
