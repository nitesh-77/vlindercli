//! Sidecar — the agent's reality controller.
//!
//! Runs as a standalone binary inside a Podman pod, alongside the agent
//! container. Owns queue and registry connections, mediates all
//! communication between the agent container and the platform.

use std::sync::Arc;
use std::time::Instant;

use vlinder_core::domain::{
    AgentId, CompleteMessage, ContainerDiagnostics, ContainerId, ContainerRuntimeInfo,
    ExpectsReply, HarnessType, ImageDigest, ImageRef, InvokeDiagnostics, InvokeMessage,
    MessageQueue, ObjectStorageType, Registry, RoutingKey, RuntimeType,
    SequenceCounter, VectorStorageType,
};

use crate::queue_bridge::QueueBridge;

use crate::config::SidecarConfig;
use crate::dispatch::{dispatch_state_machine, DispatchError};
use crate::factory;

/// The sidecar process — mediates between the platform queue and the agent container.
pub struct Sidecar {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    kv_backend: Option<ObjectStorageType>,
    vec_backend: Option<VectorStorageType>,
    bridge: Option<Arc<QueueBridge>>,
    /// Agent container port (localhost inside the pod).
    container_port: u16,
    /// Agent name (queue subscription key).
    agent_name: String,
    /// OCI image reference (diagnostics).
    image_ref: Option<ImageRef>,
    /// Content-addressed digest (diagnostics).
    image_digest: Option<ImageDigest>,
    /// Container ID (diagnostics).
    container_id: ContainerId,
}

impl Sidecar {
    /// Create a new sidecar from env-var configuration.
    ///
    /// Connects to NATS (with DAG recording) and the Registry Service,
    /// then fetches the Agent from the registry to determine storage backends.
    pub fn new(config: &SidecarConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let queue = factory::connect_queue(&config.nats_url, &config.state_url)?;
        let registry = factory::connect_registry(&config.registry_url)?;

        let agent = registry.get_agent_by_name(&config.agent)
            .ok_or_else(|| format!("agent '{}' not found in registry", config.agent))?;

        let kv_backend = agent.object_storage.as_ref()
            .and_then(|uri| ObjectStorageType::from_scheme(uri.scheme()));
        let vec_backend = agent.vector_storage.as_ref()
            .and_then(|uri| VectorStorageType::from_scheme(uri.scheme()));

        let image_ref = config.image_ref.as_ref()
            .and_then(|r| ImageRef::parse(r).ok());
        let image_digest = config.image_digest.as_ref()
            .and_then(|d| ImageDigest::parse(d).ok());
        let container_id = config.container_id.as_ref()
            .map(|id| ContainerId::new(id))
            .unwrap_or_else(ContainerId::unknown);

        Ok(Self {
            queue,
            registry,
            kv_backend,
            vec_backend,
            bridge: None,
            container_port: config.container_port,
            agent_name: config.agent.clone(),
            image_ref,
            image_digest,
            container_id,
        })
    }

