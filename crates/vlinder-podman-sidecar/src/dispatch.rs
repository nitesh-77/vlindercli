//! Dispatch — handles a single agent invocation.
//!
//! Sets up the provider server, POSTs to the agent container, and builds
//! the CompleteMessage from the response.
//!
//! Durable agents (ADR 111) return JSON actions with X-Vlinder-Mode: durable.
//! The sidecar sends service requests to the queue and delivers responses
//! back to the agent via /invoke callbacks.

use std::io::Read;
use std::sync::Arc;
use std::time::Instant;

use vlinder_core::domain::{
    CompleteMessage, ContainerId, ExpectsReply, HealthWindow, HttpMethod, ImageDigest, ImageRef,
    InvokeMessage, MessageQueue, ProviderHost, ProviderRoute, Registry, RepairMessage,
    RequestDiagnostics, RequestMessage, ResponseMessage, RoutingKey, SequenceCounter,
};

use vlinder_provider_server::handler::InvokeHandler;
use vlinder_provider_server::hosts::build_hosts;
use vlinder_provider_server::provider_server::ProviderServer;

use crate::health;
use crate::trace::TraceLog;

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

/// State for an in-progress durable invocation waiting for a service response.
pub struct DurableSession {
    pub invoke: InvokeMessage,
    pub reply_key: Option<RoutingKey>,
    pub hosts: Vec<ProviderHost>,
    pub sequence: SequenceCounter,
    pub pending_request: RequestMessage,
    pub started_at: Instant,
}

/// Result of handling an invoke or a service response.
pub enum InvokeOutcome {
    /// Invocation fully handled — CompleteMessage sent.
    Done,
    /// Durable mode — waiting for a service response.
    Pending(DurableSession),
}

