//! Embedding Service Handler - vector embedding over queues.
//!
//! Queue:
//! - `embed`: Generate embeddings for text
//!
//! Engines are lazy-loaded on first use from Registry model metadata.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::domain::EmbeddingEngine;
use crate::domain::registry::Registry;
use crate::embedding::open_embedding_engine;
use crate::queue::{ExpectsReply, MessageQueue, RequestMessage};
use crate::services::embedding;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct EmbedRequest {
    model: String,
    text: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct EmbeddingServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    engines: RwLock<HashMap<String, Arc<dyn EmbeddingEngine>>>,
    backend: String,
}

impl EmbeddingServiceWorker {
    /// Create a new embedding worker for a specific backend.
    ///
    /// The backend determines which queue this worker subscribes to:
    /// - "ollama" → `vlinder.svc.embed.ollama`
    pub fn new(
        queue: Arc<dyn MessageQueue + Send + Sync>,
        registry: Arc<dyn Registry>,
        backend: &str,
    ) -> Self {
        Self {
            queue,
            registry,
            engines: RwLock::new(HashMap::new()),
            backend: backend.to_string(),
        }
    }

    /// Register an embedding engine by model name (for testing).
    pub fn register(&self, model_name: &str, engine: Arc<dyn EmbeddingEngine>) {
        self.engines.write().unwrap().insert(model_name.to_string(), engine);
    }

