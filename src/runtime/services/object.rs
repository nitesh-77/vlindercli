//! Object Storage Service Handler - KV operations over queues.
//!
//! Queues:
//! - `kv-get`: Retrieve file content
//! - `kv-put`: Store file content
//! - `kv-list`: List files in path
//! - `kv-delete`: Delete file

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use serde::Deserialize;

use crate::domain::ObjectStorage;
use crate::queue::{InMemoryQueue, Message, MessageQueue};
use crate::services::object_storage;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct GetRequest {
    namespace: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct PutRequest {
    namespace: String,
    path: String,
    content: String, // base64 encoded
}

#[derive(Debug, Deserialize)]
struct ListRequest {
    namespace: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct DeleteRequest {
    namespace: String,
    path: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct ObjectServiceHandler {
    queue: Arc<InMemoryQueue>,
    stores: HashMap<String, Arc<dyn ObjectStorage>>,
}

impl ObjectServiceHandler {
    pub fn new(queue: Arc<InMemoryQueue>) -> Self {
        Self {
            queue,
            stores: HashMap::new(),
        }
    }

    /// Register storage for a namespace (typically agent name).
    pub fn register(&mut self, namespace: &str, storage: Arc<dyn ObjectStorage>) {
        self.stores.insert(namespace.to_string(), storage);
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
        if let Ok(msg) = self.queue.receive("kv-get") {
            let response = self.handle_get(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn try_put(&self) -> bool {
        if let Ok(msg) = self.queue.receive("kv-put") {
            let response = self.handle_put(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn try_list(&self) -> bool {
        if let Ok(msg) = self.queue.receive("kv-list") {
            let response = self.handle_list(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn try_delete(&self) -> bool {
        if let Ok(msg) = self.queue.receive("kv-delete") {
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

    fn handle_get(&self, msg: &Message) -> Vec<u8> {
        let req: GetRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        let store = match self.stores.get(&req.namespace) {
            Some(s) => s,
            None => return format!("[error] unknown namespace: {}", req.namespace).into_bytes(),
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

        let store = match self.stores.get(&req.namespace) {
            Some(s) => s,
            None => return format!("[error] unknown namespace: {}", req.namespace).into_bytes(),
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

        let store = match self.stores.get(&req.namespace) {
            Some(s) => s,
            None => return format!("[error] unknown namespace: {}", req.namespace).into_bytes(),
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

        let store = match self.stores.get(&req.namespace) {
            Some(s) => s,
            None => return format!("[error] unknown namespace: {}", req.namespace).into_bytes(),
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
    use crate::queue::Message;
    use crate::storage::dispatch::{in_memory_storage, open_object_storage};

    #[test]
    fn handles_put_and_get() {
        let queue = Arc::new(InMemoryQueue::new());
        let mut handler = ObjectServiceHandler::new(Arc::clone(&queue));

        // Register storage
        let storage = in_memory_storage();
        let object = open_object_storage(&storage).unwrap();
        handler.register("test", object);

        // Send put request
        let put_payload = serde_json::json!({
            "namespace": "test",
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
        let response = queue.receive("reply").unwrap();
        assert_eq!(response.payload, b"ok");

        // Send get request
        let get_payload = serde_json::json!({
            "namespace": "test",
            "path": "/hello.txt"
        });
        let get_msg = Message::request(
            serde_json::to_vec(&get_payload).unwrap(),
            "reply",
        );
        queue.send("kv-get", get_msg).unwrap();

        // Process
        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        assert_eq!(response.payload, b"hello world");
    }
}
