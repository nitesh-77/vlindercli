//! Dispatch — handles a single agent invocation.
//!
//! Sets up the provider server, POSTs to the agent container, and builds
//! the CompleteMessage from the response. This is where durable execution
//! mode (ADR 111) will eventually branch.

use std::io::Read;
use std::sync::Arc;
use std::time::Instant;

use vlinder_core::domain::{
    CompleteMessage, ContainerId, ExpectsReply, HealthWindow, ImageDigest, ImageRef, InvokeMessage,
    MessageQueue, Registry, RoutingKey,
};

use vlinder_provider_server::hosts::build_hosts;
use vlinder_provider_server::provider_server::ProviderServer;

use crate::health;

/// Everything the dispatch loop needs from the sidecar — avoids passing
/// 9 individual parameters.
pub struct DispatchContext {
    pub queue: Arc<dyn MessageQueue + Send + Sync>,
    pub registry: Arc<dyn Registry>,
    pub container_port: u16,
    pub container_id: ContainerId,
    pub image_ref: Option<ImageRef>,
    pub image_digest: Option<ImageDigest>,
}

/// Handle a single invocation: set context, POST to agent, get final response.
pub fn handle_invoke(
    ctx: &DispatchContext,
    health: &mut HealthWindow,
    invoke: &InvokeMessage,
    reply_key: &Option<RoutingKey>,
) -> Result<(), String> {
    let started_at = Instant::now();

    // Look up agent to build provider hosts and determine initial state.
    let agent = ctx
        .registry
        .get_agent_by_name(invoke.agent_id.as_str())
        .expect("agent not found");
    let hosts = build_hosts(&agent);
    let initial_state = if agent.object_storage.is_some() {
        Some(invoke.state.clone().unwrap_or_default())
    } else {
        None
    };

    // Spawn provider server for this invoke — drops when this function returns.
    let provider_server = ProviderServer::start(
        invoke,
        hosts,
        ctx.queue.clone(),
        ctx.registry.clone(),
        initial_state,
        3544,
    );

    let client = ureq::Agent::new();
    let agent_url = format!("http://127.0.0.1:{}/invoke", ctx.container_port);
    let payload = invoke.payload.clone();

    match client.post(&agent_url).send_bytes(&payload) {
        Ok(response) => {
            let mut output = Vec::new();
            response
                .into_reader()
                .read_to_end(&mut output)
                .map_err(|e| format!("Failed to read agent response body: {}", e))?;
            let final_state = provider_server.final_state();
            let duration_ms = started_at.elapsed().as_millis() as u64;
            let diagnostics = health::build_diagnostics(
                health,
                ctx.container_port,
                duration_ms,
                &ctx.container_id,
                &ctx.image_ref,
                &ctx.image_digest,
            );
            let complete = invoke.create_reply_with_diagnostics(output, final_state, diagnostics);
            send_reply(&ctx.queue, complete, reply_key);
            Ok(())
        }
        Err(ureq::Error::Status(code, response)) => {
            let err_body = response
                .into_string()
                .unwrap_or_else(|_| "unknown error".to_string());
            tracing::warn!(
                event = "container.error",
                container = %ctx.container_id,
                status = code,
                reason = %err_body,
                "Agent container returned an error"
            );
            let complete = invoke
                .create_reply(format!("[error] agent container error: {}", err_body).into_bytes());
            send_reply(&ctx.queue, complete, reply_key);
            Err(format!("Agent returned error: {}", err_body))
        }
        Err(e) => {
            let msg = format!("Request to agent failed: {}", e);
            tracing::warn!(event = "container.unreachable", error = %msg);
            let complete = invoke.create_reply(format!("[error] {}", msg).into_bytes());
            send_reply(&ctx.queue, complete, reply_key);
            Err(msg)
        }
    }
}

/// Route a CompleteMessage to the correct destination.
fn send_reply(
    queue: &Arc<dyn MessageQueue + Send + Sync>,
    complete: CompleteMessage,
    reply_key: &Option<RoutingKey>,
) {
    if let Some(ref key) = reply_key {
        queue.send_delegate_reply(complete, key).unwrap();
    } else {
        queue.send_complete(complete).unwrap();
    }
}
