//! Object Storage Service Handler - KV operations over queues.
//!
//! Queues:
//! - `kv-get`: Retrieve file content
//! - `kv-put`: Store file content
//! - `kv-list`: List files in path
//! - `kv-delete`: Delete file
//!
//! When a request payload contains a `"state"` field (ADR 055), the worker
//! performs versioned operations using a StateStore alongside the existing
//! ObjectStorage. The state field is the parent state commit hash.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use base64::Engine as _;

use crate::domain::registry::Registry;
use crate::domain::service_payloads::{KvGetRequest, KvPutRequest, KvListRequest, KvDeleteRequest};
use crate::domain::{ObjectStorage, ResourceId};
use crate::domain::{MessageQueue, Operation, RequestMessage, ResponseMessage, ServiceDiagnostics, ServiceType};
use crate::domain::{hash_snapshot, hash_state_commit, hash_value, StateStore};

/// Factory function that opens object storage from a URI.
pub type OpenObjectStorage = Box<dyn Fn(&ResourceId) -> Result<Arc<dyn ObjectStorage>, String> + Send + Sync>;

/// Factory function that opens a state store for an agent.
pub type OpenStateStore = Box<dyn Fn(&str) -> Result<Arc<dyn StateStore>, String> + Send + Sync>;

// ============================================================================
// Handler
// ============================================================================

pub struct ObjectServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    stores: RwLock<HashMap<String, Arc<dyn ObjectStorage>>>,
    state_stores: RwLock<HashMap<String, Arc<dyn StateStore>>>,
    backend: String,
    open_storage: OpenObjectStorage,
    open_state_store: OpenStateStore,
}

