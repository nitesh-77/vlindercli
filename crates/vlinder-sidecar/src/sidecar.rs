//! Sidecar — the agent's reality controller.
//!
//! Runs as a standalone binary inside a Podman pod, alongside the agent
//! container. Owns queue and registry connections, mediates all
//! communication between the agent container and the platform.

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vlinder_core::domain::{
    AgentId, CompleteMessage, ContainerDiagnostics, ContainerId, ContainerRuntimeInfo,
    ExpectsReply, HarnessType, ImageDigest, ImageRef, InvokeDiagnostics,
    InvokeMessage, MessageQueue, RoutingKey,
    RuntimeType, SequenceCounter,
};

use crate::queue_bridge::QueueBridge;
use crate::provider_server::ProviderServer;
use crate::config::SidecarConfig;
use crate::factory;

/// The sidecar process — mediates between the platform queue and the agent container.
pub struct Sidecar {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    bridge: Arc<QueueBridge>,
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
    /// A sync HTTP client for communicating with the agent container.
    http_client: ureq::Agent,
}

impl Sidecar {
    /// Create a new sidecar from env-var configuration.
    ///
    /// Connects to NATS (with DAG recording) and the Registry Service,
    /// then fetches the Agent from the registry to determine storage backends.
    pub fn new(config: &SidecarConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let queue = factory::connect_queue(&config.nats_url, &config.state_url)?;
        let registry = factory::connect_registry(&config.registry_url)?;

        let image_ref = config.image_ref.as_ref()
            .and_then(|r| ImageRef::parse(r).ok());
        let image_digest = config.image_digest.as_ref()
            .and_then(|d| ImageDigest::parse(d).ok());
        let container_id = config.container_id.as_ref()
            .map(|id| ContainerId::new(id))
            .unwrap_or_else(ContainerId::unknown);

        // A placeholder invoke message. This will be updated per-invocation.
        let placeholder_invoke = InvokeMessage::new(
            vlinder_core::domain::TimelineId::main(),
            vlinder_core::domain::SubmissionId::new(),
            vlinder_core::domain::SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            AgentId::new("placeholder"),
            Vec::new(),
            None,
            InvokeDiagnostics { harness_version: "".to_string(), history_turns: 0 },
        );

        let bridge = Arc::new(QueueBridge {
            queue: Arc::clone(&queue),
            registry: Arc::clone(&registry),
            invoke: std::sync::RwLock::new(placeholder_invoke),
            sequence: SequenceCounter::new(),
            pending_replies: std::sync::RwLock::new(std::collections::HashMap::new()),
        });

        Ok(Self {
            queue,
            bridge,
            container_port: config.container_port,
            agent_name: config.agent.clone(),
            image_ref,
            image_digest,
            container_id,
            http_client: ureq::Agent::new(),
        })
    }

    /// Set the current invocation context for the QueueBridge.
    fn set_context(&self, invoke: &InvokeMessage) {
        self.bridge.update_invoke(invoke.clone());
    }

    /// Route a CompleteMessage to the correct destination.
    fn send_reply(&self, complete: CompleteMessage, reply_key: &Option<RoutingKey>) {
        if let Some(ref key) = reply_key {
            self.queue.send_delegate_reply(complete, key).unwrap();
        } else {
            self.queue.send_complete(complete).unwrap();
        }
    }

    /// Handle a single invocation: set context, POST to agent, get final response.
    fn handle_invoke(
        &self,
        invoke: &InvokeMessage,
        reply_key: &Option<RoutingKey>,
    ) -> Result<(), String> {
        let started_at = Instant::now();
        self.set_context(invoke);

        // Spawn provider server for this invoke — drops when this method returns.
        let provider_server = ProviderServer::start(invoke);

        let agent_url = format!("http://127.0.0.1:{}/invoke", self.container_port);
        let payload = invoke.payload.clone();

        match self.http_client.post(&agent_url).send_bytes(&payload) {
            Ok(response) => {
                let mut output = Vec::new();
                response.into_reader().read_to_end(&mut output)
                    .map_err(|e| format!("Failed to read agent response body: {}", e))?;
                let final_state = provider_server.as_ref()
                    .and_then(|ps| ps.final_state());
                let duration_ms = started_at.elapsed().as_millis() as u64;
                let diagnostics = self.build_diagnostics(duration_ms);
                let complete = invoke.create_reply_with_diagnostics(output, final_state, diagnostics);
                self.send_reply(complete, reply_key);
                Ok(())
            }
            Err(ureq::Error::Status(code, response)) => {
                let err_body = response.into_string().unwrap_or_else(|_| "unknown error".to_string());
                tracing::warn!(
                    event = "container.error",
                    container = %self.container_id,
                    status = code,
                    reason = %err_body,
                    "Agent container returned an error"
                );
                let complete = invoke.create_reply(format!("[error] agent container error: {}", err_body).into_bytes());
                self.send_reply(complete, reply_key);
                Err(format!("Agent returned error: {}", err_body))
            }
            Err(e) => {
                let msg = format!("Request to agent failed: {}", e);
                tracing::warn!(event = "container.unreachable", error = %msg);
                let complete = invoke.create_reply(format!("[error] {}", msg).into_bytes());
                self.send_reply(complete, reply_key);
                Err(msg)
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
    fn wait_for_agent(&self) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/health", self.container_port);
        let deadline = Instant::now() + Duration::from_secs(60);

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

            match self.http_client.get(&url).call() {
                Ok(_) => {
                    tracing::info!(
                        event = "sidecar.agent_ready",
                        agent = %self.agent_name,
                        "Agent container is ready"
                    );
                    return Ok(());
                }
                _ => {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    /// Main loop: wait for agent, then poll invoke/delegate queues until container death.
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        self.wait_for_agent()
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

        tracing::info!(event = "sidecar.started", agent = %self.agent_name, "Sidecar loop started");

        loop {
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
            } else if let Ok((delegate, ack)) = self.queue.receive_delegate(&self.agent_name) {
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
            } else {
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        tracing::info!(event = "sidecar.stopped", agent = %self.agent_name, "Sidecar loop exited");
        Ok(())
    }
}
