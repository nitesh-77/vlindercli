//! Invoke handler — data-plane logic for provider and runtime requests.
//!
//! Owns queue routing, state tracking, and delegation. No HTTP knowledge —
//! the provider server calls into this after matching routes.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use vlinder_core::domain::{
    AgentName, DelegateDiagnostics, DelegateMessage, InvokeMessage, MessageQueue, Nonce,
    ProviderRoute, Registry, RequestDiagnostics, RequestMessage, RoutingKey, RuntimeDiagnostics,
    SequenceCounter,
};

/// Handles data-plane logic for a single invoke.
///
/// Created per invocation, lives for the duration of the invoke.
/// The provider server owns the HTTP plumbing; this struct owns
/// the queue interactions, state tracking, and delegation bookkeeping.
pub struct InvokeHandler {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    invoke: InvokeMessage,
    state: Arc<RwLock<Option<String>>>,
    sequence: SequenceCounter,
    pending_replies: HashMap<String, RoutingKey>,
}

impl InvokeHandler {
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        invoke: InvokeMessage,
        state: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            queue,
            registry,
            invoke,
            state,
            sequence: SequenceCounter::new(),
            pending_replies: HashMap::new(),
        }
    }

    /// Forward a matched provider request to the message queue.
    pub fn forward_provider(
        &self,
        route: &ProviderRoute,
        body: Vec<u8>,
        checkpoint: Option<String>,
    ) -> (u16, Vec<u8>) {
        let seq = self.sequence.next();
        let received_at_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);

        let diagnostics = RequestDiagnostics {
            sequence: seq.as_u32(),
            endpoint: format!("/{}", route.service_backend.service_type().as_str()),
            request_bytes: body.len() as u64,
            received_at_ms,
        };

        let mut request = RequestMessage::new(
            self.invoke.branch,
            self.invoke.submission.clone(),
            self.invoke.session.clone(),
            self.invoke.agent_id.clone(),
            route.service_backend,
            route.operation,
            seq,
            body,
            self.state.read().unwrap().clone(),
            diagnostics,
        );
        request.checkpoint = checkpoint;

        match self.queue.call_service(request) {
            Ok(response) => {
                if let Some(ref new_state) = response.state {
                    *self.state.write().unwrap() = Some(new_state.clone());
                }
                // Unwrap WireResponse envelope (ADR 118) — agent sees the HTTP body,
                // not the wire-format wrapper. Fall back to raw payload if not wrapped.
                if let Ok(wire) = serde_json::from_slice::<vlinder_core::domain::wire::WireResponse>(
                    &response.payload,
                ) {
                    (wire.inner.status().as_u16(), wire.inner.into_body())
                } else {
                    (response.status_code, response.payload.clone())
                }
            }
            Err(e) => (502, format!("queue error: {e}").into_bytes()),
        }
    }

    /// Handle a runtime request (delegate, wait).
    pub fn handle_runtime(&mut self, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
        if method == "POST" && path == "/delegate" {
            self.handle_delegate(body)
        } else if method == "POST" && path == "/wait" {
            self.handle_wait(body)
        } else {
            (404, b"not found".to_vec())
        }
    }

    fn handle_delegate(&mut self, body: &[u8]) -> (u16, Vec<u8>) {
        let parsed: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => return (400, format!("invalid JSON: {e}").into_bytes()),
        };

        let Some(target_name) = parsed.get("target").and_then(|v| v.as_str()) else {
            return (400, b"missing 'target' field".to_vec());
        };

        let Some(input) = parsed.get("input").and_then(|v| v.as_str()) else {
            return (400, b"missing 'input' field".to_vec());
        };

        if self.registry.get_agent_by_name(target_name).is_none() {
            return (
                404,
                format!("target agent '{target_name}' not found").into_bytes(),
            );
        }

        let caller = self.invoke.agent_id.clone();
        let target = AgentName::new(target_name);
        let nonce = Nonce::generate();

        let delegate = DelegateMessage::new(
            self.invoke.branch,
            self.invoke.submission.clone(),
            self.invoke.session.clone(),
            caller.clone(),
            target.clone(),
            input.as_bytes().to_vec(),
            nonce.clone(),
            None,
            DelegateDiagnostics {
                runtime: RuntimeDiagnostics::placeholder(0),
            },
        );

        let reply_key = delegate.reply_routing_key();
        let handle = nonce.as_str().to_string();

        tracing::info!(
            event = "delegation.sent",
            sha = %self.invoke.submission,
            caller = %caller, target = %target,
            nonce = %nonce, "Delegating to agent via provider server"
        );

        self.pending_replies.insert(handle.clone(), reply_key);

        if let Err(e) = self.queue.send_delegate(delegate) {
            return (502, format!("delegate send error: {e}").into_bytes());
        }

        let response = format!(r#"{{"handle":"{handle}"}}"#);
        (200, response.into_bytes())
    }

    fn handle_wait(&mut self, body: &[u8]) -> (u16, Vec<u8>) {
        let parsed: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => return (400, format!("invalid JSON: {e}").into_bytes()),
        };

        let handle = match parsed.get("handle").and_then(|v| v.as_str()) {
            Some(h) => h.to_string(),
            None => return (400, b"missing 'handle' field".to_vec()),
        };

        let reply_key = match self.pending_replies.get(&handle) {
            Some(k) => k.clone(),
            None => {
                return (
                    404,
                    format!("unknown delegation handle '{handle}'").into_bytes(),
                )
            }
        };

        tracing::debug!(handle = %handle, "wait: polling for delegation result");
        let poll_start = std::time::Instant::now();
        let mut poll_count: u64 = 0;

        loop {
            if let Ok((complete, ack)) = self.queue.receive_delegate_reply(&reply_key) {
                let payload = complete.payload.clone();
                let _ = ack();
                self.pending_replies.remove(&handle);
                tracing::info!(
                    event = "delegation.completed",
                    handle = %handle, polls = poll_count,
                    elapsed = ?poll_start.elapsed(),
                    "Delegation result received via provider server"
                );
                return (200, payload);
            }
            poll_count += 1;
            if poll_count.is_multiple_of(100) {
                tracing::warn!(
                    handle = %handle, polls = poll_count,
                    elapsed = ?poll_start.elapsed(),
                    "wait: still waiting for delegation result"
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
