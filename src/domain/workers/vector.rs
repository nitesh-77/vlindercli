//! Vector Storage Service Handler - embedding operations over queues.
//!
//! Queues:
//! - `vector-store`: Store an embedding
//! - `vector-search`: Search by vector similarity
//! - `vector-delete`: Delete an embedding

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::domain::VectorStorage;
use crate::queue::{Message, MessageQueue};
use crate::services::vector_storage;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct StoreRequest {
    namespace: String,
    key: String,
    vector: Vec<f32>,
    metadata: String,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    namespace: String,
    vector: Vec<f32>,
    limit: u32,
}

#[derive(Debug, Deserialize)]
struct DeleteRequest {
    namespace: String,
    key: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct VectorServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    stores: RwLock<HashMap<String, Arc<dyn VectorStorage>>>,
}

impl VectorServiceWorker {
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>) -> Self {
        Self {
            queue,
            stores: RwLock::new(HashMap::new()),
        }
    }

    /// Register storage for a namespace (typically agent name).
    pub fn register(&self, namespace: &str, storage: Arc<dyn VectorStorage>) {
        self.stores.write().unwrap().insert(namespace.to_string(), storage);
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

        let stores = self.stores.read().unwrap();
        let store = match stores.get(&req.namespace) {
            Some(s) => s,
            None => return format!("[error] unknown namespace: {}", req.namespace).into_bytes(),
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

        let stores = self.stores.read().unwrap();
        let store = match stores.get(&req.namespace) {
            Some(s) => s,
            None => return format!("[error] unknown namespace: {}", req.namespace).into_bytes(),
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

        let stores = self.stores.read().unwrap();
        let store = match stores.get(&req.namespace) {
            Some(s) => s,
            None => return format!("[error] unknown namespace: {}", req.namespace).into_bytes(),
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
    use crate::queue::{InMemoryQueue, Message};
    use crate::storage::dispatch::{in_memory_storage, open_vector_storage};

    #[test]
    fn handles_store_and_search() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let handler = VectorServiceWorker::new(Arc::clone(&queue));

        // Register storage
        let storage = in_memory_storage();
        let vector = open_vector_storage(&storage).unwrap();
        handler.register("test", vector);

        // Store embedding
        let embedding: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let store_payload = serde_json::json!({
            "namespace": "test",
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
            "namespace": "test",
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
