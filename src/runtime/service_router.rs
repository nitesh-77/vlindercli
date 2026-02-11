//! ServiceRouter — shared agent→service call handler.
//!
//! Used by ContainerRuntime (via HTTP bridge).
//! Contains the core logic: parse SdkMessage, resolve hop, build RequestMessage,
//! send to queue, poll for ResponseMessage, return payload.
//!
//! State tracking (ADR 055): The router maintains the current state hash for
//! the active invocation. For kv-put/kv-get operations, it injects the state
//! hash into the request payload. On kv-put responses, it extracts the new
//! state hash. This makes the router the "state cursor" for each invocation.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::domain::{ObjectStorageType, Registry, SdkMessage, VectorStorageType};
use crate::queue::{
    DelegateMessage, DelegateDiagnostics, ContainerDiagnostics, InvokeMessage,
    MessageQueue, RequestMessage, RequestDiagnostics, SequenceCounter,
};

/// Routes agent SDK calls to the appropriate backend service.
///
/// Constructed once per container, shared across all service calls.
/// The invoke context is updated per invocation via `update_invoke()`.
pub(crate) struct ServiceRouter {
    pub(crate) queue: Arc<dyn MessageQueue + Send + Sync>,
    pub(crate) registry: Arc<dyn Registry>,
    /// The invoke that triggered this execution — carries submission + agent_id.
    /// Updated per invocation so SDK calls route on the correct submission ID.
    pub(crate) invoke: RwLock<InvokeMessage>,
    /// Resolved backends from agent config (None if agent didn't declare storage)
    pub(crate) kv_backend: Option<ObjectStorageType>,
    pub(crate) vec_backend: Option<VectorStorageType>,
    /// Model name → backend string mapping (built from agent's declared models)
    pub(crate) model_backends: HashMap<String, String>,
    /// Sequence counter — incremented per service call, reset per invocation
    pub(crate) sequence: SequenceCounter,
    /// Current state hash for the active invocation (ADR 055).
    /// Updated on kv-put responses. Read by runtime on task completion.
    pub(crate) current_state: RwLock<Option<String>>,
}

impl ServiceRouter {
    /// Update the invoke context for a new invocation and reset the sequence counter.
    ///
    /// If the invoke carries no state but the agent has KV storage, bootstraps
    /// to root state ("") so versioned operations start tracking (ADR 055).
    pub(crate) fn update_invoke(&self, invoke: InvokeMessage) {
        let state = invoke.state.clone()
            .or_else(|| self.kv_backend.as_ref().map(|_| String::new()));
        *self.invoke.write().unwrap() = invoke;
        *self.current_state.write().unwrap() = state;
        self.sequence.reset();
    }

    /// Read the final state hash after an invocation completes.
    pub(crate) fn final_state(&self) -> Option<String> {
        self.current_state.read().unwrap().clone()
    }

