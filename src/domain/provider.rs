//! Provider - service worker aggregation.
//!
//! Aggregates service workers and routes messages to backends.
//! Registration maps agent IDs to backend implementations.

use std::sync::Arc;

use super::{ObjectStorage, VectorStorage, InferenceEngine, EmbeddingEngine};
use super::workers::{
    ObjectServiceWorker, VectorServiceWorker,
    InferenceServiceWorker, EmbeddingServiceWorker,
};
use crate::queue::MessageQueue;

/// Aggregates service workers for the runtime.
///
/// Each worker handles one service type (object storage, vector storage,
/// inference, embedding). Registration maps agent IDs/models to backends.
/// Supports heterogeneous deployments — different agents can use different
/// backends within the same Provider.
pub struct Provider {
    object: ObjectServiceWorker,
    vector: VectorServiceWorker,
    inference: InferenceServiceWorker,
    embedding: EmbeddingServiceWorker,
}

impl Provider {
    /// Create a new Provider with all service workers.
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>) -> Self {
        Self {
            object: ObjectServiceWorker::new(Arc::clone(&queue)),
            vector: VectorServiceWorker::new(Arc::clone(&queue)),
            inference: InferenceServiceWorker::new(Arc::clone(&queue)),
            embedding: EmbeddingServiceWorker::new(Arc::clone(&queue)),
        }
    }

    /// Register object storage for an agent.
    pub fn register_object(&self, agent_id: &str, storage: Arc<dyn ObjectStorage>) {
        self.object.register(agent_id, storage);
    }

    /// Register vector storage for an agent.
    pub fn register_vector(&self, agent_id: &str, storage: Arc<dyn VectorStorage>) {
        self.vector.register(agent_id, storage);
    }

    /// Register inference engine for a model name.
    pub fn register_inference(&self, model: &str, engine: Arc<dyn InferenceEngine>) {
        self.inference.register(model, engine);
    }

    /// Register embedding engine for a model name.
    pub fn register_embedding(&self, model: &str, engine: Arc<dyn EmbeddingEngine>) {
        self.embedding.register(model, engine);
    }

    /// Process one service message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        if self.object.tick() { return true; }
        if self.vector.tick() { return true; }
        if self.inference.tick() { return true; }
        if self.embedding.tick() { return true; }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{InMemoryQueue, Message};
    use crate::storage::dispatch::{in_memory_storage, open_object_storage};

    #[test]
    fn heterogeneous_backends() {
        // One Provider can serve multiple agents with different backends
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let provider = Provider::new(Arc::clone(&queue));

        // Register different storage backends for different agents
        let storage_a = in_memory_storage();
        let storage_b = in_memory_storage();
        let object_a = open_object_storage(&storage_a).unwrap();
        let object_b = open_object_storage(&storage_b).unwrap();

        provider.register_object("agent-a", object_a);
        provider.register_object("agent-b", object_b);

        // Write to agent-a's storage
        let put_a = serde_json::json!({
            "agent_id": "agent-a",
            "path": "/data.txt",
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"data for A")
        });
        let msg_a = Message::request(serde_json::to_vec(&put_a).unwrap(), "reply-put-a");
        queue.send("kv-put", msg_a).unwrap();
        provider.tick();
        let _ = queue.receive("reply-put-a").unwrap(); // consume put response

        // Write to agent-b's storage
        let put_b = serde_json::json!({
            "agent_id": "agent-b",
            "path": "/data.txt",
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"data for B")
        });
        let msg_b = Message::request(serde_json::to_vec(&put_b).unwrap(), "reply-put-b");
        queue.send("kv-put", msg_b).unwrap();
        provider.tick();
        let _ = queue.receive("reply-put-b").unwrap(); // consume put response

        // Read from agent-a - gets A's data
        let get_a = serde_json::json!({ "agent_id": "agent-a", "path": "/data.txt" });
        let msg = Message::request(serde_json::to_vec(&get_a).unwrap(), "reply-get-a");
        queue.send("kv-get", msg).unwrap();
        provider.tick();
        let response = queue.receive("reply-get-a").unwrap();
        assert_eq!(response.payload, b"data for A");

        // Read from agent-b - gets B's data (isolated)
        let get_b = serde_json::json!({ "agent_id": "agent-b", "path": "/data.txt" });
        let msg = Message::request(serde_json::to_vec(&get_b).unwrap(), "reply-get-b");
        queue.send("kv-get", msg).unwrap();
        provider.tick();
        let response = queue.receive("reply-get-b").unwrap();
        assert_eq!(response.payload, b"data for B");
    }
}
