//! QueueBridge — queue-backed implementation of SdkContract (ADR 074, 076).
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

use super::{
    SdkContract, ObjectStorageType, Operation, Registry, ServiceType, VectorMatch, VectorStorageType,
    DelegateMessage, DelegateDiagnostics, ContainerDiagnostics, InvokeMessage,
    MessageQueue, RequestMessage, RequestDiagnostics, SequenceCounter,
};

use super::service_payloads::{
    KvGetRequest, KvPutRequest, KvListRequest, KvDeleteRequest,
    VectorStoreRequest, VectorSearchRequest, VectorDeleteRequest,
    InferRequest, EmbedRequest,
};

/// Routes agent SDK calls to the appropriate backend service.
///
/// Constructed once per container, shared across all service calls.
/// The invoke context is updated per invocation via `update_invoke()`.
pub struct QueueBridge {
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

impl QueueBridge {
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
    fn send_service_request(
        &self,
        service: ServiceType,
        backend: String,
        operation: Operation,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        let seq = self.sequence.next();
        let invoke = self.invoke.read().unwrap();
        let sha = invoke.submission.to_string();

        let received_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let request_diag = RequestDiagnostics {
            sequence: seq.as_u32(),
            endpoint: format!("/{}", service),
            request_bytes: payload.len() as u64,
            received_at_ms,
        };

        let state = self.current_state.read().unwrap().clone();
        let request = RequestMessage::new(
            invoke.submission.clone(),
            invoke.session.clone(),
            invoke.agent_id.clone(),
            service,
            backend,
            operation,
            seq,
            payload,
            state,
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
                    // Update state cursor from response envelope (ADR 079)
                    if let Some(ref state) = response.state {
                        *self.current_state.write().unwrap() = Some(state.clone());
                    }
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
// SdkContract trait implementation
// ============================================================================

impl SdkContract for QueueBridge {
    fn kv_get(&self, path: &str) -> Result<Vec<u8>, String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-get but has no object_storage configured")?;
        let state = self.current_state.read().unwrap().clone();
        let req = KvGetRequest { path: path.to_string(), state };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Kv, backend.as_str().to_string(), Operation::Get, payload)?;
        Self::check_worker_error(&response)?;
        Ok(response)
    }

    fn kv_put(&self, path: &str, content: &str) -> Result<(), String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-put but has no object_storage configured")?;
        let state = self.current_state.read().unwrap().clone();
        let req = KvPutRequest { path: path.to_string(), content: content.to_string(), state };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Kv, backend.as_str().to_string(), Operation::Put, payload)?;
        Self::check_worker_error(&response)?;
        Ok(())
    }

