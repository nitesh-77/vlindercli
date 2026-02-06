//! Object Storage Service Handler - KV operations over queues.
//!
//! Queues:
//! - `kv-get`: Retrieve file content
//! - `kv-put`: Store file content
//! - `kv-list`: List files in path
//! - `kv-delete`: Delete file

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use base64::Engine as _;
use serde::Deserialize;

use crate::domain::registry::Registry;
use crate::domain::{ObjectStorage, ResourceId};
use crate::queue::{ExpectsReply, MessageQueue, RequestMessage};
use crate::services::object_storage;
use crate::storage::dispatch::open_object_storage_from_uri;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct GetRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct PutRequest {
    path: String,
    content: String, // base64 encoded
}

#[derive(Debug, Deserialize)]
struct ListRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct DeleteRequest {
    path: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct ObjectServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    stores: RwLock<HashMap<String, Arc<dyn ObjectStorage>>>,
    backend: String,
}

impl ObjectServiceWorker {
    /// Create a new object storage worker for a specific backend.
    ///
    /// The backend determines which queues this worker subscribes to:
    /// - "sqlite" → `vlinder.svc.kv.sqlite.*`
    /// - "memory" → `vlinder.svc.kv.memory.*`
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        backend: &str,
    ) -> Self {
        Self {
            queue,
            registry,
            stores: RwLock::new(HashMap::new()),
            backend: backend.to_string(),
        }
    }

    /// Get storage for an agent, opening lazily if needed.
    fn get_or_open(&self, agent_id: &str) -> Result<Arc<dyn ObjectStorage>, String> {
        // Check cache first
        if let Some(storage) = self.stores.read().unwrap().get(agent_id) {
            return Ok(storage.clone());
        }

        // Look up agent in Registry
        let resource_id = ResourceId::new(agent_id);
        let agent = self.registry.get_agent(&resource_id)
            .ok_or_else(|| format!("unknown agent: {}", agent_id))?;
        let uri = agent.object_storage
            .ok_or_else(|| format!("agent has no object_storage declared: {}", agent_id))?;

        // Open storage
        let storage = open_object_storage_from_uri(&uri)
            .map_err(|e| format!("failed to open object storage: {}", e))?;

        // Cache and return
        self.stores.write().unwrap().insert(agent_id.to_string(), storage.clone());
        Ok(storage)
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
        match self.queue.receive_request("kv", &self.backend, "get") {
            Ok((request, ack)) => {
                let response_payload = self.handle_get(&request);
                let response = request.create_reply(response_payload);
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_put(&self) -> bool {
        match self.queue.receive_request("kv", &self.backend, "put") {
            Ok((request, ack)) => {
                let response_payload = self.handle_put(&request);
                let response = request.create_reply(response_payload);
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_list(&self) -> bool {
        match self.queue.receive_request("kv", &self.backend, "list") {
            Ok((request, ack)) => {
                let response_payload = self.handle_list(&request);
                let response = request.create_reply(response_payload);
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_delete(&self) -> bool {
        match self.queue.receive_request("kv", &self.backend, "delete") {
            Ok((request, ack)) => {
                let response_payload = self.handle_delete(&request);
                let response = request.create_reply(response_payload);
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn handle_get(&self, request: &RequestMessage) -> Vec<u8> {
        let req: GetRequest = match serde_json::from_slice(&request.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function
        match object_storage::get_file(store.as_ref(), &req.path) {
            Ok(content) => content,
            Err(object_storage::Error::FileNotFound) => Vec::new(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_put(&self, request: &RequestMessage) -> Vec<u8> {
        let req: PutRequest = match serde_json::from_slice(&request.payload) {
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

        // Call pure service function
        match object_storage::put_file(store.as_ref(), &req.path, &content) {
            Ok(_) => b"ok".to_vec(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_list(&self, request: &RequestMessage) -> Vec<u8> {
        let req: ListRequest = match serde_json::from_slice(&request.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function
        match object_storage::list_files(store.as_ref(), &req.path) {
            Ok(json) => json.into_bytes(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_delete(&self, request: &RequestMessage) -> Vec<u8> {
        let req: DeleteRequest = match serde_json::from_slice(&request.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(request.agent_id.as_str()) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function
        match object_storage::delete_file(store.as_ref(), &req.path) {
            Ok(result) => result.into_bytes(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Agent, InMemoryRegistry};
    use crate::queue::{InMemoryQueue, Sequence, SubmissionId};

    const TEST_AGENT_ID: &str = "http://127.0.0.1:9000/agents/test-agent";

    fn test_agent_id() -> ResourceId {
        ResourceId::new(TEST_AGENT_ID)
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-123".to_string())
    }

    fn test_agent_with_object_storage() -> Agent {
        let manifest = r#"
            name = "test-agent"
            description = "Test agent for object storage"
            runtime = "container"
            executable = "localhost/test-agent:latest"
            object_storage = "memory://"
            [requirements]
            services = []
        "#;
        Agent::from_toml(manifest).unwrap()
    }

    #[test]
    fn handles_put_and_get() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new();
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);

        // Register test agent with memory:// object storage
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory");

        // Send typed put request (ADR 044)
        let put_payload = serde_json::json!({
            "path": "/hello.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"hello world")
        });
        let put_request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "kv",
            "memory",
            "put",
            Sequence::first(),
            serde_json::to_vec(&put_payload).unwrap(),
        );

        queue.send_request(put_request.clone()).unwrap();

        // Process
        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&put_request).unwrap();
        assert_eq!(response.payload, b"ok");
        ack().unwrap();

        // Send typed get request
        let get_payload = serde_json::json!({
            "path": "/hello.txt"
        });
        let get_request = RequestMessage::new(
            test_submission(),
            test_agent_id(),
            "kv",
            "memory",
            "get",
            Sequence::from(2),
            serde_json::to_vec(&get_payload).unwrap(),
        );

        queue.send_request(get_request.clone()).unwrap();

        // Process
        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&get_request).unwrap();
        assert_eq!(response.payload, b"hello world");
        ack().unwrap();
    }
}
