//! Provider - service worker aggregation.
//!
//! Aggregates service workers and routes messages to backends.
//! All workers lazy-load resources from Registry on first use.

use std::sync::Arc;

use super::registry::Registry;
use super::workers::{
    ObjectServiceWorker, VectorServiceWorker,
    InferenceServiceWorker, EmbeddingServiceWorker,
};
use crate::queue::MessageQueue;

/// Aggregates service workers for the runtime.
///
/// Each worker handles one service type (object storage, vector storage,
/// inference, embedding). Storage workers lazy-open based on agent's
/// manifest URI. Supports heterogeneous deployments.
pub struct Provider {
    object: ObjectServiceWorker,
    vector: VectorServiceWorker,
    inference: InferenceServiceWorker,
    embedding: EmbeddingServiceWorker,
}

impl Provider {
    /// Create a new Provider with all service workers for local mode.
    ///
    /// Uses "memory" backend for storage and model workers.
    /// For distributed mode with specific backends, use worker.rs instead.
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>, registry: Arc<dyn Registry>) -> Self {
        Self {
            object: ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory"),
            vector: VectorServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory"),
            inference: InferenceServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory"),
            embedding: EmbeddingServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory"),
        }
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
    use crate::domain::{Agent, InMemoryRegistry, ResourceId};
    use crate::queue::{InMemoryQueue, RequestMessage, Sequence, SubmissionId};

    fn test_agent(id: &str) -> Agent {
        let manifest = format!(r#"
            name = "test-agent"
            description = "Test agent"
            id = "{}"
            object_storage = "memory://"
            [requirements]
            services = []
        "#, id);
        Agent::from_toml(&manifest).unwrap()
    }

    fn test_submission() -> SubmissionId {
        SubmissionId::from("sub-test-123".to_string())
    }

    #[test]
    fn heterogeneous_backends() {
        // One Provider can serve multiple agents with different backends
        // Each agent gets its own isolated storage (lazy-opened from memory://)
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new();
        registry.register_runtime(crate::domain::RuntimeType::Wasm);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);

        // Register agents - each declares memory:// storage (each gets separate instance)
        registry.register_agent(test_agent("file:///agent-a.wasm")).unwrap();
        registry.register_agent(test_agent("file:///agent-b.wasm")).unwrap();

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let provider = Provider::new(Arc::clone(&queue), Arc::clone(&registry));

        // Write to agent-a's storage using typed message (ADR 044)
        let put_a = serde_json::json!({
            "agent_id": "file:///agent-a.wasm",
            "path": "/data.txt",
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"data for A")
        });
        let request_a = RequestMessage::new(
            test_submission(),
            ResourceId::new("file:///agent-a.wasm"),
            "kv",
            "memory",
            "put",
            Sequence::first(),
            serde_json::to_vec(&put_a).unwrap(),
        );
        queue.send_request(request_a).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response("res.kv.memory").unwrap();
        assert_eq!(response.payload, b"ok");
        ack().unwrap();

        // Write to agent-b's storage
        let put_b = serde_json::json!({
            "agent_id": "file:///agent-b.wasm",
            "path": "/data.txt",
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"data for B")
        });
        let request_b = RequestMessage::new(
            test_submission(),
            ResourceId::new("file:///agent-b.wasm"),
            "kv",
            "memory",
            "put",
            Sequence::from(2),
            serde_json::to_vec(&put_b).unwrap(),
        );
        queue.send_request(request_b).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response("res.kv.memory").unwrap();
        assert_eq!(response.payload, b"ok");
        ack().unwrap();

        // Read from agent-a - gets A's data
        let get_a = serde_json::json!({ "agent_id": "file:///agent-a.wasm", "path": "/data.txt" });
        let request_get_a = RequestMessage::new(
            test_submission(),
            ResourceId::new("file:///agent-a.wasm"),
            "kv",
            "memory",
            "get",
            Sequence::from(3),
            serde_json::to_vec(&get_a).unwrap(),
        );
        queue.send_request(request_get_a).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response("res.kv.memory").unwrap();
        assert_eq!(response.payload, b"data for A");
        ack().unwrap();

        // Read from agent-b - gets B's data (isolated)
        let get_b = serde_json::json!({ "agent_id": "file:///agent-b.wasm", "path": "/data.txt" });
        let request_get_b = RequestMessage::new(
            test_submission(),
            ResourceId::new("file:///agent-b.wasm"),
            "kv",
            "memory",
            "get",
            Sequence::from(4),
            serde_json::to_vec(&get_b).unwrap(),
        );
        queue.send_request(request_get_b).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response("res.kv.memory").unwrap();
        assert_eq!(response.payload, b"data for B");
        ack().unwrap();
    }
}
