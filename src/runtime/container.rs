//! ContainerRuntime - executes OCI container agents in response to queue messages.
//!
//! The runtime:
//! - Discovers container agents from Registry
//! - Polls their input queues
//! - Executes Podman containers on message arrival
//! - Sends responses to reply queues
//!
//! Container agents receive their payload on stdin and write their
//! result to stdout. Service calls (HTTP bridge) are not yet supported.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::domain::{Registry, ResourceId, Runtime, RuntimeType};
use crate::queue::{ExpectsReply, InvokeMessage, MessageQueue};

/// Tracks a container execution running in a background thread.
struct RunningTask {
    handle: JoinHandle<Vec<u8>>,
    /// The original invoke message — used to construct CompleteMessage reply
    invoke: InvokeMessage,
}

pub struct ContainerRuntime {
    id: ResourceId,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    running: Option<RunningTask>,
}

impl ContainerRuntime {
    pub fn new(
        registry_id: &ResourceId,
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
    ) -> Self {
        let id = ResourceId::new(format!(
            "{}/runtimes/{}",
            registry_id.as_str(),
            RuntimeType::Container.as_str()
        ));
        Self {
            id,
            queue,
            registry,
            running: None,
        }
    }
}

impl Runtime for ContainerRuntime {
    fn id(&self) -> &ResourceId {
        &self.id
    }

    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Container
    }

    fn tick(&mut self) -> bool {
        // First, check if a running task completed
        if let Some(task) = self.running.take() {
            if task.handle.is_finished() {
                let output = task.handle.join().unwrap();
                let complete = task.invoke.create_reply(output);
                self.queue.send_complete(complete).unwrap();
                return true;
            } else {
                // Still running, put it back
                self.running = Some(task);
                return false;
            }
        }

        // Collect container agents from registry
        let container_agents: Vec<_> = self.registry.get_agents()
            .into_iter()
            .filter(|agent| self.registry.select_runtime(agent) == Some(RuntimeType::Container))
            .collect();

        // Check for work using typed receive (ADR 044)
        for agent in container_agents {
            if let Ok((invoke, ack)) = self.queue.receive_invoke(&agent.name) {
                // Extract container image reference from agent.id
                // container://localhost/echo-agent:latest → localhost/echo-agent:latest
                let image = agent.id.as_str()
                    .strip_prefix("container://")
                    .unwrap_or(agent.id.as_str())
                    .to_string();
                let payload = invoke.payload.clone();

                // ACK the message — we've taken ownership of processing
                let _ = ack();

                // Spawn container execution in background thread
                let handle = thread::spawn(move || {
                    run_container(&image, &payload)
                });

                self.running = Some(RunningTask { handle, invoke });
                return true;
            }
        }

        false
    }
}

/// Run a container image with payload on stdin, return stdout.
fn run_container(image: &str, payload: &[u8]) -> Vec<u8> {
    let mut child = Command::new("podman")
        .args(["run", "--rm", "-i", image])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn podman — is it installed?");

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(payload).expect("failed to write payload to container stdin");
        // stdin is dropped here, closing the pipe so the container sees EOF
    }

    let output = child.wait_with_output().expect("failed to wait on podman");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(image, %stderr, "container exited with error");
    }

    output.stdout
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::InMemoryRegistry;
    use crate::queue::InMemoryQueue;

    fn test_registry_id() -> ResourceId {
        ResourceId::new("http://test:9000")
    }

    fn test_registry() -> Arc<dyn Registry> {
        Arc::new(InMemoryRegistry::new())
    }

    #[test]
    fn runtime_id_format() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let runtime = ContainerRuntime::new(&test_registry_id(), queue, test_registry());

        assert_eq!(runtime.id().as_str(), "http://test:9000/runtimes/container");
        assert_eq!(runtime.runtime_type(), RuntimeType::Container);
    }

    #[test]
    fn tick_returns_false_when_no_agents() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let mut runtime = ContainerRuntime::new(&test_registry_id(), queue, test_registry());

        assert!(!runtime.tick());
    }
}
