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
use crate::queue::{Message, MessageQueue};
use crate::services::embedding;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct EmbedRequest {
    agent_id: String,
    model: String,
    text: String,
}

// ============================================================================
// Handler
// ============================================================================

pub struct EmbeddingServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<RwLock<Registry>>,
    engines: RwLock<HashMap<String, Arc<dyn EmbeddingEngine>>>,
}

impl EmbeddingServiceWorker {
    pub fn new(queue: Arc<dyn MessageQueue + Send + Sync>, registry: Arc<RwLock<Registry>>) -> Self {
        Self {
            queue,
            registry,
            engines: RwLock::new(HashMap::new()),
        }
    }

    /// Register an embedding engine by model name (for testing).
    pub fn register(&self, model_name: &str, engine: Arc<dyn EmbeddingEngine>) {
        self.engines.write().unwrap().insert(model_name.to_string(), engine);
    }

    /// Process one message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        if let Ok(msg) = self.queue.receive("embed") {
            let response = self.handle_embed(&msg);
            self.send_response(&msg, response);
            return true;
        }
        false
    }

    fn send_response(&self, request: &Message, payload: Vec<u8>) {
        let response = Message::response(payload, &request.reply_to, request.id.clone());
        let _ = self.queue.send(&request.reply_to, response);
    }

    fn handle_embed(&self, msg: &Message) -> Vec<u8> {
        let req: EmbedRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        // Validate agent declared this model
        if let Err(e) = self.validate_agent_model(&req.agent_id, &req.model) {
            return format!("[error] {}", e).into_bytes();
        }

        // Try to get cached engine, or lazy-load from registry
        let engine = match self.get_or_load_engine(&req.model) {
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

    /// Validate that an agent declared the model in its requirements.
    fn validate_agent_model(&self, agent_id: &str, model_name: &str) -> Result<(), String> {
        let registry = self.registry.read().unwrap();
        let agent_rid = crate::domain::ResourceId::new(agent_id);
        let agent = registry.get_agent(&agent_rid)
            .ok_or_else(|| format!("agent not found: {}", agent_id))?;

        if !agent.has_model(model_name) {
            return Err(format!(
                "agent '{}' did not declare model '{}' in requirements",
                agent.name, model_name
            ));
        }

        Ok(())
    }

    /// Get cached engine or lazy-load from registry.
    fn get_or_load_engine(&self, model_name: &str) -> Result<Arc<dyn EmbeddingEngine>, String> {
        // Check cache first
        {
            let engines = self.engines.read().unwrap();
            if let Some(engine) = engines.get(model_name) {
                return Ok(Arc::clone(engine));
            }
        }

        // Not cached - look up in registry and load
        let model = {
            let registry = self.registry.read().unwrap();
            registry.get_model(model_name)
                .cloned()
                .ok_or_else(|| format!("model not registered: {}", model_name))?
        };

        // Load engine
        let engine = open_embedding_engine(&model)
            .map_err(|e| format!("failed to load engine: {}", e))?;

        // Cache it
        {
            let mut engines = self.engines.write().unwrap();
            engines.insert(model_name.to_string(), Arc::clone(&engine));
        }

        Ok(engine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Agent, EngineType, Model, ModelType};
    use crate::queue::{InMemoryQueue, Message};
    use crate::embedding::InMemoryEmbedding;

    const TEST_AGENT_ID: &str = "file:///test-agent.wasm";

    fn test_model(name: &str) -> Model {
        Model {
            id: crate::domain::ResourceId::new(format!("http://127.0.0.1:9000/models/{}", name)),
            name: name.to_string(),
            model_type: ModelType::Embedding,
            engine: EngineType::InMemory,
            model_path: crate::domain::ResourceId::new("file:///test.gguf"),
        }
    }

    fn test_agent_with_model(model_name: &str) -> Agent {
        let manifest = format!(r#"
            name = "test-agent"
            description = "Test agent"
            id = "{}"
            [requirements]
            services = []
            [requirements.models]
            {} = "http://127.0.0.1:9000/models/{}"
        "#, TEST_AGENT_ID, model_name, model_name);
        Agent::from_toml(&manifest).unwrap()
    }

    fn test_registry_with_agent_and_model(agent: Agent, model_name: &str) -> Arc<RwLock<Registry>> {
        let mut registry = Registry::new();
        registry.register_runtime(crate::domain::RuntimeType::Wasm);
        registry.register_model(test_model(model_name));
        registry.register_agent(agent).unwrap();
        Arc::new(RwLock::new(registry))
    }

    #[test]
    fn handles_embed_request() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = test_registry_with_agent_and_model(test_agent_with_model("test-model"), "test-model");
        let handler = EmbeddingServiceWorker::new(Arc::clone(&queue), registry);

        // Register mock engine
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned.clone()));
        handler.register("test-model", engine);

        // Send embed request
        let payload = serde_json::json!({
            "agent_id": TEST_AGENT_ID,
            "model": "test-model",
            "text": "hello world"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        queue.send("embed", msg).unwrap();

        // Process
        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        let vector: Vec<f32> = serde_json::from_slice(&response.payload).unwrap();
        assert_eq!(vector.len(), 768);
        assert_eq!(vector, canned);
    }

    #[test]
    fn rejects_undeclared_model() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        // Agent declares "allowed-model" but we'll request "other-model"
        let registry = test_registry_with_agent_and_model(test_agent_with_model("allowed-model"), "allowed-model");
        let handler = EmbeddingServiceWorker::new(Arc::clone(&queue), registry);

        // Register mock engine (the model exists, but agent didn't declare it)
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned));
        handler.register("other-model", engine);

        let payload = serde_json::json!({
            "agent_id": TEST_AGENT_ID,
            "model": "other-model",
            "text": "hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        queue.send("embed", msg).unwrap();

        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        let text = String::from_utf8(response.payload).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("did not declare model"));
    }

    #[test]
    fn rejects_unknown_agent() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        // Registry with no agents registered
        let registry = Arc::new(RwLock::new(Registry::new()));
        let handler = EmbeddingServiceWorker::new(Arc::clone(&queue), registry);

        // Register mock engine
        let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
        let engine = Arc::new(InMemoryEmbedding::new(canned));
        handler.register("test-model", engine);

        let payload = serde_json::json!({
            "agent_id": "file:///unknown-agent.wasm",
            "model": "test-model",
            "text": "hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        queue.send("embed", msg).unwrap();

        assert!(handler.tick());
        let response = queue.receive("reply").unwrap();
        let text = String::from_utf8(response.payload).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("agent not found"));
    }
}
