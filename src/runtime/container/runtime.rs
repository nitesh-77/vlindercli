//! ContainerRuntime — tick-loop orchestrator for OCI container agents.
//!
//! Polls queues for invoke and delegate work, dispatches to containers,
//! sweeps completed tasks, and handles retry on container death (ADR 073).

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use crate::domain::{Agent, ObjectStorageType, Registry, ResourceId, Runtime, RuntimeType, VectorStorageType};
use crate::queue::{
    ExpectsReply, HarnessType, InvokeDiagnostics, InvokeMessage, MessageQueue, SequenceCounter,
};

use super::dispatch::{DispatchError, RunningTask, dispatch_to_container};
use super::pool::{ContainerPool, ImagePolicy};
use crate::bridge::{HttpBridge, HttpBridgeServer};

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

    /// Build the HTTP bridge server for a new container.
    ///
    /// Needs `queue` + `registry` to construct the HttpBridge, which is why
    /// this lives on ContainerRuntime rather than ContainerPool.
    fn build_bridge(&self, agent: &Agent, invoke: &InvokeMessage) -> Result<HttpBridgeServer, String> {
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

        // Create HttpBridge for the server.
        // Bootstrap state to root ("") if agent uses KV but no prior state exists (ADR 055).
        let initial_state = invoke.state.clone()
            .or_else(|| kv_backend.as_ref().map(|_| String::new()));
        let send_data = Arc::new(HttpBridge {
            queue: Arc::clone(&self.queue),
            registry: Arc::clone(&self.registry),
            current_state: std::sync::RwLock::new(initial_state),
            invoke: std::sync::RwLock::new(invoke.clone()),
            kv_backend,
            vec_backend,
            model_backends,
            sequence: SequenceCounter::new(),
        });

        HttpBridgeServer::start(send_data)
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

    /// Dispatch an invocation to a container and track it as a running task.
    ///
    /// Ensures the container is running, spawns the HTTP dispatch on a thread,
    /// and inserts the RunningTask. On container-start failure, sends an error
    /// reply and returns false.
    fn dispatch(
        &mut self,
        name: &str,
        agent: &Agent,
        invoke: InvokeMessage,
        reply_subject: Option<String>,
        is_retry: bool,
    ) -> bool {
        let host_port = match self.ensure_container(agent, &invoke) {
            Ok(port) => port,
            Err(e) => {
                tracing::error!(event = "dispatch.failed", agent = %name, error = %e, "Failed to start container");
                let complete = invoke.create_reply(
                    format!("[error] container start failed: {}", e).into_bytes()
                );
                self.send_reply(complete, &reply_subject);
                return false;
            }
        };

        let payload = invoke.payload.clone();
        let session_id = invoke.session.as_str().to_string();
        let handle = thread::spawn(move || {
            dispatch_to_container(host_port, &payload, &session_id)
        });

        self.running.insert(name.to_string(), RunningTask {
            handle, invoke, reply_subject, started_at: Instant::now(), is_retry,
        });
        true
    }

    // ========================================================================
    // Tick phases
    // ========================================================================

    /// Join finished dispatch threads, handle results, retry on container death (ADR 073).
    fn sweep_completed(&mut self) -> bool {
        let finished: Vec<String> = self.running.iter()
            .filter(|(_, task)| task.handle.is_finished())
            .map(|(name, _)| name.clone())
            .collect();

        if finished.is_empty() {
            return false;
        }

        for name in finished {
            let task = self.running.remove(&name).unwrap();
            let result = task.handle.join().unwrap();

            match result {
                Ok(output) => {
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
                    // First failure — evict and retry once (ADR 073)
                    tracing::warn!(
                        event = "container.dead",
                        agent = %name,
                        reason = %reason,
                        "Dispatch failed — evicting and retrying"
                    );
                    self.pool.evict(&name);

                    let agent = self.registry.get_agents().into_iter()
                        .find(|a| a.name == name);

                    if let Some(agent) = agent {
                        self.dispatch(&name, &agent, task.invoke, task.reply_subject, true);
                    } else {
                        tracing::error!(event = "dispatch.agent_gone", agent = %name, "Agent not found after eviction");
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
        }

        true
    }

    /// Poll invoke queues for idle container agents, dispatch work.
    fn dispatch_invokes(&mut self) -> bool {
        let mut did_work = false;
        let all_agents = self.registry.get_agents();
        let container_agents: Vec<_> = all_agents.iter()
            .filter(|a| self.registry.select_runtime(a) == Some(RuntimeType::Container))
            .collect();

        for agent in &container_agents {
            if self.running.contains_key(&agent.name) {
                continue;
            }

            if let Ok((invoke, ack)) = self.queue.receive_invoke(&agent.name) {
                let _ = ack();

                tracing::info!(
                    event = "dispatch.started",
                    sha = %invoke.submission,
                    session = %invoke.session,
                    agent = %agent.name,
                    "Dispatching to container"
                );

                self.dispatch(&agent.name, agent, invoke, None, false);
                did_work = true;
            }
        }

        did_work
    }

    /// Poll delegate queues for idle container agents, dispatch delegated work (ADR 056).
    fn dispatch_delegates(&mut self) -> bool {
        let mut did_work = false;
        let all_agents = self.registry.get_agents();
        let container_agents: Vec<_> = all_agents.iter()
            .filter(|a| self.registry.select_runtime(a) == Some(RuntimeType::Container))
            .collect();

        for agent in &container_agents {
            if self.running.contains_key(&agent.name) {
                continue;
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

                self.dispatch(&agent.name, agent, invoke, Some(delegate.reply_subject), false);
                did_work = true;
            }
        }

        did_work
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
        did_work |= self.sweep_completed();
        did_work |= self.dispatch_invokes();
        did_work |= self.dispatch_delegates();
        did_work
    }

    fn shutdown(&mut self) {
        self.pool.shutdown();
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
