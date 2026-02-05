//! Inference Service Handler - LLM inference over queues.
//!
//! Queue:
//! - `infer`: Run inference with a model
//!
//! Engines are lazy-loaded on first use from Registry model metadata.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::domain::InferenceEngine;
use crate::domain::registry::Registry;
use crate::inference::open_inference_engine;
use crate::queue::{Message, MessageQueue};
use crate::services::inference;

// ============================================================================
// Request Types (queue protocol)
// ============================================================================

#[derive(Debug, Deserialize)]
struct InferRequest {
    agent_id: String,
    model: String,
    prompt: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    256
}

// ============================================================================
// Handler
// ============================================================================

pub struct InferenceServiceWorker {
    queue: Arc<dyn MessageQueue + Send + Sync>,
    registry: Arc<dyn Registry>,
    engines: RwLock<HashMap<String, Arc<dyn InferenceEngine>>>,
    backend: String,
}

impl InferenceServiceWorker {
    /// Create a new inference worker for a specific backend.
    ///
    /// The backend determines which queue this worker subscribes to:
    /// - "ollama" → `vlinder.svc.infer.ollama`
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

    /// Register an inference engine by model name (for testing).
    pub fn register(&self, model_name: &str, engine: Arc<dyn InferenceEngine>) {
        self.engines.write().unwrap().insert(model_name.to_string(), engine);
    }

    /// Process one message if available. Returns true if processed.
    pub fn tick(&self) -> bool {
        let queue_name = self.queue.service_queue("infer", &self.backend, "");
        match self.queue.receive(&queue_name) {
            Ok(pending) => {
                let response = self.handle_infer(&pending.message);
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

    fn handle_infer(&self, msg: &Message) -> Vec<u8> {
        let req: InferRequest = match serde_json::from_slice(&msg.payload) {
            Ok(r) => r,
            Err(e) => return format!("[error] invalid request: {}", e).into_bytes(),
        };

        // Resolve model alias to model_path via agent's manifest
        let model_path = match self.resolve_model_uri(&req.agent_id, &req.model) {
            Ok(uri) => uri,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Try to get cached engine, or lazy-load from registry
        let engine = match self.get_or_load_engine(&req.model, &model_path) {
            Ok(e) => e,
            Err(e) => return format!("[error] {}", e).into_bytes(),
        };

        // Call pure service function
        match inference::run_infer(engine.as_ref(), &req.prompt, req.max_tokens) {
            Ok(result) => result.into_bytes(),
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
    fn get_or_load_engine(&self, model_alias: &str, model_path: &crate::domain::ResourceId) -> Result<Arc<dyn InferenceEngine>, String> {
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
        let engine = open_inference_engine(&model)
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
    use crate::domain::{Agent, EngineType, InMemoryRegistry, Model, ModelType};
    use crate::queue::{InMemoryQueue, Message};
    use crate::inference::InMemoryInference;

    const TEST_AGENT_ID: &str = "file:///test-agent.wasm";

    fn test_model(name: &str) -> Model {
        // model_path must match the URI in the agent manifest
        Model {
            id: crate::domain::ResourceId::new(format!("http://127.0.0.1:9000/models/{}", name)),
            name: name.to_string(),
            model_type: ModelType::Inference,
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
            id = "{}"
            [requirements]
            services = []
            [requirements.models]
            {} = "memory://test/{}"
        "#, TEST_AGENT_ID, model_alias, model_alias);
        Agent::from_toml(&manifest).unwrap()
    }

    fn test_registry_with_agent_and_model(agent: Agent, model_name: &str) -> Arc<dyn Registry> {
        let registry = InMemoryRegistry::new();
        registry.register_runtime(crate::domain::RuntimeType::Wasm);
        registry.register_model(test_model(model_name));
        registry.register_agent(agent).unwrap();
        Arc::new(registry)
    }

    #[test]
    fn handles_infer_request() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = test_registry_with_agent_and_model(test_agent_with_model("test-model"), "test-model");
        let handler = InferenceServiceWorker::new(Arc::clone(&queue), registry, "memory");

        // Register mock engine
        let engine = Arc::new(InMemoryInference::new("test response"));
        handler.register("test-model", engine);

        // Send infer request
        let payload = serde_json::json!({
            "agent_id": TEST_AGENT_ID,
            "model": "test-model",
            "prompt": "Hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        // Use backend-qualified queue name (ADR 043)
        queue.send("vlinder.svc.infer.memory", msg).unwrap();

        // Process
        assert!(handler.tick());
        let pending = queue.receive("reply").unwrap();
        assert_eq!(String::from_utf8(pending.message.payload.clone()).unwrap(), "test response");
        pending.ack().unwrap();
    }

    #[test]
    fn rejects_undeclared_model() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        // Agent declares "allowed-model" but we'll request "other-model"
        let registry = test_registry_with_agent_and_model(test_agent_with_model("allowed-model"), "allowed-model");
        let handler = InferenceServiceWorker::new(Arc::clone(&queue), registry, "memory");

        // Register mock engine (the model exists, but agent didn't declare it)
        let engine = Arc::new(InMemoryInference::new("test response"));
        handler.register("other-model", engine);

        let payload = serde_json::json!({
            "agent_id": TEST_AGENT_ID,
            "model": "other-model",
            "prompt": "Hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        // Use backend-qualified queue name (ADR 043)
        queue.send("vlinder.svc.infer.memory", msg).unwrap();

        assert!(handler.tick());
        let pending = queue.receive("reply").unwrap();
        let text = String::from_utf8(pending.message.payload.clone()).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("did not declare model"));
        pending.ack().unwrap();
    }

    #[test]
    fn rejects_unknown_agent() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        // Registry with no agents registered
        let registry: Arc<dyn Registry> = Arc::new(InMemoryRegistry::new());
        let handler = InferenceServiceWorker::new(Arc::clone(&queue), registry, "memory");

        // Register mock engine
        let engine = Arc::new(InMemoryInference::new("test response"));
        handler.register("test-model", engine);

        let payload = serde_json::json!({
            "agent_id": "file:///unknown-agent.wasm",
            "model": "test-model",
            "prompt": "Hello"
        });
        let msg = Message::request(
            serde_json::to_vec(&payload).unwrap(),
            "reply",
        );
        // Use backend-qualified queue name (ADR 043)
        queue.send("vlinder.svc.infer.memory", msg).unwrap();

        assert!(handler.tick());
        let pending = queue.receive("reply").unwrap();
        let text = String::from_utf8(pending.message.payload.clone()).unwrap();
        assert!(text.contains("[error]"));
        assert!(text.contains("agent not found"));
        pending.ack().unwrap();
    }
}
