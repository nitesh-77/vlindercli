//! ContainerRuntime — executes OCI container agents as long-running processes.
//!
//! The runtime:
//! - Discovers container agents from Registry
//! - Lazily starts containers on first invocation (podman run -d)
//! - Dispatches work via HTTP POST /invoke
//! - Provides an HTTP bridge for service callbacks (kv, infer, etc.)
//! - Stops containers on shutdown

mod podman;
mod pool;

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::domain::{Agent, ObjectStorageType, Registry, ResourceId, Runtime, RuntimeType, VectorStorageType};
use crate::queue::{
    ExpectsReply, HarnessType, InvokeDiagnostics, InvokeMessage, MessageQueue, SequenceCounter,
};

use self::podman::{Podman, PodmanCli};
use self::pool::ContainerPool;
use super::http_bridge::HttpBridge;
use super::service_router::ServiceRouter;

pub use pool::ImagePolicy;

/// Tracks an in-flight invocation dispatched to a container.
struct RunningTask {
    handle: JoinHandle<Result<Vec<u8>, DispatchError>>,
    invoke: InvokeMessage,
    /// For delegated work: the subject to send the result to.
    /// None for harness-invoked work (uses normal send_complete).
    reply_subject: Option<String>,
    /// Wall-clock start time for duration measurement.
    started_at: Instant,
    /// True if this is a retry after dispatch-failure eviction (ADR 073).
    /// Prevents infinite retry loops — at most one retry per invocation.
    is_retry: bool,
}

/// Dispatch failure classification (ADR 073).
///
/// Transport errors (connection refused, timeout) indicate the container is dead.
/// HTTP status errors mean the container is alive — treated as successful dispatch.
enum DispatchError {
    /// The container is unreachable (connection refused, DNS failure, timeout).
    ContainerDead(String),
}

pub struct ContainerRuntime {
    id: ResourceId,
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    running: HashMap<String, RunningTask>,
    pool: ContainerPool,
}

impl ContainerRuntime {
    pub fn new(
        registry_id: &ResourceId,
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        image_policy: ImagePolicy,
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
            pool: ContainerPool::new(image_policy),
        }
    }

    /// Ensure a container is running for this agent. Starts one lazily if needed.
    fn ensure_container(&mut self, agent: &Agent, invoke: &InvokeMessage) -> Result<u16, String> {
        if let Some(port) = self.pool.get_port(&agent.name, invoke) {
            return Ok(port);
        }
        let bridge = self.build_bridge(agent, invoke)?;
        self.pool.start(&agent.name, agent, bridge)
    }

    /// Build the HTTP bridge + service router for a new container.
    ///
    /// Needs `queue` + `registry` to construct the ServiceRouter, which is why
    /// this lives on ContainerRuntime rather than ContainerPool.
    fn build_bridge(&self, agent: &Agent, invoke: &InvokeMessage) -> Result<HttpBridge, String> {
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

        HttpBridge::start(send_data)
            .map_err(|e| format!("failed to start bridge: {}", e))
    }

    /// Route a CompleteMessage to the correct destination (harness or delegating agent).
    fn send_reply(&self, complete: crate::queue::CompleteMessage, reply_subject: &Option<String>) {
        if let Some(ref subject) = reply_subject {
            self.queue.send_complete_to_subject(complete, subject).unwrap();
        } else {
            self.queue.send_complete(complete).unwrap();
        }
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
            let result = task.handle.join().unwrap();

            match result {
                Ok(output) => {
                    // Normal completion — build diagnostics and reply
                    let final_state = self.pool.final_state(&name);

                    let duration_ms = task.started_at.elapsed().as_millis() as u64;
                    let diagnostics = self.pool.diagnostics(&name, duration_ms);
                    let complete = task.invoke.create_reply_with_diagnostics(output, final_state, diagnostics);

                    tracing::info!(
                        event = "dispatch.completed",
                        agent = %name,
                        delegated = task.reply_subject.is_some(),
                        duration_ms = duration_ms,
                        "Task completed"
                    );

                    self.send_reply(complete, &task.reply_subject);
                }
                Err(DispatchError::ContainerDead(ref reason)) if !task.is_retry => {
                    // First failure — evict stale container and retry once (ADR 073)
                    tracing::warn!(
                        event = "container.dead",
                        agent = %name,
                        reason = %reason,
                        "Dispatch failed — evicting and retrying"
                    );

                    self.pool.evict(&name);

                    // Look up agent to restart container
                    let agent = self.registry.get_agents().into_iter()
                        .find(|a| a.name == name);

                    if let Some(agent) = agent {
                        match self.ensure_container(&agent, &task.invoke) {
                            Ok(host_port) => {
                                let payload = task.invoke.payload.clone();
                                let session_id = task.invoke.session.as_str().to_string();
                                let handle = thread::spawn(move || {
                                    dispatch_to_container(host_port, &payload, &session_id)
                                });
                                self.running.insert(name, RunningTask {
                                    handle,
                                    invoke: task.invoke,
                                    reply_subject: task.reply_subject,
                                    started_at: Instant::now(),
                                    is_retry: true,
                                });
                            }
                            Err(e) => {
                                tracing::error!(
                                    event = "dispatch.retry_failed",
                                    agent = %name,
                                    error = %e,
                                    "Container restart failed after eviction"
                                );
                                let complete = task.invoke.create_reply(
                                    format!("[error] container restart failed: {}", e).into_bytes()
                                );
                                self.send_reply(complete, &task.reply_subject);
                            }
                        }
                    } else {
                        tracing::error!(
                            event = "dispatch.agent_gone",
                            agent = %name,
                            "Agent not found in registry after eviction"
                        );
                        let complete = task.invoke.create_reply(
                            format!("[error] agent {} not found after container eviction", name).into_bytes()
                        );
                        self.send_reply(complete, &task.reply_subject);
                    }
                }
                Err(DispatchError::ContainerDead(reason)) => {
                    // Retry also failed — give up (ADR 073)
                    tracing::error!(
                        event = "dispatch.retry_exhausted",
                        agent = %name,
                        reason = %reason,
                        "Retry dispatch also failed — giving up"
                    );
                    self.pool.evict(&name);
                    let complete = task.invoke.create_reply(
                        format!("[error] container dead after retry: {}", reason).into_bytes()
                    );
                    self.send_reply(complete, &task.reply_subject);
                }
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
                        self.send_reply(complete, &None);
                        did_work = true;
                        continue;
                    }
                };

                let session_id = invoke.session.as_str().to_string();
                let handle = thread::spawn(move || {
                    dispatch_to_container(host_port, &payload, &session_id)
                });

                self.running.insert(agent.name.clone(), RunningTask {
                    handle, invoke, reply_subject: None, started_at: Instant::now(), is_retry: false,
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
                    InvokeDiagnostics {
                        harness_version: env!("CARGO_PKG_VERSION").to_string(),
                        history_turns: 0,
                    },
                );

                let reply_subject = Some(delegate.reply_subject.clone());
                let host_port = match self.ensure_container(agent, &invoke) {
                    Ok(port) => port,
                    Err(e) => {
                        tracing::error!(event = "dispatch.failed", agent = %agent.name, error = %e, "Failed to start container for delegation");
                        let complete = invoke.create_reply(
                            format!("[error] container start failed: {}", e).into_bytes()
                        );
                        self.send_reply(complete, &reply_subject);
                        did_work = true;
                        continue;
                    }
                };

                let payload = delegate.payload;
                let session_id = delegate.session.as_str().to_string();

                let handle = thread::spawn(move || {
                    dispatch_to_container(host_port, &payload, &session_id)
                });

                self.running.insert(agent.name.clone(), RunningTask {
                    handle, invoke, reply_subject: reply_subject.clone(), started_at: Instant::now(), is_retry: false,
                });
                did_work = true;
            }
        }

        did_work
    }

    fn shutdown(&mut self) {
        self.pool.shutdown();
    }
}

