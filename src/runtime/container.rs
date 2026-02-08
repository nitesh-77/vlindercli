//! ContainerRuntime — executes OCI container agents as long-running processes.
//!
//! The runtime:
//! - Discovers container agents from Registry
//! - Lazily starts containers on first invocation (podman run -d)
//! - Dispatches work via HTTP POST /invoke
//! - Provides an HTTP bridge for service callbacks (kv, infer, etc.)
//! - Stops containers on shutdown

use std::collections::HashMap;
use std::io::Read;
use std::process::Command;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::domain::{Agent, ObjectStorageType, Registry, ResourceId, Runtime, RuntimeType, VectorStorageType};
use crate::queue::{ExpectsReply, HarnessType, InvokeMessage, MessageQueue, SequenceCounter};

use super::http_bridge::HttpBridge;
use super::service_router::ServiceRouter;

/// A long-running container managed by the runtime.
struct ManagedContainer {
    container_id: String,
    host_port: u16,
    bridge: HttpBridge,
}

/// Tracks an in-flight invocation dispatched to a container.
struct RunningTask {
    handle: JoinHandle<Vec<u8>>,
    invoke: InvokeMessage,
    /// For delegated work: the subject to send the result to.
    /// None for harness-invoked work (uses normal send_complete).
    reply_subject: Option<String>,
}

pub struct ContainerRuntime {
    id: ResourceId,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    running: HashMap<String, RunningTask>,
    /// Long-running containers keyed by agent name.
    containers: HashMap<String, ManagedContainer>,
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
            running: HashMap::new(),
            containers: HashMap::new(),
        }
    }

    /// Ensure a container is running for this agent. Starts one lazily if needed.
    fn ensure_container(&mut self, agent: &Agent, invoke: &InvokeMessage) -> Result<u16, String> {
        if let Some(mc) = self.containers.get(&agent.name) {
            mc.bridge.update_invoke(invoke.clone());
            return Ok(mc.host_port);
        }

        // Extract storage backends from agent config
        let kv_backend = agent.object_storage.as_ref()
            .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()));
        let vec_backend = agent.vector_storage.as_ref()
            .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()));

        // Build model→backend map from agent's declared models
        let mut model_backends = HashMap::new();
        for (model_alias, model_uri) in &agent.requirements.models {
            if let Some(model) = self.registry.get_model_by_path(model_uri) {
                model_backends.insert(model_alias.clone(), model.engine.as_backend_str().to_string());
            }
        }

        // Create ServiceRouter for the bridge.
        // Bootstrap state to root ("") if agent uses KV but no prior state exists (ADR 055).
        let initial_state = invoke.state.clone()
            .or_else(|| kv_backend.as_ref().map(|_| String::new()));
        let send_data = Arc::new(ServiceRouter {
            queue: Arc::clone(&self.queue),
            registry: Arc::clone(&self.registry),
            current_state: std::sync::RwLock::new(initial_state),
            invoke: std::sync::RwLock::new(invoke.clone()),
            kv_backend,
            vec_backend,
            model_backends,
            sequence: SequenceCounter::new(),
        });

        // Start bridge server
        let bridge = HttpBridge::start(send_data)
            .map_err(|e| format!("failed to start bridge: {}", e))?;
        let bridge_url = bridge.container_url();

        // Use the agent's executable directly — it's a native OCI image ref
        let image = &agent.executable;

        // Start container in detached mode with port mapping, bridge URL, and mounts
        let bridge_env = format!("VLINDER_BRIDGE_URL={}", bridge_url);
        let mut podman_args = vec![
            "run", "-d",
            "-p", ":8080",
            "-e", &bridge_env,
        ];

        // Add volume mounts from agent manifest (ADR 057)
        let mount_flags: Vec<String> = agent.mounts.iter().map(|m| {
            let mode = if m.readonly { "ro" } else { "rw" };
            format!("{}:{}:{}", m.host_path, m.guest_path.display(), mode)
        }).collect();
        for flag in &mount_flags {
            podman_args.push("-v");
            podman_args.push(flag);
        }

        podman_args.push(image);

        let output = Command::new("podman")
            .args(&podman_args)
            .output()
            .map_err(|e| format!("failed to spawn podman: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bridge.stop();
            return Err(format!("podman run failed: {}", stderr));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Discover the mapped host port
        let host_port = discover_host_port(&container_id)?;

        // Wait for container to be ready
        wait_for_ready(host_port)?;

        tracing::info!(event = "container.started", agent = %agent.name, container = %container_id, port = host_port, "Container started");

        self.containers.insert(agent.name.clone(), ManagedContainer {
            container_id,
            host_port,
            bridge,
        });

        Ok(host_port)
    }

}