    /// Dispatch a service call from the agent.
    ///
    /// Validates the payload, resolves the next hop, builds a typed request,
    /// sends it, and waits for the response. Returns the response payload.
    ///
    /// For delegate/wait operations (ADR 056), handles them directly without
    /// hop routing. For kv-put and kv-get operations, injects the current state
    /// hash into the payload so the ObjectServiceWorker can perform versioned operations.
    pub(crate) fn dispatch(&self, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let msg: SdkMessage = serde_json::from_slice(&payload)
            .map_err(|e| format!("invalid SDK message: {}", e))?;

        // Delegation operations are handled here, not via hop routing (ADR 056)
        match &msg {
            SdkMessage::Delegate { agent, input } => {
                let handle = self.delegate(agent, input)?;
                let json = serde_json::json!({ "handle": handle });
                return serde_json::to_vec(&json)
                    .map_err(|e| format!("delegate serialize error: {}", e));
            }
            SdkMessage::Wait { handle } => {
                let output = self.wait(handle)?;
                let json = serde_json::json!({ "output": String::from_utf8_lossy(&output) });
                return serde_json::to_vec(&json)
                    .map_err(|e| format!("wait serialize error: {}", e));
            }
            _ => {}
        }

        let hop = msg.hop(self.kv_backend, self.vec_backend, &self.model_backends)?;
        let is_kv = hop.service == "kv";
        let is_put = is_kv && hop.operation == "put";
        let seq = self.sequence.next();

        // For KV operations, inject current state hash into payload (ADR 055)
        let request_payload = if is_kv {
            self.inject_state(payload)?
        } else {
            payload
        };

        let invoke = self.invoke.read().unwrap();
        let sha = invoke.submission.to_string();

        let received_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let request_diag = RequestDiagnostics {
            sequence: seq.as_u32(),
            endpoint: format!("/{}", hop.service),
            request_bytes: request_payload.len() as u64,
            received_at_ms,
        };

        let request = RequestMessage::new(
            invoke.submission.clone(),
            invoke.session.clone(),
            invoke.agent_id.clone(),
            hop.service,
            hop.backend,
            hop.operation,
            seq,
            request_payload,
            request_diag,
        );
        drop(invoke);

        tracing::debug!(sha = %sha, event = "service.request", service = %request.service, backend = %request.backend, seq = %seq, "dispatch: sending request");

        self.queue.send_request(request.clone())
            .map_err(|e| format!("send error: {}", e))?;

        tracing::debug!(sha = %sha, event = "service.polling", service = %request.service, seq = %seq, "dispatch: polling for response");
        let poll_start = std::time::Instant::now();
        let mut poll_count: u64 = 0;

        loop {
            match self.queue.receive_response(&request) {
                Ok((response, ack)) => {
                    let response_payload = response.payload.clone();
                    let _ = ack();
                    tracing::debug!(
                        sha = %sha, event = "service.response",
                        service = %request.service, seq = %seq,
                        polls = poll_count, elapsed = ?poll_start.elapsed(),
                        "dispatch: got response"
                    );

                    // For kv-put responses, extract new state hash (ADR 055)
                    if is_put {
                        self.extract_state(&response_payload);
                    }

                    return Ok(response_payload);
                }
                Err(e) => {
                    poll_count += 1;
                    if poll_count % 5000 == 0 {
                        tracing::warn!(
                            sha = %sha,
                            service = %request.service, seq = %seq,
                            polls = poll_count, elapsed = ?poll_start.elapsed(),
                            error = %e,
                            "dispatch: still waiting for response"
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Inject the current state hash into a KV request payload.
    fn inject_state(&self, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let state = self.current_state.read().unwrap().clone();
        let Some(state_hash) = state else {
            return Ok(payload);
        };

        tracing::debug!(state = %state_hash, "inject_state: injecting into KV payload");
        let mut map: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&payload)
            .map_err(|e| format!("inject_state: invalid JSON: {}", e))?;
        map.insert("state".to_string(), serde_json::Value::String(state_hash));
        serde_json::to_vec(&map)
            .map_err(|e| format!("inject_state: serialize failed: {}", e))
    }

    /// Extract the new state hash from a kv-put response and update current_state.
    fn extract_state(&self, response_payload: &[u8]) {
        if let Ok(map) = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(response_payload) {
            if let Some(serde_json::Value::String(new_state)) = map.get("state") {
                tracing::debug!(new_state = %new_state, "extract_state: updated current_state");
                *self.current_state.write().unwrap() = Some(new_state.clone());
            }
        }
    }

    /// Delegate work to another agent (ADR 056, ADR 074).
    ///
    /// Validates the target agent exists, builds a unique reply subject,
    /// sends a DelegateMessage, and returns the handle string.
    pub(crate) fn delegate(&self, target_agent: &str, input: &str) -> Result<String, String> {
        // Verify target agent is registered
        let _agent = self.registry.get_agent_by_name(target_agent)
            .ok_or_else(|| format!("delegate: target agent '{}' not found", target_agent))?;

        let invoke = self.invoke.read().unwrap();
        let caller_agent = crate::queue::agent_routing_key(&invoke.agent_id);
        let sha = invoke.submission.to_string();
        let short_uuid = &Uuid::new_v4().to_string()[..8];
        let reply_subject = format!(
            "vlinder.{}.delegate-reply.{}.{}.{}",
            invoke.submission, caller_agent, target_agent, short_uuid,
        );

        let delegate = DelegateMessage::new(
            invoke.submission.clone(),
            invoke.session.clone(),
            &caller_agent,
            target_agent,
            input.as_bytes().to_vec(),
            &reply_subject,
            DelegateDiagnostics { container: ContainerDiagnostics::placeholder(0) },
        );
        drop(invoke);

        tracing::info!(
            sha = %sha, event = "delegation.sent",
            caller = %caller_agent, target = %target_agent,
            reply = %reply_subject, "Delegating to agent"
        );

        self.queue.send_delegate(delegate)
            .map_err(|e| format!("delegate send error: {}", e))?;

        Ok(reply_subject)
    }

    /// Wait for a delegated task to complete (ADR 056, ADR 074).
    ///
    /// Polls the reply subject until a CompleteMessage arrives, then returns
    /// the raw result payload.
    pub(crate) fn wait(&self, handle: &str) -> Result<Vec<u8>, String> {
        let sha = self.invoke.read().unwrap().submission.to_string();
        tracing::debug!(sha = %sha, handle = %handle, "wait: polling for delegation result");
        let poll_start = std::time::Instant::now();
        let mut poll_count: u64 = 0;

        loop {
            match self.queue.receive_complete_on_subject(handle) {
                Ok((complete, ack)) => {
                    let payload = complete.payload.clone();
                    let _ = ack();
                    tracing::info!(
                        sha = %sha, event = "delegation.completed",
                        handle = %handle, polls = poll_count,
                        elapsed = ?poll_start.elapsed(),
                        "Delegation result received"
                    );
                    return Ok(payload);
                }
                Err(_) => {
                    poll_count += 1;
                    if poll_count % 5000 == 0 {
                        tracing::warn!(
                            sha = %sha,
                            handle = %handle, polls = poll_count,
                            elapsed = ?poll_start.elapsed(),
                            "wait: still waiting for delegation result"
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
