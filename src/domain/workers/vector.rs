//! Vector Storage Service Handler - embedding operations over queues.
//!
//! Queues:
//! - `vector-store`: Store an embedding
//! - `vector-search`: Search by vector similarity
//! - `vector-delete`: Delete an embedding

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::domain::registry::Registry;
use crate::domain::service_payloads::{VectorStoreRequest, VectorSearchRequest, VectorDeleteRequest};
use crate::domain::{VectorStorage, ResourceId};
use crate::domain::{MessageQueue, Operation, RequestMessage, ResponseMessage, ServiceDiagnostics, ServiceType};

/// Factory function that opens vector storage from a URI.
pub type OpenVectorStorage = Box<dyn Fn(&ResourceId) -> Result<Arc<dyn VectorStorage>, String> + Send + Sync>;

// ============================================================================
// Handler
// ============================================================================

pub struct VectorServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    stores: RwLock<HashMap<String, Arc<dyn VectorStorage>>>,
    backend: String,
    open_storage: OpenVectorStorage,
}

impl VectorServiceWorker {
    /// Create a new vector storage worker for a specific backend.
    ///
    /// The `open_storage` factory is called to lazy-load stores from
    /// agent metadata. Injected so the worker doesn't depend on
    /// concrete storage implementations.
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        backend: &str,
        open_storage: OpenVectorStorage,
    ) -> Self {
        Self {
            queue,
            registry,
            stores: RwLock::new(HashMap::new()),
            backend: backend.to_string(),
            open_storage,
        }
    }

    /// Get storage for an agent, opening lazily if needed.
    fn get_or_open(&self, agent_id: &str) -> Result<Arc<dyn VectorStorage>, String> {
        // Check cache first
        if let Some(storage) = self.stores.read().unwrap().get(agent_id) {
            return Ok(storage.clone());
        }

        // Look up agent in Registry
        let resource_id = ResourceId::new(agent_id);
        let agent = self.registry.get_agent(&resource_id)
            .ok_or_else(|| format!("unknown agent: {}", agent_id))?;
        let uri = agent.vector_storage
            .ok_or_else(|| format!("agent has no vector_storage declared: {}", agent_id))?;

        // Open storage via injected factory
        let storage = (self.open_storage)(&uri)?;

        // Cache and return
        self.stores.write().unwrap().insert(agent_id.to_string(), storage.clone());
        Ok(storage)
    }

    /// Process one message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        if self.try_store() { return true; }
        if self.try_search() { return true; }
        if self.try_delete() { return true; }
        false
    }

    fn try_store(&self) -> bool {
        // Receive typed RequestMessage (ADR 044)
        match self.queue.receive_request(ServiceType::Vec, &self.backend, Operation::Store) {
            Ok((request, ack)) => {
                let start = std::time::Instant::now();
                let response_payload = self.handle_store(&request);
                let duration_ms = start.elapsed().as_millis() as u64;
                let diag = ServiceDiagnostics::storage(
                    ServiceType::Vec, &self.backend, Operation::Store, response_payload.len() as u64, duration_ms,
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

    fn try_search(&self) -> bool {
        match self.queue.receive_request(ServiceType::Vec, &self.backend, Operation::Search) {
            Ok((request, ack)) => {
                let start = std::time::Instant::now();
                let response_payload = self.handle_search(&request);
                let duration_ms = start.elapsed().as_millis() as u64;
                let diag = ServiceDiagnostics::storage(
                    ServiceType::Vec, &self.backend, Operation::Search, response_payload.len() as u64, duration_ms,
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
        match self.queue.receive_request(ServiceType::Vec, &self.backend, Operation::Delete) {
            Ok((request, ack)) => {
                let start = std::time::Instant::now();
                let response_payload = self.handle_delete(&request);
                let duration_ms = start.elapsed().as_millis() as u64;
                let diag = ServiceDiagnostics::storage(
                    ServiceType::Vec, &self.backend, Operation::Delete, response_payload.len() as u64, duration_ms,
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

    fn handle_store(&self, request: &RequestMessage) -> Vec<u8> {
        let req: VectorStoreRequest = match serde_json::from_slice(request.payload.legacy_bytes()) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        match store.store_embedding(&req.key, &req.vector, &req.metadata) {
            Ok(_) => b"ok".to_vec(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_search(&self, request: &RequestMessage) -> Vec<u8> {
        let req: VectorSearchRequest = match serde_json::from_slice(request.payload.legacy_bytes()) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        match store.search_by_vector(&req.vector, req.limit) {
            Ok(results) => {
                let formatted: Vec<serde_json::Value> = results.iter()
                    .map(|(key, metadata, distance)| {
                        serde_json::json!({
                            "key": key,
                            "metadata": metadata,
                            "distance": distance
                        })
                    })
                    .collect();
                serde_json::to_string(&formatted)
                    .map(|s| s.into_bytes())
                    .unwrap_or_else(|e| format!("[error] {}", e).into_bytes())
            }
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_delete(&self, request: &RequestMessage) -> Vec<u8> {
        let req: VectorDeleteRequest = match serde_json::from_slice(request.payload.legacy_bytes()) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call trait method directly (no pure function needed for simple delete)
        match store.delete_embedding(&req.key) {
            Ok(true) => b"ok".to_vec(),
            Ok(false) => b"not_found".to_vec(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Agent, Registry};
    use crate::registry::InMemoryRegistry;
    use crate::domain::{Operation, RequestDiagnostics, Sequence, ServiceType, SessionId, SubmissionId, TimelineId};
    use crate::domain::SecretStore;
    use crate::secret_store::InMemorySecretStore;
    use crate::queue::InMemoryQueue;

    fn test_secret_store() -> Arc<dyn SecretStore> {
        Arc::new(InMemorySecretStore::new())
    }

    const TEST_AGENT_ID: &str = "http://127.0.0.1:9000/agents/test-agent";

    fn test_request_diag() -> RequestDiagnostics {
        RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 }
    }

    fn test_agent_id() -> ResourceId {
        ResourceId::new(TEST_AGENT_ID)
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-123".to_string())
    }

    fn test_agent_with_vector_storage() -> Agent {
        let manifest = r#"
            name = "test-agent"
            description = "Test agent for vector storage"
            runtime = "container"
            executable = "localhost/test-agent:latest"
            vector_storage = "memory://"
            [requirements]

        "#;
        Agent::from_toml(manifest).unwrap()
    }

    fn test_open_vector_storage() -> OpenVectorStorage {
        Box::new(|_uri: &ResourceId| {
            Ok(Arc::new(crate::storage::InMemoryVectorStorage::new()) as Arc<dyn VectorStorage>)
        })
    }

    #[test]
    fn vector_search_response_echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_vector_storage(crate::domain::VectorStorageType::InMemory);
        let agent = test_agent_with_vector_storage();
        registry.register_agent(agent).unwrap();
        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = VectorServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_vector_storage());

        // Store an embedding first
        let embedding: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let store_payload = serde_json::json!({
            "key": "doc1",
            "vector": embedding,
            "metadata": "test"
        });
        let store_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(), SessionId::new(), test_agent_id(),
            ServiceType::Vec, "memory", Operation::Store, Sequence::first(),
            serde_json::to_vec(&store_payload).unwrap(),
            Some("state-vec".to_string()),
            test_request_diag(),
        );
        queue.send_request(store_request.clone()).unwrap();
        handler.tick();
        let (store_resp, ack) = queue.receive_response(&store_request).unwrap();
        ack().unwrap();
        assert_eq!(store_resp.state, Some("state-vec".to_string()), "store should echo request.state");

        // Search — also with state
        let search_payload = serde_json::json!({
            "vector": embedding,
            "limit": 1
        });
        let search_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(), SessionId::new(), test_agent_id(),
            ServiceType::Vec, "memory", Operation::Search, Sequence::from(2),
            serde_json::to_vec(&search_payload).unwrap(),
            Some("state-vec2".to_string()),
            test_request_diag(),
        );
        queue.send_request(search_request.clone()).unwrap();
        handler.tick();
        let (search_resp, ack) = queue.receive_response(&search_request).unwrap();
        ack().unwrap();
        assert_eq!(search_resp.state, Some("state-vec2".to_string()), "search should echo request.state");
    }

    #[test]
    fn handles_store_and_search() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_vector_storage(crate::domain::VectorStorageType::InMemory);

        // Register test agent with memory:// vector storage
        let agent = test_agent_with_vector_storage();
        registry.register_agent(agent).unwrap();

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = VectorServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory", test_open_vector_storage());

        // Store embedding - worker will lazy-open storage from agent's URI
        let embedding: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let store_payload = serde_json::json!({
            "key": "doc1",
            "vector": embedding,
            "metadata": "test document"
        });
        let store_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceType::Vec,
            "memory",
            Operation::Store,
            Sequence::first(),
            serde_json::to_vec(&store_payload).unwrap(),
            None,
            test_request_diag(),
        );

        queue.send_request(store_request.clone()).unwrap();

        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&store_request).unwrap();
        assert_eq!(response.payload.legacy_bytes(), b"ok");
        ack().unwrap();

        // Search
        let search_payload = serde_json::json!({
            "vector": embedding,
            "limit": 1
        });
        let search_request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceType::Vec,
            "memory",
            Operation::Search,
            Sequence::from(2),
            serde_json::to_vec(&search_payload).unwrap(),
            None,
            test_request_diag(),
        );

        queue.send_request(search_request.clone()).unwrap();

        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&search_request).unwrap();
        let results: Vec<serde_json::Value> = serde_json::from_slice(response.payload.legacy_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["key"], "doc1");
        ack().unwrap();
    }
}
