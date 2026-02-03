//! Vector Storage Service Handler - embedding operations over queues.
//!
//! Queues:
//! - `vector-store`: Store an embedding
//! - `vector-search`: Search by vector similarity
//! - `vector-delete`: Delete an embedding

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::domain::registry::Registry;
use crate::domain::{VectorStorage, ResourceId};
use crate::queue::{Message, MessageQueue};
use crate::services::vector_storage;
use crate::storage::dispatch::open_vector_storage_from_uri;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct StoreRequest {
    agent_id: String,
    key: String,
    vector: Vec<f32>,
    metadata: String,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    agent_id: String,
    vector: Vec<f32>,
    limit: u32,
}

#[derive(Debug, Deserialize)]
struct DeleteRequest {
    agent_id: String,
    key: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct VectorServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    stores: RwLock<HashMap<String, Arc<dyn VectorStorage>>>,
}

impl VectorServiceWorker {
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>, registry: Arc<dyn Registry>) -> Self {
        Self {
            queue,
            registry,
            stores: RwLock::new(HashMap::new()),
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

        // Open storage
        let storage = open_vector_storage_from_uri(&uri)
            .map_err(|e| format!("failed to open vector storage: {}", e))?;

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
        if let Ok(msg) = self.queue.receive("vector-store") {
            let response = self.handle_store(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn try_search(&self) -> bool {
        if let Ok(msg) = self.queue.receive("vector-search") {
            let response = self.handle_search(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn try_delete(&self) -> bool {
        if let Ok(msg) = self.queue.receive("vector-delete") {
            let response = self.handle_delete(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn send_response(&self, request: &Message, payload: Vec<u8>) {
        let response = Message::response(payload, &request.reply_to, request.id.clone());
        let _ = self.queue.send(&request.reply_to, response);
    }

    fn handle_store(&self, msg: &Message) -> Vec<u8> {
        let req: StoreRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(&req.agent_id) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function (vec variant)
        match vector_storage::store_embedding_vec(store.as_ref(), &req.key, &req.vector, &req.metadata) {
            Ok(_) => b"ok".to_vec(),
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    fn handle_search(&self, msg: &Message) -> Vec<u8> {
        let req: SearchRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.get_or_open(&req.agent_id) {
            Ok(s) => s,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function (vec variant)
        match vector_storage::search_by_vec(store.as_ref(), &req.vector, req.limit) {
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
    use crate::domain::{Agent, InMemoryRegistry, Registry};
    use crate::queue::{InMemoryQueue, Message};

    fn test_agent_with_vector_storage() -> Agent {
        let manifest = r#"
            name = "test-agent"
            description = "Test agent for vector storage"
            id = "file:///test.wasm"
            vector_storage = "memory://"
            [requirements]
            services = []
        "#;
        Agent::from_toml(manifest).unwrap()
    }

    #[test]
    fn handles_store_and_search() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new();
        registry.register_runtime(crate::domain::RuntimeType::Wasm);
        registry.register_vector_storage(crate::domain::VectorStorageType::InMemory);

        // Register test agent with memory:// vector storage
        let agent = test_agent_with_vector_storage();
        registry.register_agent(agent).unwrap();

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let handler = VectorServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry));

        // Store embedding - worker will lazy-open storage from agent's URI
        let embedding: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let store_payload = serde_json::json!({
            "agent_id": "file:///test.wasm",
            "key": "doc1",
            "vector": embedding,
            "metadata": "test document"
        });
        let store_msg = Message::request(
            serde_json::to_vec(&store_payload).unwrap(),
            "reply",
        );
        queue.send("vector-store", store_msg).unwrap();

        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        assert_eq!(response.payload, b"ok");

        // Search
        let search_payload = serde_json::json!({
            "agent_id": "file:///test.wasm",
            "vector": embedding,
            "limit": 1
        });
        let search_msg = Message::request(
            serde_json::to_vec(&search_payload).unwrap(),
            "reply",
        );
        queue.send("vector-search", search_msg).unwrap();

        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        let results: Vec<serde_json::Value> = serde_json::from_slice(&response.payload).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["key"], "doc1");
    }
}
