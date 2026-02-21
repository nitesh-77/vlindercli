//! Pod — the deployable unit managed by the pool.
//!
//! A Pod = Container + Sidecar. Container holds OCI lifecycle state,
//! Sidecar mediates between queue and container.

use std::sync::Arc;
use std::time::Instant;

use crate::config::Config;
use crate::domain::{Agent, AgentId, CompleteMessage, ContainerDiagnostics, ContainerId, ContainerRuntimeInfo, ExpectsReply, HarnessType, ImageDigest, ImageRef, InvokeDiagnostics, MessageQueue, ObjectStorageType, QueueBridge, Registry, RoutingKey, RuntimeType, InvokeMessage, SequenceCounter, VectorStorageType};

use super::dispatch::{DispatchError, dispatch_state_machine};

/// Container metadata — OCI lifecycle state for diagnostics.
pub(super) struct Container {
    pub(super) container_id: ContainerId,
    /// The OCI image reference (always `agent.executable` — identifies *which* image).
    pub(super) image_ref: ImageRef,
    /// Content-addressed digest from `podman image inspect` at container start.
    /// None if the inspect failed.
    pub(super) image_digest: Option<ImageDigest>,
}

/// The sidecar half of a Pod — the agent's reality controller.
///
/// Controls what the agent container sees: connections, state, invoke
/// context. The bridge is the current context — swapped per submission.
pub(super) struct Sidecar {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    kv_backend: Option<ObjectStorageType>,
    vec_backend: Option<VectorStorageType>,
    bridge: Option<Arc<QueueBridge>>,
}

impl Sidecar {
    pub(super) fn new(config: &Config, agent: &Agent) -> Result<Self, Box<dyn std::error::Error>> {
        let queue = crate::queue_factory::recording_from_config(config)?;
        let registry = crate::registry_factory::from_config(config)?;
        let kv_backend = agent.object_storage.as_ref()
            .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()));
        let vec_backend = agent.vector_storage.as_ref()
            .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()));
        Ok(Self { queue, registry, kv_backend, vec_backend, bridge: None })
    }

    /// Set the current context for this sidecar.
    pub(super) fn set_context(&mut self, invoke: &InvokeMessage) {
        let initial_state = invoke.state.clone()
            .or_else(|| self.kv_backend.as_ref().map(|_| String::new()));
        self.bridge = Some(Arc::new(QueueBridge {
            queue: Arc::clone(&self.queue),
            registry: Arc::clone(&self.registry),
            current_state: std::sync::RwLock::new(initial_state),
            invoke: std::sync::RwLock::new(invoke.clone()),
            kv_backend: self.kv_backend,
            vec_backend: self.vec_backend,
            sequence: SequenceCounter::new(),
            pending_replies: std::sync::RwLock::new(std::collections::HashMap::new()),
        }));
    }

    pub(super) fn bridge(&self) -> &Arc<QueueBridge> {
        self.bridge.as_ref().expect("bridge not set — call set_context first")
    }

    /// Route a CompleteMessage to the correct destination (harness or delegating agent).
    pub(super) fn send_reply(&self, complete: CompleteMessage, reply_key: &Option<RoutingKey>) {
        if let Some(ref key) = reply_key {
            self.queue.send_delegate_reply(complete, key).unwrap();
        } else {
            self.queue.send_complete(complete).unwrap();
        }
    }

    /// Handle a single invocation: set context → dispatch → send reply.
    ///
    /// Returns `Err(DispatchError::ContainerDead)` if the container is unreachable,
    /// signalling the caller to exit the loop so the pool can restart.
    pub(super) fn handle_invoke(
        &mut self,
        invoke: &InvokeMessage,
        host_port: u16,
        container: &Container,
        engine_version: &Option<semver::Version>,
        reply_key: &Option<RoutingKey>,
    ) -> Result<(), DispatchError> {
        let started_at = Instant::now();
        self.set_context(invoke);
        let bridge = Arc::clone(self.bridge());

        let payload = &invoke.payload;
        let session_id = invoke.session.as_str();

        match dispatch_state_machine(host_port, payload, session_id, bridge) {
            Ok(output) => {
                let final_state = self.bridge().final_state();
                let duration_ms = started_at.elapsed().as_millis() as u64;
                let diagnostics = Self::build_diagnostics(container, engine_version, duration_ms);
                let complete = invoke.create_reply_with_diagnostics(output, final_state, diagnostics);
                self.send_reply(complete, reply_key);
                Ok(())
            }
            Err(DispatchError::ContainerDead(ref reason)) => {
                tracing::warn!(
                    event = "container.dead",
                    container = %container.container_id,
                    reason = %reason,
                    "Container dead during dispatch"
                );
                let complete = invoke.create_reply(
                    format!("[error] container dead: {}", reason).into_bytes()
                );
                self.send_reply(complete, reply_key);
                Err(DispatchError::ContainerDead(reason.clone()))
            }
        }
    }

    /// Build ContainerDiagnostics from container metadata.
    fn build_diagnostics(
        container: &Container,
        engine_version: &Option<semver::Version>,
        duration_ms: u64,
    ) -> ContainerDiagnostics {
        ContainerDiagnostics {
            stderr: Vec::new(),
            runtime: ContainerRuntimeInfo {
                engine_version: engine_version.as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                image_ref: Some(container.image_ref.clone()),
                image_digest: container.image_digest.clone(),
                container_id: container.container_id.clone(),
            },
            duration_ms,
        }
    }

    /// Blocking loop: poll invoke/delegate queues, handle each, exit on container death.
    ///
    /// Takes ownership — when this returns, the sidecar thread is done.
    /// The pool detects the finished JoinHandle and cleans up.
    pub(super) fn run(
        mut self,
        agent_name: String,
        host_port: u16,
        container: Container,
        engine_version: Option<semver::Version>,
    ) {
        tracing::info!(event = "sidecar.started", agent = %agent_name, "Sidecar loop started");

        loop {
            // Poll invoke queue
            if let Ok((invoke, ack)) = self.queue.receive_invoke(&agent_name) {
                let _ = ack();
                tracing::info!(
                    event = "dispatch.started",
                    sha = %invoke.submission,
                    session = %invoke.session,
                    agent = %agent_name,
                    "Dispatching to container"
                );
                if self.handle_invoke(&invoke, host_port, &container, &engine_version, &None).is_err() {
                    break;
                }
                continue;
            }

            // Poll delegate queue
            if let Ok((delegate, ack)) = self.queue.receive_delegate(&agent_name) {
                let _ = ack();
                tracing::info!(
                    event = "delegation.received",
                    sha = %delegate.submission,
                    session = %delegate.session,
                    agent = %agent_name,
                    caller = %delegate.caller,
                    "Dispatching delegated work"
                );
                let invoke = InvokeMessage::new(
                    delegate.timeline.clone(),
                    delegate.submission.clone(),
                    delegate.session.clone(),
                    HarnessType::Cli,
                    RuntimeType::Container,
                    AgentId::new(&agent_name),
                    delegate.payload.clone(),
                    None,
                    InvokeDiagnostics {
                        harness_version: env!("CARGO_PKG_VERSION").to_string(),
                        history_turns: 0,
                    },
                );
                let reply_key = Some(delegate.reply_routing_key());
                if self.handle_invoke(&invoke, host_port, &container, &engine_version, &reply_key).is_err() {
                    break;
                }
                continue;
            }

            // Nothing to do — yield before polling again
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        tracing::info!(event = "sidecar.stopped", agent = %agent_name, "Sidecar loop exited");
    }
}

