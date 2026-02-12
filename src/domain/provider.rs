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
use super::MessageQueue;

/// Aggregates service workers for the runtime.
///
/// In local mode, creates workers for all known backends so any agent
/// manifest (sqlite, memory, ollama, etc.) is handled in-process.
/// In distributed mode, individual workers are started separately.
pub struct Provider {
    workers: Vec<Box<dyn TickWorker>>,
}

/// Trait for tick-able workers so Provider can hold them in a Vec.
trait TickWorker {
    fn tick(&self) -> bool;
}

impl TickWorker for ObjectServiceWorker {
    fn tick(&self) -> bool { self.tick() }
}

impl TickWorker for VectorServiceWorker {
    fn tick(&self) -> bool { self.tick() }
}

impl TickWorker for InferenceServiceWorker {
    fn tick(&self) -> bool { self.tick() }
}

impl TickWorker for EmbeddingServiceWorker {
    fn tick(&self) -> bool { self.tick() }
}

impl Provider {
    /// Create a new Provider with workers for all backends.
    ///
    /// Local mode needs to handle any agent manifest, so we register
    /// workers for every backend: sqlite, memory, ollama, etc.
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>, registry: Arc<dyn Registry>) -> Self {
        let workers: Vec<Box<dyn TickWorker>> = vec![
            // Object storage: sqlite + memory
            Box::new(ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "sqlite")),
            Box::new(ObjectServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory")),
            // Vector storage: sqlite-vec + memory
            Box::new(VectorServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "sqlite-vec")),
            Box::new(VectorServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory")),
            // Inference: ollama + openrouter + memory (test)
            Box::new(InferenceServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "ollama")),
            Box::new(InferenceServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "openrouter")),
            Box::new(InferenceServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "memory")),
            // Embedding: ollama + memory (test)
            Box::new(EmbeddingServiceWorker::new(Arc::clone(&queue), Arc::clone(&registry), "ollama")),
            Box::new(EmbeddingServiceWorker::new(Arc::clone(&queue), registry, "memory")),
        ];

        Self { workers }
    }

    /// Process one service message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        for worker in &self.workers {
            if worker.tick() {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Agent, InMemoryRegistry, ResourceId};
    use crate::domain::{RequestDiagnostics, RequestMessage, Sequence, SessionId, SubmissionId};
    use crate::queue::InMemoryQueue;

    fn test_agent(name: &str) -> Agent {
        let manifest = format!(r#"
            name = "{}"
            description = "Test agent"
            runtime = "container"
            executable = "localhost/{}:latest"
            object_storage = "memory://"
            [requirements]
            services = []
        "#, name, name);
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
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_object_storage(crate::domain::ObjectStorageType::InMemory);

        // Register agents - each declares memory:// storage (each gets separate instance)
        registry.register_agent(test_agent("agent-a")).unwrap();
        registry.register_agent(test_agent("agent-b")).unwrap();

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let provider = Provider::new(Arc::clone(&queue), Arc::clone(&registry));

        // Write to agent-a's storage using typed message (ADR 044)
        let put_a = serde_json::json!({
            "path": "/data.txt",
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"data for A")
        });
        let request_a = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            ResourceId::new("http://127.0.0.1:9000/agents/agent-a"),
            "kv",
            "memory",
            "put",
            Sequence::first(),
            serde_json::to_vec(&put_a).unwrap(),
            None,
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );
        queue.send_request(request_a.clone()).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response(&request_a).unwrap();
        assert_eq!(response.payload, b"ok");
        ack().unwrap();

        // Write to agent-b's storage
        let put_b = serde_json::json!({
            "path": "/data.txt",
            "content": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"data for B")
        });
        let request_b = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            ResourceId::new("http://127.0.0.1:9000/agents/agent-b"),
            "kv",
            "memory",
            "put",
            Sequence::from(2),
            serde_json::to_vec(&put_b).unwrap(),
            None,
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );
        queue.send_request(request_b.clone()).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response(&request_b).unwrap();
        assert_eq!(response.payload, b"ok");
        ack().unwrap();

        // Read from agent-a - gets A's data
        let get_a = serde_json::json!({ "path": "/data.txt" });
        let request_get_a = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            ResourceId::new("http://127.0.0.1:9000/agents/agent-a"),
            "kv",
            "memory",
            "get",
            Sequence::from(3),
            serde_json::to_vec(&get_a).unwrap(),
            None,
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );
        queue.send_request(request_get_a.clone()).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response(&request_get_a).unwrap();
        assert_eq!(response.payload, b"data for A");
        ack().unwrap();

        // Read from agent-b - gets B's data (isolated)
        let get_b = serde_json::json!({ "path": "/data.txt" });
        let request_get_b = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            ResourceId::new("http://127.0.0.1:9000/agents/agent-b"),
            "kv",
            "memory",
            "get",
            Sequence::from(4),
            serde_json::to_vec(&get_b).unwrap(),
            None,
            RequestDiagnostics { sequence: 0, endpoint: String::new(), request_bytes: 0, received_at_ms: 0 },
        );
        queue.send_request(request_get_b.clone()).unwrap();
        provider.tick();
        let (response, ack) = queue.receive_response(&request_get_b).unwrap();
        assert_eq!(response.payload, b"data for B");
        ack().unwrap();
    }
}
