//! ServiceRouter — queue-backed implementation of AgentBridge (ADR 074).
//!
//! Routes typed platform service calls through the MessageQueue.
//! Each trait method builds the appropriate request, sends it to the
//! queue, and polls for a response.
//!
//! State tracking (ADR 055): For KV operations, injects the current
//! state hash into requests and extracts the new hash from kv-put
//! responses. This makes the router the "state cursor" for each invocation.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::domain::{AgentBridge, Hop, ObjectStorageType, Registry, VectorMatch, VectorStorageType};
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

    // ========================================================================
    // Private helpers
    // ========================================================================

    /// Send a service request through the queue and poll for the response.
    fn send_service_request(&self, hop: Hop, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        let seq = self.sequence.next();
        let invoke = self.invoke.read().unwrap();
        let sha = invoke.submission.to_string();

        let received_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let request_diag = RequestDiagnostics {
            sequence: seq.as_u32(),
            endpoint: format!("/{}", hop.service),
            request_bytes: payload.len() as u64,
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
            payload,
            request_diag,
        );
        drop(invoke);

        tracing::debug!(sha = %sha, event = "service.request", service = %request.service, backend = %request.backend, seq = %seq, "sending service request");

        self.queue.send_request(request.clone())
            .map_err(|e| format!("send error: {}", e))?;

        tracing::debug!(sha = %sha, event = "service.polling", service = %request.service, seq = %seq, "polling for response");
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
                        "got response"
                    );
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
                            "still waiting for response"
                        );
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Build a KV request payload with state injection (ADR 055).
    fn build_kv_payload(&self, op: &str, fields: serde_json::Value) -> Result<Vec<u8>, String> {
        let mut map = match fields {
            serde_json::Value::Object(m) => m,
            _ => return Err("build_kv_payload: expected JSON object".to_string()),
        };
        map.insert("op".to_string(), serde_json::Value::String(op.to_string()));
        if let Some(ref state) = *self.current_state.read().unwrap() {
            map.insert("state".to_string(), serde_json::Value::String(state.clone()));
        }
        serde_json::to_vec(&map).map_err(|e| format!("serialize error: {}", e))
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

    /// Check for `[error]` prefix in a worker response.
    fn check_worker_error(response: &[u8]) -> Result<(), String> {
        if response.starts_with(b"[error]") {
            Err(String::from_utf8_lossy(response).to_string())
        } else {
            Ok(())
        }
    }
}

// ============================================================================
// AgentBridge trait implementation
// ============================================================================

impl AgentBridge for ServiceRouter {
    fn kv_get(&self, path: &str) -> Result<Vec<u8>, String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-get but has no object_storage configured")?;
        let hop = Hop { service: "kv", backend: backend.as_str().to_string(), operation: "get" };
        let payload = self.build_kv_payload("kv-get", serde_json::json!({"path": path}))?;
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        Ok(response)
    }

    fn kv_put(&self, path: &str, content: &str) -> Result<(), String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-put but has no object_storage configured")?;
        let hop = Hop { service: "kv", backend: backend.as_str().to_string(), operation: "put" };
        let payload = self.build_kv_payload("kv-put", serde_json::json!({"path": path, "content": content}))?;
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        self.extract_state(&response);
        Ok(())
    }

    fn kv_list(&self, prefix: &str) -> Result<Vec<String>, String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-list but has no object_storage configured")?;
        let hop = Hop { service: "kv", backend: backend.as_str().to_string(), operation: "list" };
        let payload = self.build_kv_payload("kv-list", serde_json::json!({"path": prefix}))?;
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        serde_json::from_slice(&response)
            .map_err(|e| format!("kv-list response parse error: {}", e))
    }

    fn kv_delete(&self, path: &str) -> Result<bool, String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-delete but has no object_storage configured")?;
        let hop = Hop { service: "kv", backend: backend.as_str().to_string(), operation: "delete" };
        let payload = self.build_kv_payload("kv-delete", serde_json::json!({"path": path}))?;
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        Ok(response == b"ok")
    }

    fn vector_store(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String> {
        let backend = self.vec_backend
            .ok_or("agent called vector-store but has no vector_storage configured")?;
        let hop = Hop { service: "vec", backend: backend.as_str().to_string(), operation: "store" };
        let payload = serde_json::to_vec(&serde_json::json!({
            "op": "vector-store", "key": key, "vector": vector, "metadata": metadata,
        })).unwrap();
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        Ok(())
    }

    fn vector_search(&self, vector: &[f32], limit: u32) -> Result<Vec<VectorMatch>, String> {
        let backend = self.vec_backend
            .ok_or("agent called vector-search but has no vector_storage configured")?;
        let hop = Hop { service: "vec", backend: backend.as_str().to_string(), operation: "search" };
        let payload = serde_json::to_vec(&serde_json::json!({
            "op": "vector-search", "vector": vector, "limit": limit,
        })).unwrap();
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        serde_json::from_slice(&response)
            .map_err(|e| format!("vector-search response parse error: {}", e))
    }

    fn vector_delete(&self, key: &str) -> Result<bool, String> {
        let backend = self.vec_backend
            .ok_or("agent called vector-delete but has no vector_storage configured")?;
        let hop = Hop { service: "vec", backend: backend.as_str().to_string(), operation: "delete" };
        let payload = serde_json::to_vec(&serde_json::json!({
            "op": "vector-delete", "key": key,
        })).unwrap();
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        Ok(response == b"ok")
    }

    fn infer(&self, model: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
        let backend = self.model_backends.get(model)
            .ok_or_else(|| format!("agent called infer with undeclared model '{}'", model))?;
        let hop = Hop { service: "infer", backend: backend.clone(), operation: "run" };
        let payload = serde_json::to_vec(&serde_json::json!({
            "op": "infer", "model": model, "prompt": prompt, "max_tokens": max_tokens,
        })).unwrap();
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        String::from_utf8(response)
            .map_err(|e| format!("infer response not valid UTF-8: {}", e))
    }

    fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, String> {
        let backend = self.model_backends.get(model)
            .ok_or_else(|| format!("agent called embed with undeclared model '{}'", model))?;
        let hop = Hop { service: "embed", backend: backend.clone(), operation: "run" };
        let payload = serde_json::to_vec(&serde_json::json!({
            "op": "embed", "model": model, "text": text,
        })).unwrap();
        let response = self.send_service_request(hop, payload)?;
        Self::check_worker_error(&response)?;
        serde_json::from_slice(&response)
            .map_err(|e| format!("embed response parse error: {}", e))
    }

    fn delegate(&self, target_agent: &str, input: &str) -> Result<String, String> {
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

    fn wait(&self, handle: &str) -> Result<Vec<u8>, String> {
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