    /// Process one message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        // Receive typed RequestMessage (ADR 044)
        match self.queue.receive_request("embed", &self.backend, "run") {
            Ok((request, ack)) => {
                tracing::debug!(seq = %request.sequence, agent = %request.agent_id, "embed worker: received request");
                let start = std::time::Instant::now();
                let response_payload = self.handle_embed(&request);
                tracing::debug!(seq = %request.sequence, elapsed = ?start.elapsed(), "embed worker: handled, sending response");
                // Use ExpectsReply to build properly-correlated ResponseMessage
                let response = request.create_reply(response_payload);
                if let Err(e) = self.queue.send_response(response) {
                    tracing::error!(seq = %request.sequence, error = %e, "embed worker: failed to send response");
                }
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    fn handle_embed(&self, request: &RequestMessage) -> Vec<u8> {
        let req: EmbedRequest = match serde_json::from_slice(&request.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        // Resolve model alias to model_path via agent's manifest
        let model_path = match self.resolve_model_uri(request.agent_id.as_str(), &req.model) {
            Ok(uri) => uri,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Try to get cached engine, or lazy-load from registry
        let engine = match self.get_or_load_engine(&req.model, &model_path) {
            Ok(e) => e,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function
        match embedding::run_embed(engine.as_ref(), &req.text) {
            Ok(vector) => {
                // Serialize to JSON for transport
                match serde_json::to_vec(&vector) {
                    Ok(json) => json,
                    Err(e) => format!("[error] {}", e).into_bytes(),
                }
            }
            Err(e) => format!("[error] {}", e).into_bytes(),
        }
    }

    /// Validate that an agent declared the model and return its URI.
    fn resolve_model_uri(&self, agent_id: &str, model_alias: &str) -> Result<crate::domain::ResourceId, String> {
        let agent_rid = crate::domain::ResourceId::new(agent_id);
        let agent = self.registry.get_agent(&agent_rid)
            .ok_or_else(|| format!("agent not found: {}", agent_id))?;

        agent.model_uri(model_alias)
            .cloned()
            .ok_or_else(|| format!(
                "agent '{}' did not declare model '{}' in requirements",
                agent.name, model_alias
            ))
    }

    /// Get cached engine or lazy-load from registry using model_path.
    fn get_or_load_engine(&self, model_alias: &str, model_path: &crate::domain::ResourceId) -> Result<Arc<dyn EmbeddingEngine>, String> {
        // Check cache first (keyed by alias for this agent's usage)
        {
            let engines = self.engines.read().unwrap();
            if let Some(engine) = engines.get(model_alias) {
                return Ok(Arc::clone(engine));
            }
        }

        // Not cached - look up in registry by model_path
        let model = self.registry.get_model_by_path(model_path)
            .ok_or_else(|| format!("model not registered with path: {}", model_path))?;

        // Load engine
        let engine = open_embedding_engine(&model)
            .map_err(|e| format!("failed to load engine: {}", e))?;

        // Cache it (keyed by alias)
        {
            let mut engines = self.engines.write().unwrap();
            engines.insert(model_alias.to_string(), Arc::clone(&engine));
        }

        Ok(engine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Agent, EngineType, InMemoryRegistry, Model, ModelType, ResourceId};
    use crate::queue::{InMemoryQueue, RequestDiagnostics, Sequence, SessionId, SubmissionId};
    use crate::embedding::InMemoryEmbedding;

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

    fn test_model(name: &str) -> Model {
        // model_path must match the URI in the agent manifest
        Model {
            id: crate::domain::ResourceId::new(format!("http://127.0.0.1:9000/models/{}", name)),
            name: name.to_string(),
            model_type: ModelType::Embedding,
            engine: EngineType::InMemory,
            model_path: crate::domain::ResourceId::new(format!("memory://test/{}", name)),
            digest: format!("sha256:test-digest-{}", name),
        }
    }

    fn test_agent_with_model(model_alias: &str) -> Agent {
        // The RHS URI must match test_model's model_path
        let manifest = format!(r#"
            name = "test-agent"
            description = "Test agent"
            runtime = "container"
            executable = "localhost/test-agent:latest"
            [requirements]
            services = []
            [requirements.models]
            {} = "memory://test/{}"
        "#, model_alias, model_alias);
        Agent::from_toml(&manifest).unwrap()
    }

    fn test_registry_with_agent_and_model(agent: Agent, model_name: &str) -> Arc<dyn Registry> {
        let registry = InMemoryRegistry::new();
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_embedding_engine(EngineType::InMemory);
        registry.register_model(test_model(model_name)).unwrap();
        registry.register_agent(agent).unwrap();
        Arc::new(registry)
    }

    #[test]
    fn handles_embed_request() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = test_registry_with_agent_and_model(test_agent_with_model("test-model"), "test-model");
        let handler = EmbeddingServiceWorker::new(Arc::clone(&queue), registry, "memory");

        // Register mock engine
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned.clone()));
        handler.register("test-model", engine);

        // Send typed RequestMessage (ADR 044)
        let payload = serde_json::json!({
            "model": "test-model",
            "text": "hello world"
        });
        let request = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            "embed",
            "memory",
            "run",
            Sequence::first(),
            serde_json::to_vec(&payload).unwrap(),
            test_request_diag(),
        );

        queue.send_request(request.clone()).unwrap();

        // Process
        assert!(handler.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        let vector: Vec<f32> = serde_json::from_slice(&response.payload).unwrap();
        assert_eq!(vector.len(), 768);
        assert_eq!(vector, canned);
        ack().unwrap();
    }

    #[test]
    fn rejects_undeclared_model() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        // Agent declares "allowed-model" but we'll request "other-model"
        let registry = test_registry_with_agent_and_model(test_agent_with_model("allowed-model"), "allowed-model");
        let handler = EmbeddingServiceWorker::new(Arc::clone(&queue), registry, "memory");

        // Register mock engine (the model exists, but agent didn't declare it)
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned));
        handler.register("other-model", engine);

        let payload = serde_json::json!({
            "model": "other-model",
            "text": "hello"
        });
        let request = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            "embed",
            "memory",
            "run",
            Sequence::first(),
            serde_json::to_vec(&payload).unwrap(),
            test_request_diag(),
        );

        queue.send_request(request.clone()).unwrap();

        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&request).unwrap();
        let text = String::from_utf8(response.payload.clone()).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("did not declare model"));
        ack().unwrap();
    }

    #[test]
    fn rejects_unknown_agent() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        // Registry with no agents registered
        let registry: Arc<dyn Registry> = Arc::new(InMemoryRegistry::new());
        let handler = EmbeddingServiceWorker::new(Arc::clone(&queue), registry, "memory");

        // Register mock engine
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned));
        handler.register("test-model", engine);

        let payload = serde_json::json!({
            "model": "test-model",
            "text": "hello"
        });
        let request = RequestMessage::new(
            test_submission(),
            SessionId::new(),
            ResourceId::new("http://127.0.0.1:9000/agents/unknown-agent"),
            "embed",
            "memory",
            "run",
            Sequence::first(),
            serde_json::to_vec(&payload).unwrap(),
            test_request_diag(),
        );

        queue.send_request(request.clone()).unwrap();

        assert!(handler.tick());
        let (response, ack) = queue.receive_response(&request).unwrap();
        let text = String::from_utf8(response.payload.clone()).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("agent not found"));
        ack().unwrap();
    }
}
