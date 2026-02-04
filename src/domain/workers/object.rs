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
use crate::queue::{Message, MessageQueue};
use crate::services::object_storage;
use crate::storage::dispatch::open_object_storage_from_uri;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct GetRequest {
    agent_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct PutRequest {
    agent_id: String,
    path: String,
    content: String, // base64 encoded
}

#[derive(Debug, Deserialize)]
struct ListRequest {
    agent_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct DeleteRequest {
    agent_id: String,
    path: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct ObjectServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    stores: RwLock<HashMap<String, Arc<dyn ObjectStorage>>>,
}

impl ObjectServiceWorker {
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>, registry: Arc<dyn Registry>) -> Self {
        Self {
            queue,
            registry,
            stores: RwLock::new(HashMap::new()),
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
        tracing::debug!("ObjectServiceWorker: checking for kv-get messages");
        match self.queue.receive("kv-get") {
            Ok(pending) => {
                tracing::info!("ObjectServiceWorker: received kv-get request");
                let response = self.handle_get(&pending.message);
                self.send_response(&pending.message, response);
                let _ = pending.ack();
                true
            }
            Err(e) => {
                tracing::debug!("ObjectServiceWorker: no kv-get message: {:?}", e);
                false
            }
        }
    }

    fn try_put(&self) -> bool {
        match self.queue.receive("kv-put") {
            Ok(pending) => {
                let response = self.handle_put(&pending.message);
                self.send_response(&pending.message, response);
                let _ = pending.ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_list(&self) -> bool {
        match self.queue.receive("kv-list") {
            Ok(pending) => {
                let response = self.handle_list(&pending.message);
                self.send_response(&pending.message, response);
                let _ = pending.ack();
                true
            }
            Err(_) => false,
        }
    }

    fn try_delete(&self) -> bool {
        match self.queue.receive("kv-delete") {
            Ok(pending) => {
                let response = self.handle_delete(&pending.message);
                self.send_response(&pending.message, response);
                let _ = pending.ack();
                true
            }
            Err(_) => false,
        }
    }

    fn send_response(&self, request: &Message, payload: Vec<u8>) {
        let response = Message::response(payload, &request.reply_to, request.id.clone());
        let _ = self.queue.send(&request.reply_to, response);
    }

    fn handle_get(&self, msg: &Message) -> Vec<u8> {
        let req: GetRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(&req.agent_id) {
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

    fn handle_put(&self, msg: &Message) -> Vec<u8> {
        let req: PutRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(&req.agent_id) {
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

    fn handle_list(&self, msg: &Message) -> Vec<u8> {
        let req: ListRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(&req.agent_id) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function
        match object_storage::list_files(store.as_ref(), &req.path) {
            Ok(json) => json.into_bytes(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_delete(&self, msg: &Message) -> Vec<u8> {
        let req: DeleteRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(&req.agent_id) {
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
    use crate::queue::{InMemoryQueue, Message};

    fn test_agent_with_object_storage() -> Agent {
        let manifest = r#"
            name = "test-agent"
            description = "Test agent for object storage"
            id = "file:///test.wasm"
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
        registry.register_runtime(crate::domain::RuntimeType::Wasm);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);

        // Register test agent with memory:// object storage
        let agent = test_agent_with_object_storage();
        registry.register_agent(agent).unwrap();

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry));

        // Send put request - worker will lazy-open storage from agent's URI
        let put_payload = serde_json::json!({
            "agent_id": "file:///test.wasm",
            "path": "/hello.txt",
            "content": base64::engine::general_purpose::STANDARD.encode(b"hello world")
        });
        let put_msg = Message::request(
            serde_json::to_vec(&put_payload).unwrap(),
            "reply",
        );
        queue.send("kv-put", put_msg).unwrap();

        // Process
        assert!(handler.tick());
        let pending = queue.receive("reply").unwrap();
        assert_eq!(pending.message.payload, b"ok");
        pending.ack().unwrap();

        // Send get request
        let get_payload = serde_json::json!({
            "agent_id": "file:///test.wasm",
            "path": "/hello.txt"
        });
        let get_msg = Message::request(
            serde_json::to_vec(&get_payload).unwrap(),
            "reply",
        );
        queue.send("kv-get", get_msg).unwrap();

        // Process
        assert!(handler.tick());
        let pending = queue.receive("reply").unwrap();
        assert_eq!(pending.message.payload, b"hello world");
        pending.ack().unwrap();
    }
}
