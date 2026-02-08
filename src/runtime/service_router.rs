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

use crate::domain::{ObjectStorageType, SdkMessage, VectorStorageType};
use crate::queue::{InvokeMessage, MessageQueue, RequestMessage, SequenceCounter};

/// Routes agent SDK calls to the appropriate backend service.
///
/// Constructed once per container, shared across all service calls.
/// The invoke context is updated per invocation via `update_invoke()`.
pub(crate) struct ServiceRouter {
    pub(crate) queue: Arc<dyn MessageQueue + Send + Sync>,
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
    /// For kv-put and kv-get operations, injects the current state hash into
    /// the payload so the ObjectServiceWorker can perform versioned operations.
    pub(crate) fn dispatch(&self, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let msg: SdkMessage = serde_json::from_slice(&payload)
            .map_err(|e| format!("invalid SDK message: {}", e))?;

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
        let request = RequestMessage::new(
            invoke.submission.clone(),
            invoke.agent_id.clone(),
            hop.service,
            hop.backend,
            hop.operation,
            seq,
            request_payload,
        );
        drop(invoke);

        tracing::debug!(service = %request.service, backend = %request.backend, seq = %seq, "dispatch: sending request");

        self.queue.send_request(request.clone())
            .map_err(|e| format!("send error: {}", e))?;

        tracing::debug!(service = %request.service, seq = %seq, "dispatch: polling for response");
        let poll_start = std::time::Instant::now();
        let mut poll_count: u64 = 0;

        loop {
            match self.queue.receive_response(&request) {
                Ok((response, ack)) => {
                    let response_payload = response.payload.clone();
                    let _ = ack();
                    tracing::debug!(
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
}