impl Drop for ContainerRuntime {
    fn drop(&mut self) {
        self.shutdown();
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
        let mut did_work = false;

        // 1. Sweep completed tasks
        let finished: Vec<String> = self.running.iter()
            .filter(|(_, task)| task.handle.is_finished())
            .map(|(name, _)| name.clone())
            .collect();

        for name in finished {
            let task = self.running.remove(&name).unwrap();
            let output = task.handle.join().unwrap();

            // Read final state from the ServiceRouter (ADR 055)
            let final_state = self.containers.get(&name)
                .and_then(|mc| mc.bridge.final_state());

            let complete = task.invoke.create_reply_with_state(output, final_state);

            tracing::info!(
                event = "dispatch.completed",
                agent = %name,
                delegated = task.reply_subject.is_some(),
                "Task completed"
            );

            // Delegated work replies to the reply subject; invoked work replies to harness
            if let Some(ref reply_subject) = task.reply_subject {
                self.queue.send_complete_to_subject(complete, reply_subject).unwrap();
            } else {
                self.queue.send_complete(complete).unwrap();
            }
            did_work = true;
        }

        // 2. Dispatch new invoke work to idle agents
        let all_agents = self.registry.get_agents();
        let container_agents: Vec<_> = all_agents.iter()
            .filter(|a| self.registry.select_runtime(a) == Some(RuntimeType::Container))
            .collect();

        for agent in &container_agents {
            if self.running.contains_key(&agent.name) {
                continue; // agent already busy
            }

            let routing_key = agent.name.clone();
            if let Ok((invoke, ack)) = self.queue.receive_invoke(&routing_key) {
                let payload = invoke.payload.clone();
                let _ = ack();

                tracing::info!(
                    event = "dispatch.started",
                    sha = %invoke.submission,
                    session = %invoke.session,
                    agent = %agent.name,
                    "Dispatching to container"
                );

                let host_port = match self.ensure_container(agent, &invoke) {
                    Ok(port) => port,
                    Err(e) => {
                        tracing::error!(event = "dispatch.failed", agent = %agent.name, error = %e, "Failed to start container");
                        let complete = invoke.create_reply(
                            format!("[error] container start failed: {}", e).into_bytes()
                        );
                        self.queue.send_complete(complete).unwrap();
                        did_work = true;
                        continue;
                    }
                };

                let session_id = invoke.session.as_str().to_string();
                let handle = thread::spawn(move || {
                    dispatch_to_container(host_port, &payload, &session_id)
                });

                self.running.insert(agent.name.clone(), RunningTask {
                    handle, invoke, reply_subject: None,
                });
                did_work = true;
            }
        }

        // 3. Dispatch delegated work to idle agents (ADR 056)
        for agent in &container_agents {
            if self.running.contains_key(&agent.name) {
                continue; // agent already busy
            }

            if let Ok((delegate, ack)) = self.queue.receive_delegate(&agent.name) {
                let _ = ack();

                tracing::info!(
                    event = "delegation.received",
                    sha = %delegate.submission,
                    session = %delegate.session,
                    agent = %agent.name,
                    caller = %delegate.caller_agent,
                    "Dispatching delegated work"
                );

                // Build a synthetic InvokeMessage so the container sees a normal /invoke
                let invoke = InvokeMessage::new(
                    delegate.submission.clone(),
                    delegate.session.clone(),
                    HarnessType::Cli,  // placeholder — delegated work doesn't route to harness
                    RuntimeType::Container,
                    agent.id.clone(),
                    delegate.payload.clone(),
                    None,
                );

                let host_port = match self.ensure_container(agent, &invoke) {
                    Ok(port) => port,
                    Err(e) => {
                        tracing::error!(event = "dispatch.failed", agent = %agent.name, error = %e, "Failed to start container for delegation");
                        // Send error back to reply subject so the waiting agent unblocks
                        let complete = invoke.create_reply(
                            format!("[error] container start failed: {}", e).into_bytes()
                        );
                        self.queue.send_complete_to_subject(complete, &delegate.reply_subject).unwrap();
                        did_work = true;
                        continue;
                    }
                };

                let payload = delegate.payload;
                let session_id = delegate.session.as_str().to_string();
                let reply_subject = delegate.reply_subject;

                let handle = thread::spawn(move || {
                    dispatch_to_container(host_port, &payload, &session_id)
                });

                self.running.insert(agent.name.clone(), RunningTask {
                    handle, invoke, reply_subject: Some(reply_subject),
                });
                did_work = true;
            }
        }

        did_work
    }

    fn shutdown(&mut self) {
        for (name, mc) in self.containers.drain() {
            tracing::info!(event = "container.stopped", agent = %name, container = %mc.container_id, "Stopping container");
            let _ = Command::new("podman")
                .args(["stop", "-t", "5", &mc.container_id])
                .output();
            let _ = Command::new("podman")
                .args(["rm", "-f", &mc.container_id])
                .output();
            mc.bridge.stop();
        }
    }
}

/// Dispatch payload to a container's /invoke endpoint via HTTP POST.
///
/// Passes the session ID as X-Vlinder-Session header (ADR 054).
fn dispatch_to_container(host_port: u16, payload: &[u8], session_id: &str) -> Vec<u8> {
    let url = format!("http://127.0.0.1:{}/invoke", host_port);

    match ureq::post(&url)
        .set("X-Vlinder-Session", session_id)
        .send_bytes(payload)
    {
        Ok(response) => {
            let mut body = Vec::new();
            response.into_reader().read_to_end(&mut body).unwrap_or_default();
            body
        }
        Err(e) => {
            tracing::error!(event = "dispatch.failed", port = host_port, error = %e, "Failed to dispatch to container");
            format!("[error] dispatch failed: {}", e).into_bytes()
        }
    }
}

/// Discover the host port mapped to container port 8080.
fn discover_host_port(container_id: &str) -> Result<u16, String> {
    let output = Command::new("podman")
        .args(["port", container_id, "8080"])
        .output()
        .map_err(|e| format!("podman port failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("podman port failed: {}", stderr));
    }

    // Output format: "0.0.0.0:XXXXX\n"
    let port_str = String::from_utf8_lossy(&output.stdout);
    let port_str = port_str.trim();

    // Extract port after the last ':'
    let port = port_str
        .rsplit(':')
        .next()
        .ok_or_else(|| format!("unexpected podman port output: {}", port_str))?
        .parse::<u16>()
        .map_err(|e| format!("invalid port number: {}", e))?;

    Ok(port)
}

/// Wait for a container to become ready by polling its /health endpoint.
fn wait_for_ready(host_port: u16) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/health", host_port);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

    loop {
        if std::time::Instant::now() > deadline {
            return Err("container did not become ready within 30 seconds".to_string());
        }

        match ureq::get(&url).call() {
            Ok(_) => return Ok(()),
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
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
