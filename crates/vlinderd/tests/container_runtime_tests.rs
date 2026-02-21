//! Integration tests for ContainerRuntime (long-running model).
//!
//! Requires: podman installed + `just build-echo-container`
//! Compile with: cargo test --features test-support

#![cfg(feature = "test-support")]

use vlinderd::config::Config;
use vlinderd::domain::{
    Agent, AgentId, Runtime, RuntimeType,
    InvokeDiagnostics, InvokeMessage, HarnessType, SessionId, SubmissionId, TimelineId,
};
use vlinderd::runtime::ContainerRuntime;

#[test]
#[ignore] // Run via: just run-integration-tests
fn container_runtime_executes_echo_agent() {
    let mut runtime = ContainerRuntime::new(&Config::for_test()).unwrap();

    // Use the runtime's own queue and registry for test setup
    let queue = runtime.queue().clone();
    let registry = runtime.registry().clone();

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