    fn kv_list(&self, prefix: &str) -> Result<Vec<String>, String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-list but has no object_storage configured")?;
        let req = KvListRequest { path: prefix.to_string() };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Kv, backend.as_str().to_string(), Operation::List, payload)?;
        Self::check_worker_error(&response)?;
        serde_json::from_slice(&response)
            .map_err(|e| format!("kv-list response parse error: {}", e))
    }

    fn kv_delete(&self, path: &str) -> Result<bool, String> {
        let backend = self.kv_backend
            .ok_or("agent called kv-delete but has no object_storage configured")?;
        let req = KvDeleteRequest { path: path.to_string() };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Kv, backend.as_str().to_string(), Operation::Delete, payload)?;
        Self::check_worker_error(&response)?;
        Ok(response == b"ok")
    }

    fn vector_store(&self, key: &str, vector: &[f32], metadata: &str) -> Result<(), String> {
        let backend = self.vec_backend
            .ok_or("agent called vector-store but has no vector_storage configured")?;
        let req = VectorStoreRequest { key: key.to_string(), vector: vector.to_vec(), metadata: metadata.to_string() };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Vec, backend.as_str().to_string(), Operation::Store, payload)?;
        Self::check_worker_error(&response)?;
        Ok(())
    }

    fn vector_search(&self, vector: &[f32], limit: u32) -> Result<Vec<VectorMatch>, String> {
        let backend = self.vec_backend
            .ok_or("agent called vector-search but has no vector_storage configured")?;
        let req = VectorSearchRequest { vector: vector.to_vec(), limit };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Vec, backend.as_str().to_string(), Operation::Search, payload)?;
        Self::check_worker_error(&response)?;
        serde_json::from_slice(&response)
            .map_err(|e| format!("vector-search response parse error: {}", e))
    }

    fn vector_delete(&self, key: &str) -> Result<bool, String> {
        let backend = self.vec_backend
            .ok_or("agent called vector-delete but has no vector_storage configured")?;
        let req = VectorDeleteRequest { key: key.to_string() };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Vec, backend.as_str().to_string(), Operation::Delete, payload)?;
        Self::check_worker_error(&response)?;
        Ok(response == b"ok")
    }

    fn infer(&self, model: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
        let backend = self.model_backends.get(model)
            .ok_or_else(|| format!("agent called infer with undeclared model '{}'", model))?;
        let req = InferRequest { model: model.to_string(), prompt: prompt.to_string(), max_tokens };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Infer, backend.clone(), Operation::Run, payload)?;
        Self::check_worker_error(&response)?;
        String::from_utf8(response)
            .map_err(|e| format!("infer response not valid UTF-8: {}", e))
    }

    fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, String> {
        let backend = self.model_backends.get(model)
            .ok_or_else(|| format!("agent called embed with undeclared model '{}'", model))?;
        let req = EmbedRequest { model: model.to_string(), text: text.to_string() };
        let payload = serde_json::to_vec(&req).map_err(|e| format!("serialize error: {}", e))?;
        let response = self.send_service_request(ServiceType::Embed, backend.clone(), Operation::Run, payload)?;
        Self::check_worker_error(&response)?;
        serde_json::from_slice(&response)
            .map_err(|e| format!("embed response parse error: {}", e))
    }

    fn delegate(&self, target_agent: &str, input: &str) -> Result<String, String> {
        let _agent = self.registry.get_agent_by_name(target_agent)
            .ok_or_else(|| format!("delegate: target agent '{}' not found", target_agent))?;

        let invoke = self.invoke.read().unwrap();
        let caller_agent = super::agent_routing_key(&invoke.agent_id);
        let sha = invoke.submission.to_string();
        let reply_subject = self.queue.create_reply_address(
            &invoke.submission, &caller_agent, target_agent,
        );

        let state = self.current_state.read().unwrap().clone();
        let delegate = DelegateMessage::new(
            invoke.submission.clone(),
            invoke.session.clone(),
            &caller_agent,
            target_agent,
            input.as_bytes().to_vec(),
            &reply_subject,
            state,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InMemoryQueue;
    use crate::registry::InMemoryRegistry;
    use crate::domain::{
        HarnessType, InvokeDiagnostics, RuntimeType, ResourceId, SessionId, SubmissionId,
        SecretStore,
    };
    use crate::secret_store::InMemorySecretStore;

    fn test_secret_store() -> Arc<dyn SecretStore> {
        Arc::new(InMemorySecretStore::new())
    }

    /// Build a QueueBridge wired to in-memory backends for unit testing.
    fn test_bridge(kv: Option<ObjectStorageType>) -> QueueBridge {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry: Arc<dyn Registry> = Arc::new(InMemoryRegistry::new(test_secret_store()));
        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            ResourceId::new("http://test/agents/echo"),
            b"hello".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );
        QueueBridge {
            queue,
            registry,
            invoke: RwLock::new(invoke),
            kv_backend: kv,
            vec_backend: None,
            model_backends: HashMap::new(),
            sequence: SequenceCounter::new(),
            current_state: RwLock::new(None),
        }
    }

    // ========================================================================
    // check_worker_error
    // ========================================================================

    #[test]
    fn check_worker_error_passes_normal_response() {
        assert!(QueueBridge::check_worker_error(b"some normal data").is_ok());
    }

    #[test]
    fn check_worker_error_detects_error_prefix() {
        let result = QueueBridge::check_worker_error(b"[error] something went wrong");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("something went wrong"));
    }

    #[test]
    fn check_worker_error_passes_empty() {
        assert!(QueueBridge::check_worker_error(b"").is_ok());
    }

    // ========================================================================
    // typed payload serialization
    // ========================================================================

    #[test]
    fn kv_get_request_includes_state() {
        use crate::domain::service_payloads::KvGetRequest;
        let req = KvGetRequest {
            path: "/notes".to_string(),
            state: Some("sha256:prev".to_string()),
        };
        let payload = serde_json::to_vec(&req).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&payload).unwrap();

        assert_eq!(parsed["path"], "/notes");
        assert_eq!(parsed["state"], "sha256:prev");
        assert!(parsed.get("op").is_none());
    }

    #[test]
    fn kv_get_request_omits_state_when_none() {
        use crate::domain::service_payloads::KvGetRequest;
        let req = KvGetRequest {
            path: "/x".to_string(),
            state: None,
        };
        let payload = serde_json::to_vec(&req).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&payload).unwrap();

        assert_eq!(parsed["path"], "/x");
        assert!(parsed.get("state").is_none());
        assert!(parsed.get("op").is_none());
    }

    // ========================================================================
    // update_invoke
    // ========================================================================

    #[test]
    fn update_invoke_with_state_sets_current_state() {
        let bridge = test_bridge(Some(ObjectStorageType::Sqlite));

        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            ResourceId::new("http://test/agents/echo"),
            b"new input".to_vec(),
            Some("sha256:explicit".to_string()),
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );

        bridge.update_invoke(invoke);
        assert_eq!(
            *bridge.current_state.read().unwrap(),
            Some("sha256:explicit".to_string()),
        );
    }

    #[test]
    fn update_invoke_bootstraps_empty_state_for_kv_agents() {
        let bridge = test_bridge(Some(ObjectStorageType::Sqlite));

        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            ResourceId::new("http://test/agents/echo"),
            b"input".to_vec(),
            None, // No state in invoke
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );

        bridge.update_invoke(invoke);
        // Agent has KV storage, so state bootstraps to empty string
        assert_eq!(
            *bridge.current_state.read().unwrap(),
            Some(String::new()),
        );
    }

    #[test]
    fn update_invoke_no_kv_no_state() {
        let bridge = test_bridge(None); // No KV backend

        let invoke = InvokeMessage::new(
            SubmissionId::new(),
            SessionId::new(),
            HarnessType::Cli,
            RuntimeType::Container,
            ResourceId::new("http://test/agents/echo"),
            b"input".to_vec(),
            None,
            InvokeDiagnostics { harness_version: String::new(), history_turns: 0 },
        );

        bridge.update_invoke(invoke);
        assert!(bridge.current_state.read().unwrap().is_none());
    }

    // ========================================================================
    // final_state
    // ========================================================================

    #[test]
    fn final_state_reads_current_state() {
        let bridge = test_bridge(None);
        assert!(bridge.final_state().is_none());

        *bridge.current_state.write().unwrap() = Some("sha256:final".to_string());
        assert_eq!(bridge.final_state(), Some("sha256:final".to_string()));
    }
}
