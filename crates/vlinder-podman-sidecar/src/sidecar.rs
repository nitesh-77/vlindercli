//! Sidecar — the agent's reality controller.
//!
//! Runs as a standalone binary inside a Podman pod, alongside the agent
//! container. Owns queue and registry connections, mediates all
//! communication between the agent container and the platform.

use std::time::Duration;

use vlinder_core::domain::{
    AgentId, ContainerId, HarnessType, HealthWindow, ImageDigest, ImageRef, InvokeDiagnostics,
    InvokeMessage, RuntimeType,
};

use vlinder_provider_server::factory;

use crate::config::SidecarConfig;
use crate::dispatch::{self, DispatchContext, DurableSession, InvokeOutcome};
use crate::health;

/// The sidecar process — mediates between the platform queue and the agent container.
pub struct Sidecar {
    dispatch: DispatchContext,
    /// Agent name (queue subscription key).
    agent_name: String,
    /// Sliding window of agent health observations.
    health: HealthWindow,
}

impl Sidecar {
    /// Create a new sidecar from env-var configuration.
    ///
    /// Connects to NATS (with DAG recording) and the Registry Service,
    /// then fetches the Agent from the registry to determine storage backends.
    pub fn new(config: &SidecarConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let queue = factory::connect_queue(
            &config.nats_url,
            &config.state_url,
            config.secret_url.as_deref(),
        )?;
        let registry = factory::connect_registry(&config.registry_url)?;
        let image_ref = config
            .image_ref
            .as_ref()
            .and_then(|r| ImageRef::parse(r).ok());
        let image_digest = config
            .image_digest
            .as_ref()
            .and_then(|d| ImageDigest::parse(d).ok());
        let container_id = config
            .container_id
            .as_ref()
            .map(ContainerId::new)
            .unwrap_or_else(ContainerId::unknown);

        Ok(Self {
            dispatch: DispatchContext {
                queue,
                registry,
                container_port: config.container_port,
                container_id,
                image_ref,
                image_digest,
            },
            agent_name: config.agent.clone(),
            health: HealthWindow::new(60_000), // 60 second window
        })
    }

    /// Main loop: wait for agent, then poll invoke/delegate/response queues.
    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        health::wait_for_ready(
            &mut self.health,
            self.dispatch.container_port,
            &self.agent_name,
        )
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

        tracing::info!(event = "sidecar.started", agent = %self.agent_name, "Sidecar loop started");

        let mut durable_session: Option<DurableSession> = None;

        loop {
            let agent_id = AgentId::new(&self.agent_name);

            // Poll for service responses first (durable mode).
            if let Some(session) = durable_session.take() {
                match self
                    .dispatch
                    .queue
                    .receive_response(&session.pending_request)
                {
                    Ok((response, ack)) => {
                        let _ = ack();
                        match dispatch::handle_service_response(&self.dispatch, session, response) {
                            Ok(InvokeOutcome::Done) => {}
                            Ok(InvokeOutcome::Pending(next)) => {
                                durable_session = Some(next);
                            }
                            Err(e) => {
                                tracing::error!(
                                    event = "durable.handler_error",
                                    error = %e,
                                    agent = %self.agent_name,
                                    "Durable handler failed"
                                );
                                break;
                            }
                        }
                    }
                    Err(vlinder_core::domain::QueueError::Timeout) => {
                        // No response yet — put session back and continue polling.
                        durable_session = Some(session);
                    }
                    Err(e) => {
                        tracing::warn!(
                            event = "durable.response_error",
                            error = %e,
                            "Failed to receive service response"
                        );
                        break;
                    }
                }
            } else if let Ok((invoke, ack)) = self.dispatch.queue.receive_invoke(&agent_id) {
                let _ = ack();
                tracing::info!(
                    event = "dispatch.started",
                    sha = %invoke.submission,
                    session = %invoke.session,
                    agent = %self.agent_name,
                    "Dispatching to container"
                );
                match dispatch::handle_invoke(&self.dispatch, &mut self.health, &invoke, &None) {
                    Ok(InvokeOutcome::Done) => {}
                    Ok(InvokeOutcome::Pending(session)) => {
                        durable_session = Some(session);
                    }
                    Err(e) => {
                        tracing::error!(
                            event = "dispatch.error",
                            error = %e,
                            agent = %self.agent_name,
                            "Dispatch failed"
                        );
                        break;
                    }
                }
            } else if let Ok((delegate, ack)) = self.dispatch.queue.receive_delegate(&agent_id) {
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
                    String::new(),
                );
                let reply_key = Some(delegate.reply_routing_key());
                match dispatch::handle_invoke(&self.dispatch, &mut self.health, &invoke, &reply_key)
                {
                    Ok(InvokeOutcome::Done) => {}
                    Ok(InvokeOutcome::Pending(session)) => {
                        durable_session = Some(session);
                    }
                    Err(e) => {
                        tracing::error!(
                            event = "delegation.error",
                            error = %e,
                            agent = %self.agent_name,
                            "Delegation dispatch failed"
                        );
                        break;
                    }
                }
            } else if let Ok((repair, ack)) = self.dispatch.queue.receive_repair(&agent_id) {
                let _ = ack();
                tracing::info!(
                    event = "repair.received",
                    sha = %repair.submission,
                    session = %repair.session,
                    agent = %self.agent_name,
                    checkpoint = %repair.checkpoint,
                    "Dispatching repair"
                );
                match dispatch::handle_repair(&self.dispatch, &repair) {
                    Ok(InvokeOutcome::Done) => {}
                    Ok(InvokeOutcome::Pending(session)) => {
                        durable_session = Some(session);
                    }
                    Err(e) => {
                        tracing::error!(
                            event = "repair.error",
                            error = %e,
                            agent = %self.agent_name,
                            "Repair dispatch failed"
                        );
                        break;
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        tracing::info!(event = "sidecar.stopped", agent = %self.agent_name, "Sidecar loop exited");
        Ok(())
    }
}