impl ObjectServiceWorker {
    /// Create a new object storage worker for a specific backend.
    ///
    /// The `open_storage` and `open_state_store` factories are called
    /// to lazy-load stores from agent metadata. Injected so the worker
    /// doesn't depend on concrete storage implementations.
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        backend: &str,
        open_storage: OpenObjectStorage,
        open_state_store: OpenStateStore,
    ) -> Self {
        Self {
            queue,
            registry,
            stores: RwLock::new(HashMap::new()),
            state_stores: RwLock::new(HashMap::new()),
            backend: backend.to_string(),
            open_storage,
            open_state_store,
        }
    }

    /// Get storage for an agent, opening lazily if needed.
    fn get_or_open(&self, agent_id: &str) -> Result<Arc<dyn ObjectStorage>, String> {
        // Check cache first
        if let Some(storage) = self.stores.read().unwrap().get(agent_id) {
            return Ok(storage.clone());
        }

        // Look up agent in Registry
        let agent = self.registry.get_agent_by_name(agent_id)
            .ok_or_else(|| format!("unknown agent: {}", agent_id))?;
        let uri = agent.object_storage
            .ok_or_else(|| format!("agent has no object_storage declared: {}", agent_id))?;

        // Open storage via injected factory
        let storage = (self.open_storage)(&uri)?;

        // Cache and return
        self.stores.write().unwrap().insert(agent_id.to_string(), storage.clone());
        Ok(storage)
    }

    /// Get or open a StateStore for an agent via the injected factory.
    fn get_or_open_state_store(&self, agent_id: &str) -> Result<Arc<dyn StateStore>, String> {
        if let Some(store) = self.state_stores.read().unwrap().get(agent_id) {
            return Ok(store.clone());
        }

        let store = (self.open_state_store)(agent_id)?;
        self.state_stores.write().unwrap().insert(agent_id.to_string(), store.clone());
        Ok(store)
    }

    /// Process one message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        if self.try_get() { return true; }
        if self.try_put() { return true; }
        if self.try_list() { return true; }
        if self.try_delete() { return true; }
        false
    }

    fn try_get(&self) -> bool {
        // Receive typed RequestMessage (ADR 044)
        match self.queue.receive_request(ServiceType::Kv, &self.backend, Operation::Get) {
            Ok((request, ack)) => {
                let start = std::time::Instant::now();
                let response_payload = self.handle_get(&request);
                let duration_ms = start.elapsed().as_millis() as u64;
                let diag = ServiceDiagnostics::storage(
                    ServiceType::Kv, &self.backend, Operation::Get, response_payload.len() as u64, duration_ms,
                );
                let mut response = ResponseMessage::from_request_with_diagnostics(
                    &request, response_payload, diag,
                );
                response.state = request.state.clone();
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_put(&self) -> bool {
        match self.queue.receive_request(ServiceType::Kv, &self.backend, Operation::Put) {
            Ok((request, ack)) => {
                let start = std::time::Instant::now();
                let response_payload = self.handle_put(&request);
                let duration_ms = start.elapsed().as_millis() as u64;
                let diag = ServiceDiagnostics::storage(
                    ServiceType::Kv, &self.backend, Operation::Put, response_payload.len() as u64, duration_ms,
                );
                let mut response = ResponseMessage::from_request_with_diagnostics(
                    &request, response_payload.clone(), diag,
                );
                // Extract new state hash from versioned put response (ADR 055).
                // Falls back to echoing request.state for unversioned puts.
                response.state = extract_state_from_payload(&response_payload)
                    .or_else(|| request.state.clone());
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_list(&self) -> bool {
        match self.queue.receive_request(ServiceType::Kv, &self.backend, Operation::List) {
            Ok((request, ack)) => {
                let start = std::time::Instant::now();
                let response_payload = self.handle_list(&request);
                let duration_ms = start.elapsed().as_millis() as u64;
                let diag = ServiceDiagnostics::storage(
                    ServiceType::Kv, &self.backend, Operation::List, response_payload.len() as u64, duration_ms,
                );
                let mut response = ResponseMessage::from_request_with_diagnostics(
                    &request, response_payload, diag,
                );
                response.state = request.state.clone();
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_delete(&self) -> bool {
        match self.queue.receive_request(ServiceType::Kv, &self.backend, Operation::Delete) {
            Ok((request, ack)) => {
                let start = std::time::Instant::now();
                let response_payload = self.handle_delete(&request);
                let duration_ms = start.elapsed().as_millis() as u64;
                let diag = ServiceDiagnostics::storage(
                    ServiceType::Kv, &self.backend, Operation::Delete, response_payload.len() as u64, duration_ms,
                );
                let mut response = ResponseMessage::from_request_with_diagnostics(
                    &request, response_payload, diag,
                );
                response.state = request.state.clone();
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn handle_get(&self, request: &RequestMessage) -> Vec<u8> {
        let req: KvGetRequest = match serde_json::from_slice(request.payload.legacy_bytes()) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        // Versioned get (ADR 055): resolve through state commit → snapshot → value
        // Empty state hash means "no state yet" — fall through to unversioned.
        if let Some(ref state_hash) = req.state {
            if !state_hash.is_empty() {
                return match self.versioned_get(request.agent_id.as_str(), state_hash, &req.path) {
                    Ok(Some(content)) => content,
                    Ok(None) => Vec::new(),
                    Err(e) => format!("[error] {}", e).into_bytes(),
                };
            }
        }

        // Unversioned: existing behavior
        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        match store.get_file(&req.path) {
            Ok(Some(content)) => content,
            Ok(None) => Vec::new(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_put(&self, request: &RequestMessage) -> Vec<u8> {
        let req: KvPutRequest = match serde_json::from_slice(request.payload.legacy_bytes()) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Decode base64 (protocol concern)
        let content = match base64::engine::general_purpose::STANDARD.decode(&req.content) {
            Ok(c) => c,
            Err(e) => return format!("[error] invalid base64: {}", e).into_bytes(),
        };

        // Always write to ObjectStorage (current-state access for unversioned reads)
        if let Err(e) = store.put_file(&req.path, &content) {
            return format!("[error] {}", e).into_bytes();
        }

        // Versioned put (ADR 055): compute hashes, store in state store, return new state hash
        if let Some(ref parent_state) = req.state {
            return match self.versioned_put(request.agent_id.as_str(), parent_state, &req.path, &content) {
                Ok(new_state) => {
                    let response = serde_json::json!({"state": new_state});
                    serde_json::to_vec(&response).unwrap()
                }
                Err(e) => format!("[error] {}", e).into_bytes(),
            };
        }

        // Unversioned: existing behavior
        b"ok".to_vec()
    }

    fn handle_list(&self, request: &RequestMessage) -> Vec<u8> {
        let req: KvListRequest = match serde_json::from_slice(request.payload.legacy_bytes()) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        match store.list_files(&req.path) {
            Ok(files) => {
                serde_json::to_string(&files)
                    .map(|s| s.into_bytes())
                    .unwrap_or_else(|e| format!("[error] {}", e).into_bytes())
            }
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_delete(&self, request: &RequestMessage) -> Vec<u8> {
        let req: KvDeleteRequest = match serde_json::from_slice(request.payload.legacy_bytes()) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        match store.delete_file(&req.path) {
            Ok(true) => b"ok".to_vec(),
            Ok(false) => b"not_found".to_vec(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    // --- Versioned operations (ADR 055) ---

    /// Versioned put: store value, update snapshot, create state commit.
    fn versioned_put(
        &self,
        agent_id: &str,
        parent_state: &str,
        path: &str,
        content: &[u8],
    ) -> Result<String, String> {
        let state_store = self.get_or_open_state_store(agent_id)?;

        // 1. Store value
        let value_hash = hash_value(content);
        state_store.put_value(&value_hash, content)?;

        // 2. Load parent snapshot (or start from empty)
        let parent_entries = if parent_state.is_empty() {
            HashMap::new()
        } else {
            let commit = state_store.get_state_commit(parent_state)?
                .ok_or_else(|| format!("unknown parent state: {}", parent_state))?;
            state_store.get_snapshot(&commit.snapshot_hash)?
                .unwrap_or_default()
        };

        // 3. Update snapshot with new path → value_hash
        let mut new_entries = parent_entries;
        new_entries.insert(path.to_string(), value_hash);

        // 4. Store snapshot
        let snapshot_hash = hash_snapshot(&new_entries);
        state_store.put_snapshot(&snapshot_hash, &new_entries)?;

        // 5. Create and store state commit
        let commit_hash = hash_state_commit(&snapshot_hash, parent_state);
        state_store.put_state_commit(&commit_hash, &snapshot_hash, parent_state)?;

        Ok(commit_hash)
    }

    /// Versioned get: resolve through state commit → snapshot → value.
    fn versioned_get(
        &self,
        agent_id: &str,
        state_hash: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        let state_store = self.get_or_open_state_store(agent_id)?;

        // Load state commit
        let commit = state_store.get_state_commit(state_hash)?
            .ok_or_else(|| format!("unknown state: {}", state_hash))?;

        // Load snapshot
        let entries = state_store.get_snapshot(&commit.snapshot_hash)?
            .unwrap_or_default();

        // Look up path
        let value_hash = match entries.get(path) {
            Some(h) => h,
            None => return Ok(None),
        };

        // Load value
        state_store.get_value(value_hash)
    }
}

/// Extract the state hash from a versioned kv-put response payload.
///
/// Versioned put responses are JSON: `{"state": "hash"}`.
/// Returns None for unversioned responses (plain `b"ok"`).
fn extract_state_from_payload(payload: &[u8]) -> Option<String> {
    let map: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(payload).ok()?;
    match map.get("state") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Agent, AgentId};
    use crate::domain::InMemoryRegistry;
    use crate::domain::{ObjectStorageType, Operation, RequestDiagnostics, Sequence, ServiceBackend, SessionId, SubmissionId, TimelineId};
    use crate::domain::SecretStore;
    use crate::domain::InMemorySecretStore;
    use crate::queue::InMemoryQueue;

    fn test_secret_store() -> Arc<dyn SecretStore> {
        Arc::new(InMemorySecretStore::new())
    }

    fn test_agent_id() -> AgentId {
        AgentId::new("test-agent")
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-123".to_string())
    }

    fn test_session() -> SessionId {
        SessionId::new()
    }

    fn test_request_diag() -> RequestDiagnostics {
        RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 }
    }

    fn test_agent_with_object_storage() -> Agent {
        let manifest = r#"
            name = "test-agent"
            description = "Test agent for object storage"
            runtime = "container"
            executable = "localhost/test-agent:latest"
            object_storage = "memory://"
            [requirements]

        "#;
        Agent::from_toml(manifest).unwrap()
    }

    fn test_open_storage() -> OpenObjectStorage {
        Box::new(|_uri: &ResourceId| {
            Ok(Arc::new(crate::domain::InMemoryObjectStorage::new()) as Arc<dyn ObjectStorage>)
        })
    }

    fn test_open_state_store() -> OpenStateStore {
        Box::new(|_agent_id: &str| {
            Ok(Arc::new(crate::domain::InMemoryStateStore::new()) as Arc<dyn StateStore>)
        })
    }

    #[test]
    fn handles_put_and_get() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);

        // Register test agent with memory:// object storage
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_storage(), test_open_state_store());

        // Send typed put request (ADR 044)
        let put_payload = serde_json::json!({
            "path": "/hello.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"hello world")
        });
        let put_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            test_session(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory),
            Operation::Put,
            Sequence::first(),
            serde_json::to_vec(&put_payload).unwrap(),
            None,
            test_request_diag(),
        );

        queue.send_request(put_request.clone()).unwrap();

        // Process
        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&put_request).unwrap();
        assert_eq!(response.payload.legacy_bytes(), b"ok");
        ack().unwrap();

        // Send typed get request
        let get_payload = serde_json::json!({
            "path": "/hello.txt"
        });
        let get_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            test_session(),
            test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory),
            Operation::Get,
            Sequence::from(2),
            serde_json::to_vec(&get_payload).unwrap(),
            None,
            test_request_diag(),
        );

        queue.send_request(get_request.clone()).unwrap();

        // Process
        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&get_request).unwrap();
        assert_eq!(response.payload.legacy_bytes(), b"hello world");
        ack().unwrap();
    }

    #[test]
    fn versioned_put_returns_state_hash() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_storage(), test_open_state_store());

        let put_payload = serde_json::json!({
            "path": "/todos.json",
            "content": base64::engine::general_purpose::STANDARD.encode(b"[\"buy milk\"]"),
            "state": ""  // root state
        });
        let put_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Put, Sequence::first(),
            serde_json::to_vec(&put_payload).unwrap(),
            None,
            test_request_diag(),
        );

        queue.send_request(put_request.clone()).unwrap();
        assert!(handler.tick());

        let (response, ack) = queue.receive_response(&put_request).unwrap();
        ack().unwrap();

        // Response should be JSON with a state field
        let resp: serde_json::Value = serde_json::from_slice(response.payload.legacy_bytes()).unwrap();
        let state = resp["state"].as_str().unwrap();
        assert!(!state.is_empty());
        // SHA-256 hash is 64 hex chars
        assert_eq!(state.len(), 64);
    }

    #[test]
    fn versioned_get_resolves_through_snapshot() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_storage(), test_open_state_store());

        // Put with state
        let put_payload = serde_json::json!({
            "path": "/data.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"versioned content"),
            "state": ""
        });
        let put_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Put, Sequence::first(),
            serde_json::to_vec(&put_payload).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(put_request.clone()).unwrap();
        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&put_request).unwrap();
        ack().unwrap();
        let resp: serde_json::Value = serde_json::from_slice(response.payload.legacy_bytes()).unwrap();
        let state_hash = resp["state"].as_str().unwrap().to_string();

        // Get with state
        let get_payload = serde_json::json!({
            "path": "/data.txt",
            "state": state_hash
        });
        let get_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Get, Sequence::from(2),
            serde_json::to_vec(&get_payload).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(get_request.clone()).unwrap();
        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&get_request).unwrap();
        ack().unwrap();

        assert_eq!(response.payload.legacy_bytes(), b"versioned content");
    }

    #[test]
    fn versioned_put_chains_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_storage(), test_open_state_store());

        // First put
        let put1 = serde_json::json!({
            "path": "/a.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"aaa"),
            "state": ""
        });
        let req1 = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Put, Sequence::first(),
            serde_json::to_vec(&put1).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(req1.clone()).unwrap();
        handler.tick();
        let (resp1, ack) = queue.receive_response(&req1).unwrap();
        ack().unwrap();
        let state1: serde_json::Value = serde_json::from_slice(resp1.payload.legacy_bytes()).unwrap();
        let hash1 = state1["state"].as_str().unwrap().to_string();

        // Second put chained from first
        let put2 = serde_json::json!({
            "path": "/b.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"bbb"),
            "state": hash1
        });
        let req2 = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Put, Sequence::from(2),
            serde_json::to_vec(&put2).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(req2.clone()).unwrap();
        handler.tick();
        let (resp2, ack) = queue.receive_response(&req2).unwrap();
        ack().unwrap();
        let state2: serde_json::Value = serde_json::from_slice(resp2.payload.legacy_bytes()).unwrap();
        let hash2 = state2["state"].as_str().unwrap().to_string();

        // Hashes should differ
        assert_ne!(hash1, hash2);

        // Reading /a.txt from state2 should still work (inherited from snapshot)
        let get = serde_json::json!({"path": "/a.txt", "state": hash2});
        let get_req = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Get, Sequence::from(3),
            serde_json::to_vec(&get).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(get_req.clone()).unwrap();
        handler.tick();
        let (resp, ack) = queue.receive_response(&get_req).unwrap();
        ack().unwrap();
        assert_eq!(resp.payload.legacy_bytes(), b"aaa");
    }

    #[test]
    fn versioned_get_returns_empty_for_unknown_path() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_storage(), test_open_state_store());

        // Put to create a state
        let put = serde_json::json!({
            "path": "/exists.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"data"),
            "state": ""
        });
        let put_req = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Put, Sequence::first(),
            serde_json::to_vec(&put).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(put_req.clone()).unwrap();
        handler.tick();
        let (resp, ack) = queue.receive_response(&put_req).unwrap();
        ack().unwrap();
        let state: serde_json::Value = serde_json::from_slice(resp.payload.legacy_bytes()).unwrap();
        let state_hash = state["state"].as_str().unwrap().to_string();

        // Get non-existent path from that state
        let get = serde_json::json!({"path": "/nope.txt", "state": state_hash});
        let get_req = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Get, Sequence::from(2),
            serde_json::to_vec(&get).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(get_req.clone()).unwrap();
        handler.tick();
        let (resp, ack) = queue.receive_response(&get_req).unwrap();
        ack().unwrap();
        assert!(resp.payload.legacy_bytes().is_empty());
    }

    #[test]
    fn kv_get_response_echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_storage(), test_open_state_store());

        // First put a file so get has something to return
        let put_payload = serde_json::json!({
            "path": "/hello.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"hello")
        });
        let put_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Put, Sequence::first(),
            serde_json::to_vec(&put_payload).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(put_request.clone()).unwrap();
        handler.tick();
        let (_resp, ack) = queue.receive_response(&put_request).unwrap();
        ack().unwrap();

        // Now send a get request with state
        let get_payload = serde_json::json!({"path": "/hello.txt"});
        let get_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Get, Sequence::from(2),
            serde_json::to_vec(&get_payload).unwrap(),
            Some("hash123".to_string()),
            test_request_diag(),
        );
        queue.send_request(get_request.clone()).unwrap();
        handler.tick();
        let (response, ack) = queue.receive_response(&get_request).unwrap();
        ack().unwrap();

        assert_eq!(response.state, Some("hash123".to_string()), "get should echo request.state");
    }

    #[test]
    fn unversioned_put_and_get_still_work() {
        // Backward compat: no state field = existing behavior
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_storage(), test_open_state_store());

        let put_payload = serde_json::json!({
            "path": "/test.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"plain data")
        });
        let put_req = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Put, Sequence::first(),
            serde_json::to_vec(&put_payload).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(put_req.clone()).unwrap();
        handler.tick();
        let (resp, ack) = queue.receive_response(&put_req).unwrap();
        ack().unwrap();
        assert_eq!(resp.payload.legacy_bytes(), b"ok");

        let get_payload = serde_json::json!({"path": "/test.txt"});
        let get_req = RequestMessage::new(
            TimelineId::main(),
            test_submission(), test_session(), test_agent_id(),
            ServiceBackend::Kv(ObjectStorageType::InMemory), Operation::Get, Sequence::from(2),
            serde_json::to_vec(&get_payload).unwrap(),
            None,
            test_request_diag(),
        );
        queue.send_request(get_req.clone()).unwrap();
        handler.tick();
        let (resp, ack) = queue.receive_response(&get_req).unwrap();
        ack().unwrap();
        assert_eq!(resp.payload.legacy_bytes(), b"plain data");
    }
}