/// Handle a single invocation: POST to agent, detect mode, handle response.
pub fn handle_invoke(
    ctx: &DispatchContext,
    health: &mut HealthWindow,
    invoke: &InvokeMessage,
    reply_key: &Option<RoutingKey>,
) -> Result<InvokeOutcome, String> {
    let started_at = Instant::now();
    let mut trace = TraceLog::new();

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

    // Spawn provider server for unmanaged mode — drops when this function returns.
    let state = std::sync::Arc::new(std::sync::RwLock::new(initial_state));
    let handler = InvokeHandler::new(
        ctx.queue.clone(),
        ctx.registry.clone(),
        invoke.clone(),
        std::sync::Arc::clone(&state),
    );
    let provider_server = ProviderServer::start(handler, hosts, state, 3544);

    let client = ureq::Agent::new();
    let agent_url = format!("http://127.0.0.1:{}/invoke", ctx.container_port);
    let payload = invoke.payload.clone();

    trace.log(format!("POST {} ({} bytes)", agent_url, payload.len()));

    match client.post(&agent_url).send_bytes(&payload) {
        Ok(response) => {
            let is_durable = response
                .header("X-Vlinder-Mode")
                .map(|v| v == "durable")
                .unwrap_or(false);

            let mut output = Vec::new();
            response
                .into_reader()
                .read_to_end(&mut output)
                .map_err(|e| format!("Failed to read agent response body: {}", e))?;
            trace.log(format!(
                "Agent responded ({} bytes, {}ms)",
                output.len(),
                started_at.elapsed().as_millis()
            ));

            if is_durable {
                trace.log("Agent running in durable mode");
                // Provider server not needed for durable mode — drop it.
                drop(provider_server);

                // Build checkpoint hosts with :3544 suffix for URL matching.
                let checkpoint_hosts: Vec<ProviderHost> = build_hosts(&agent)
                    .into_iter()
                    .map(|mut h| {
                        h.hostname = format!("{}:{}", h.hostname, 3544);
                        h
                    })
                    .collect();

                let sequence = SequenceCounter::new();
                handle_action(
                    ctx,
                    output,
                    invoke,
                    reply_key,
                    checkpoint_hosts,
                    sequence,
                    started_at,
                )
            } else {
                // Unmanaged mode — response is the final output.
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
                trace.log("Sending complete");
                let complete =
                    invoke.create_reply_with_diagnostics(output, final_state, diagnostics);
                send_reply(&ctx.queue, complete, reply_key);
                Ok(InvokeOutcome::Done)
            }
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

/// Handle a service response arriving for a durable session.
///
/// Builds the callback JSON and POSTs it to the agent's /invoke endpoint.
pub fn handle_service_response(
    ctx: &DispatchContext,
    session: DurableSession,
    response: ResponseMessage,
) -> Result<InvokeOutcome, String> {
    let mut trace = TraceLog::new();

    let checkpoint = response
        .checkpoint
        .as_deref()
        .ok_or("service response missing checkpoint")?;

    trace.log(format!(
        "Service response for checkpoint '{}' ({} bytes)",
        checkpoint,
        response.payload.len()
    ));

    let result_json: serde_json::Value =
        serde_json::from_slice(&response.payload).unwrap_or(serde_json::Value::Null);

    let callback = serde_json::json!({
        "handler": checkpoint,
        "result": result_json,
    });

    let callback_bytes = serde_json::to_vec(&callback)
        .map_err(|e| format!("Failed to serialize callback: {}", e))?;

    let client = ureq::Agent::new();
    let agent_url = format!("http://127.0.0.1:{}/invoke", ctx.container_port);

    trace.log(format!(
        "POST {} callback ({} bytes)",
        agent_url,
        callback_bytes.len()
    ));

    match client
        .post(&agent_url)
        .set("Content-Type", "application/json")
        .send_bytes(&callback_bytes)
    {
        Ok(resp) => {
            let mut output = Vec::new();
            resp.into_reader()
                .read_to_end(&mut output)
                .map_err(|e| format!("Failed to read callback response: {}", e))?;
            trace.log(format!(
                "Callback responded ({} bytes, {}ms elapsed)",
                output.len(),
                session.started_at.elapsed().as_millis()
            ));

            handle_action(
                ctx,
                output,
                &session.invoke,
                &session.reply_key,
                session.hosts,
                session.sequence,
                session.started_at,
            )
        }
        Err(e) => {
            let msg = format!("Callback to agent failed: {}", e);
            trace.log(&msg);
            let complete = session
                .invoke
                .create_reply(format!("[error] {}", msg).into_bytes());
            send_reply(&ctx.queue, complete, &session.reply_key);
            Err(msg)
        }
    }
}

/// Handle a repair message: construct and send the service request, return
/// a durable session waiting for the response (ADR 113).
///
/// Skips the initial agent POST — the platform initiates the service call
/// directly. When the response arrives, `handle_service_response` delivers
/// it to the agent's checkpoint handler, re-entering the normal durable loop.
pub fn handle_repair(
    ctx: &DispatchContext,
    repair: &RepairMessage,
) -> Result<InvokeOutcome, String> {
    let mut trace = TraceLog::new();
    let started_at = Instant::now();

    let agent = ctx
        .registry
        .get_agent_by_name(repair.agent_id.as_str())
        .expect("agent not found");

    // Build checkpoint hosts with :3544 suffix (same as durable mode).
    let hosts: Vec<ProviderHost> = build_hosts(&agent)
        .into_iter()
        .map(|mut h| {
            h.hostname = format!("{}:{}", h.hostname, 3544);
            h
        })
        .collect();

    let diagnostics = RequestDiagnostics {
        sequence: repair.sequence.as_u32(),
        endpoint: format!("/{}", repair.service.service_type().as_str()),
        request_bytes: repair.payload.len() as u64,
        received_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    };

    let mut request = RequestMessage::new(
        repair.timeline.clone(),
        repair.submission.clone(),
        repair.session.clone(),
        repair.agent_id.clone(),
        repair.service,
        repair.operation,
        repair.sequence,
        repair.payload.clone(),
        repair.state.clone(),
        diagnostics,
    );
    request.checkpoint = Some(repair.checkpoint.clone());

    trace.log(format!(
        "Repair: sending request to {} (checkpoint '{}', seq {})",
        repair.service.service_type().as_str(),
        repair.checkpoint,
        repair.sequence.as_u32()
    ));

    ctx.queue
        .send_request(request.clone())
        .map_err(|e| format!("Failed to send repair request: {}", e))?;

    // Build a synthetic InvokeMessage so DurableSession can create
    // CompleteMessage replies when the agent finishes.
    let invoke = InvokeMessage::new(
        repair.timeline.clone(),
        repair.submission.clone(),
        repair.session.clone(),
        repair.harness,
        vlinder_core::domain::RuntimeType::Container,
        repair.agent_id.clone(),
        repair.payload.clone(),
        repair.state.clone(),
        vlinder_core::domain::InvokeDiagnostics {
            harness_version: env!("CARGO_PKG_VERSION").to_string(),
            history_turns: 0,
        },
        repair.dag_parent.clone(),
    );

    let sequence = SequenceCounter::new();
    // Advance past the repair's sequence so subsequent calls don't collide.
    for _ in 0..repair.sequence.as_u32() {
        sequence.next();
    }

    Ok(InvokeOutcome::Pending(DurableSession {
        invoke,
        reply_key: None,
        hosts,
        sequence,
        pending_request: request,
        started_at,
    }))
}

/// Parse and execute a JSON action from the agent.
fn handle_action(
    ctx: &DispatchContext,
    action_bytes: Vec<u8>,
    invoke: &InvokeMessage,
    reply_key: &Option<RoutingKey>,
    hosts: Vec<ProviderHost>,
    sequence: SequenceCounter,
    started_at: Instant,
) -> Result<InvokeOutcome, String> {
    let mut trace = TraceLog::new();

    let action: serde_json::Value = serde_json::from_slice(&action_bytes)
        .map_err(|e| format!("Failed to parse action: {}", e))?;

    let action_type = action["action"]
        .as_str()
        .ok_or("Missing 'action' field in agent response")?;

    trace.log(format!(
        "Action '{}' ({} bytes)",
        action_type,
        action_bytes.len()
    ));

    match action_type {
        "complete" => {
            let payload = action["payload"].as_str().unwrap_or("");
            trace.log(format!(
                "Durable complete ({} bytes, {}ms elapsed)",
                payload.len(),
                started_at.elapsed().as_millis()
            ));
            let complete = invoke.create_reply(payload.as_bytes().to_vec());
            send_reply(&ctx.queue, complete, reply_key);
            Ok(InvokeOutcome::Done)
        }
        "call" => {
            let url = action["url"]
                .as_str()
                .ok_or("Missing 'url' in call action")?;
            let checkpoint = action["then"]
                .as_str()
                .ok_or("Missing 'then' in call action")?;

            let call_body = serde_json::to_vec(&action["json"])
                .map_err(|e| format!("Failed to serialize call body: {}", e))?;

            let (host, path) = parse_url_host_path(url)?;
            let route = match_route(&hosts, &host, &path)
                .ok_or_else(|| format!("No route for {}:{}", host, path))?;

            let seq = sequence.next();
            let diagnostics = RequestDiagnostics {
                sequence: seq.as_u32(),
                endpoint: format!("/{}", route.service_backend.service_type().as_str()),
                request_bytes: call_body.len() as u64,
                received_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            };

            let mut request = RequestMessage::new(
                invoke.timeline.clone(),
                invoke.submission.clone(),
                invoke.session.clone(),
                invoke.agent_id.clone(),
                route.service_backend,
                route.operation,
                seq,
                call_body,
                None,
                diagnostics,
            );
            request.checkpoint = Some(checkpoint.to_string());

            trace.log(format!(
                "Sending request to {} (checkpoint '{}', seq {})",
                route.service_backend.service_type().as_str(),
                checkpoint,
                seq.as_u32()
            ));

            ctx.queue
                .send_request(request.clone())
                .map_err(|e| format!("Failed to send request: {}", e))?;

            Ok(InvokeOutcome::Pending(DurableSession {
                invoke: invoke.clone(),
                reply_key: reply_key.clone(),
                hosts,
                sequence,
                pending_request: request,
                started_at,
            }))
        }
        other => Err(format!("Unknown action: {}", other)),
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

/// Parse `http://host:port/path` into `("host:port", "/path")`.
fn parse_url_host_path(url: &str) -> Result<(String, String), String> {
    let without_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| format!("URL missing scheme: {}", url))?;
    match without_scheme.find('/') {
        Some(i) => Ok((
            without_scheme[..i].to_string(),
            without_scheme[i..].to_string(),
        )),
        None => Ok((without_scheme.to_string(), "/".to_string())),
    }
}

/// Find the route matching a host and path in the provider host table.
fn match_route<'a>(hosts: &'a [ProviderHost], host: &str, path: &str) -> Option<&'a ProviderRoute> {
    for vhost in hosts {
        if vhost.hostname == host {
            for route in &vhost.routes {
                if route.method == HttpMethod::Post && route.path == path {
                    return Some(route);
                }
            }
        }
    }
    None
}