/// Dispatch payload to a container's /invoke endpoint via HTTP POST.
///
/// Passes the session ID as X-Vlinder-Session header (ADR 054).
///
/// Returns `Ok(body)` if the container responded (any HTTP status — the agent
/// is alive). Returns `Err(ContainerDead)` on transport errors (connection
/// refused, timeout) — the container is gone (ADR 073).
fn dispatch_to_container(host_port: u16, payload: &[u8], session_id: &str) -> Result<Vec<u8>, DispatchError> {
    let url = format!("http://127.0.0.1:{}/invoke", host_port);

    match ureq::post(&url)
        .set("X-Vlinder-Session", session_id)
        .send_bytes(payload)
    {
        Ok(response) => {
            let mut body = Vec::new();
            response.into_reader().read_to_end(&mut body).unwrap_or_default();
            Ok(body)
        }
        Err(ureq::Error::Status(code, response)) => {
            // Container is alive but returned an error status — pass the body through
            let mut body = Vec::new();
            response.into_reader().read_to_end(&mut body).unwrap_or_default();
            if body.is_empty() {
                body = format!("[error] agent returned HTTP {}", code).into_bytes();
            }
            Ok(body)
        }
        Err(ureq::Error::Transport(t)) => {
            // Container is dead — connection refused, DNS failure, timeout
            tracing::warn!(
                event = "dispatch.transport_error",
                port = host_port,
                error = %t,
                "Transport error — container likely dead"
            );
            Err(DispatchError::ContainerDead(t.to_string()))
        }
    }
}

/// Resolve the content-addressed digest for an image via `podman image inspect`.
/// Returns None if the inspect fails (image not found, Podman unavailable, etc.).
///
/// Thin delegation to `PodmanCli` — keeps the public API stable for `harness.rs`.
pub(crate) fn resolve_image_digest(image_ref: &str) -> Option<String> {
    PodmanCli.image_digest(image_ref)
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
        let runtime = ContainerRuntime::new(&test_registry_id(), queue, test_registry(), ImagePolicy::Mutable);

        assert_eq!(runtime.id().as_str(), "http://test:9000/runtimes/container");
        assert_eq!(runtime.runtime_type(), RuntimeType::Container);
    }

    #[test]
    fn tick_returns_false_when_no_agents() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let mut runtime = ContainerRuntime::new(&test_registry_id(), queue, test_registry(), ImagePolicy::Mutable);

        assert!(!runtime.tick());
    }
}
