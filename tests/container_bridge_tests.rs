//! Integration tests for ContainerRuntime HTTP bridge.
//!
//! Requires: podman installed + `just build-kv-bridge-agent`

use std::sync::Arc;
use std::time::{Duration, Instant};

use vlindercli::domain::{
    Agent, InMemoryRegistry, ObjectStorageType, Provider, Registry,
    ResourceId, Runtime, RuntimeType,
};
use vlindercli::queue::{InMemoryQueue, InvokeMessage, MessageQueue, HarnessType, SubmissionId};
use vlindercli::runtime::ContainerRuntime;

#[test]
#[ignore] // Requires: podman + just build-kv-bridge-agent
fn container_bridge_kv_round_trip() {
    let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());

    let registry = InMemoryRegistry::new();
    registry.register_runtime(RuntimeType::Container);
    registry.register_object_storage(ObjectStorageType::InMemory);

    let agent = Agent::from_toml(r#"
        name = "kv-bridge-agent"
        description = "KV bridge test agent"
        id = "container://localhost/kv-bridge-agent:latest"
        object_storage = "memory://"
        [requirements]
        services = ["kv"]
    "#).unwrap();
    let agent_id = agent.id.clone();
    registry.register_agent(agent).unwrap();
    let registry: Arc<dyn Registry> = Arc::new(registry);

    // Provider handles KV service requests from the bridge
    let provider = Provider::new(Arc::clone(&queue), Arc::clone(&registry));

    let mut runtime = ContainerRuntime::new(
        &ResourceId::new("http://test:9000"),
        Arc::clone(&queue),
        Arc::clone(&registry),
    );

    // Send InvokeMessage
    let invoke = InvokeMessage::new(
        SubmissionId::new(),
        HarnessType::Cli,
        RuntimeType::Container,
        agent_id,
        b"bridge test data".to_vec(),
    );
    let invoke_id = invoke.id.clone();
    queue.send_invoke(invoke).unwrap();

    // First tick starts container and dispatches work
    assert!(runtime.tick());

    // Tick both runtime and provider until complete
    let start = Instant::now();
    loop {
        // Provider processes KV requests from the bridge
        provider.tick();

        // Runtime checks if container finished
        if runtime.tick() {
            break;
        }

        if start.elapsed() > Duration::from_secs(60) {
            panic!("container did not complete within 60 seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // Verify CompleteMessage — the agent stores input in KV, reads it back
    let (complete, ack) = queue.receive_complete("cli").unwrap();
    assert_eq!(complete.correlation_id, invoke_id);
    assert_eq!(
        String::from_utf8(complete.payload).unwrap(),
        "bridge test data"
    );
    ack().unwrap();

    runtime.shutdown();
}