    /// Set the current invocation context.
    fn set_context(&mut self, invoke: &InvokeMessage) {
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

    fn bridge(&self) -> &Arc<QueueBridge> {
        self.bridge.as_ref().expect("bridge not set — call set_context first")
    }

    /// Route a CompleteMessage to the correct destination.
    fn send_reply(&self, complete: CompleteMessage, reply_key: &Option<RoutingKey>) {
        if let Some(ref key) = reply_key {
            self.queue.send_delegate_reply(complete, key).unwrap();
        } else {
            self.queue.send_complete(complete).unwrap();
        }
    }

    /// Handle a single invocation: set context, dispatch, send reply.
    fn handle_invoke(
        &mut self,
        invoke: &InvokeMessage,
        reply_key: &Option<RoutingKey>,
    ) -> Result<(), DispatchError> {
        let started_at = Instant::now();
        self.set_context(invoke);
        let bridge = Arc::clone(self.bridge());

        let payload = &invoke.payload;
        let session_id = invoke.session.as_str();

        match dispatch_state_machine(self.container_port, payload, session_id, bridge) {
            Ok(output) => {
                let final_state = self.bridge().final_state();
                let duration_ms = started_at.elapsed().as_millis() as u64;
                let diagnostics = self.build_diagnostics(duration_ms);
                let complete = invoke.create_reply_with_diagnostics(output, final_state, diagnostics);
                self.send_reply(complete, reply_key);
                Ok(())
            }
            Err(DispatchError::ContainerDead(ref reason)) => {
                tracing::warn!(
                    event = "container.dead",
                    container = %self.container_id,
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

    /// Build diagnostics from env-var metadata (not Podman inspect).
    fn build_diagnostics(&self, duration_ms: u64) -> ContainerDiagnostics {
        ContainerDiagnostics {
            stderr: Vec::new(),
            runtime: ContainerRuntimeInfo {
                engine_version: "sidecar".to_string(),
                image_ref: self.image_ref.clone(),
                image_digest: self.image_digest.clone(),
                container_id: self.container_id.clone(),
            },
            duration_ms,
        }
    }

    /// Wait for the agent container to become ready.
    ///
    /// Retries GET /health on localhost:{port} until the container responds
    /// or 60 seconds elapse. Inside a pod, the agent container shares the
    /// network namespace — localhost is the right address.
    fn wait_for_agent(&self) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/health", self.container_port);
        let deadline = Instant::now() + std::time::Duration::from_secs(60);

        tracing::info!(
            event = "sidecar.waiting",
            agent = %self.agent_name,
            port = self.container_port,
            "Waiting for agent container to become ready"
        );

        loop {
            if Instant::now() > deadline {
                return Err(format!(
                    "agent container did not become ready within 60 seconds (port {})",
                    self.container_port
                ));
            }

            match ureq::get(&url).call() {
                Ok(_) => {
                    tracing::info!(
                        event = "sidecar.agent_ready",
                        agent = %self.agent_name,
                        "Agent container is ready"
                    );
                    return Ok(());
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    }

    /// Main loop: wait for agent, then poll invoke/delegate queues until container death.
    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.wait_for_agent()
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

        tracing::info!(event = "sidecar.started", agent = %self.agent_name, "Sidecar loop started");

        loop {
            // Poll invoke queue
            if let Ok((invoke, ack)) = self.queue.receive_invoke(&self.agent_name) {
                let _ = ack();
                tracing::info!(
                    event = "dispatch.started",
                    sha = %invoke.submission,
                    session = %invoke.session,
                    agent = %self.agent_name,
                    "Dispatching to container"
                );
                if self.handle_invoke(&invoke, &None).is_err() {
                    break;
                }
                continue;
            }

            // Poll delegate queue
            if let Ok((delegate, ack)) = self.queue.receive_delegate(&self.agent_name) {
                let _ = ack();
                tracing::info!(
                    event = "delegation.received",
                    sha = %delegate.submission,
                    session = %delegate.session,
                    agent = %self.agent_name,
                    caller = %delegate.caller,
                    "Dispatching delegated work"
                );
                let invoke = InvokeMessage::new(
                    delegate.timeline.clone(),
                    delegate.submission.clone(),
                    delegate.session.clone(),
                    HarnessType::Cli,
                    RuntimeType::Container,
                    AgentId::new(&self.agent_name),
                    delegate.payload.clone(),
                    None,
                    InvokeDiagnostics {
                        harness_version: env!("CARGO_PKG_VERSION").to_string(),
                        history_turns: 0,
                    },
                );
                let reply_key = Some(delegate.reply_routing_key());
                if self.handle_invoke(&invoke, &reply_key).is_err() {
                    break;
                }
                continue;
            }

            // Nothing to do — yield before polling again
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        tracing::info!(event = "sidecar.stopped", agent = %self.agent_name, "Sidecar loop exited");
        Ok(())
    }
}
