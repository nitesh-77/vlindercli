//! Integration tests for ContainerRuntime (long-running model).
//!
//! Requires: podman installed + `just build-echo-container` + NATS running.
//! The sidecar creates its own queue from config, so these tests require
//! shared queue infrastructure (NATS) — in-memory queues won't work.
//!
//! Compile with: cargo test --features test-support

#![cfg(feature = "test-support")]

use vlinderd::config::Config;
use vlinder_core::domain::{
    Agent, AgentId, Runtime, RuntimeType,
    InvokeDiagnostics, InvokeMessage, HarnessType, SessionId, SubmissionId, TimelineId,
};
use vlinder_podman_runtime::{ContainerRuntime, PodmanRuntimeConfig};

#[test]
#[ignore] // Run via: just run-integration-tests
fn container_runtime_executes_echo_agent() {
    let config = Config::for_test();
    let registry = vlinderd::registry_factory::from_config(&config)
        .expect("Failed to create registry");
    let podman_config = PodmanRuntimeConfig {
        image_policy: config.runtime.image_policy.clone(),
        podman_socket: config.runtime.podman_socket.clone(),
        sidecar_image: config.runtime.sidecar_image.clone(),
        nats_url: config.queue.nats_url.clone(),
        registry_addr: config.distributed.registry_addr.clone(),
        state_addr: config.distributed.state_addr.clone(),
    };
    let mut runtime = ContainerRuntime::new(&podman_config, registry.clone()).unwrap();

    // Use the injected registry and a queue from config for test setup.
    // The sidecar creates its own queue — requires shared infra (NATS) to work.
    let queue = vlinderd::queue_factory::recording_from_config(&config).unwrap();

    registry.register_runtime(RuntimeType::Container);

    let agent = Agent::from_toml(r#"
        name = "echo-container"
        description = "Echo container agent"
        runtime = "container"
        executable = "localhost/echo-container:latest"
        [requirements]

    "#).unwrap();
    registry.register_agent(agent).unwrap();
    let agent_id = AgentId::new("echo-container");

    // Send InvokeMessage
    let submission = SubmissionId::new();
    let invoke = InvokeMessage::new(
        TimelineId::main(),
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

    // Tick until work completes
    let start = std::time::Instant::now();
    loop {
        runtime.tick();
        if start.elapsed() > std::time::Duration::from_secs(60) {
            panic!("container did not complete within 60 seconds");
        }
        // Check for completion
        if let Ok((complete, ack)) = queue.receive_complete(&submission, "cli") {
            assert_eq!(
                String::from_utf8(complete.payload).unwrap(),
                "hello from container"
            );
            ack().unwrap();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Explicit shutdown (stops containers)
    runtime.shutdown();
}
