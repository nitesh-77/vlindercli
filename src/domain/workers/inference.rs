//! Inference Service Handler - LLM inference over queues.
//!
//! Queue:
//! - `infer`: Run inference with a model
//!
//! Engines are lazy-loaded on first use from Registry model metadata.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::domain::{InferenceEngine, InferenceResult};
use crate::domain::registry::Registry;
use crate::domain::service_payloads::InferRequest;
use crate::inference::open_inference_engine;
use crate::domain::{MessageQueue, Operation, RequestMessage, ResponseMessage, ServiceDiagnostics, ServiceMetrics, ServiceType};
use crate::services::inference;

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
        // Receive typed RequestMessage (ADR 044)
        match self.queue.receive_request(ServiceType::Infer, &self.backend, Operation::Run) {
            Ok((request, ack)) => {
                let model = self.extract_model_name(&request);
                let start = Instant::now();
                let result = self.handle_infer(&request);
                let duration_ms = start.elapsed().as_millis() as u64;

                let (response_payload, tokens_input, tokens_output) = match result {
                    Ok(r) => (r.text.into_bytes(), r.tokens_input, r.tokens_output),
                    Err(err_bytes) => (err_bytes, 0, 0),
                };

                let diag = ServiceDiagnostics {
                    service: ServiceType::Infer,
                    backend: self.backend.clone(),
                    duration_ms,
                    metrics: ServiceMetrics::Inference {
                        tokens_input,
                        tokens_output,
                        model,
                    },
                };
                let mut response = ResponseMessage::from_request_with_diagnostics(
                    &request, response_payload, diag,
                );
                response.state = request.state.clone();
                let _ = self.queue.send_response(response);
                let _ = ack();
                true
            }
            Err(_) => false,
        }
    }

    /// Best-effort extraction of model name from request payload.
    fn extract_model_name(&self, request: &RequestMessage) -> String {
        serde_json::from_slice::<InferRequest>(&request.payload)
            .map(|r| r.model)
            .unwrap_or_default()
    }

    fn handle_infer(&self, request: &RequestMessage) -> Result<InferenceResult, Vec<u8>> {
        let req: InferRequest = serde_json::from_slice(&request.payload)
            .map_err(|e| format!("[error] invalid request: {}", e).into_bytes())?;

        // Resolve model alias to model_path via agent's manifest
        let model_path = self.resolve_model_uri(request.agent_id.as_str(), &req.model)
            .map_err(|e| format!("[error] {}", e).into_bytes())?;

        // Try to get cached engine, or lazy-load from registry
        let engine = self.get_or_load_engine(&req.model, &model_path)
            .map_err(|e| format!("[error] {}", e).into_bytes())?;

        // Call pure service function
        inference::run_infer(engine.as_ref(), &req.prompt, req.max_tokens)
            .map_err(|e| format!("[error] {}", e).into_bytes())
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
    use crate::domain::{Agent, Provider, Model, ModelType, ResourceId};
    use crate::registry::InMemoryRegistry;
    use crate::domain::{Operation, RequestDiagnostics, Sequence, ServiceType, SessionId, SubmissionId, TimelineId};
    use crate::domain::SecretStore;
    use crate::secret_store::InMemorySecretStore;
    use crate::queue::InMemoryQueue;
    use crate::inference::InMemoryInference;

    fn test_secret_store() -> Arc<dyn SecretStore> {
        Arc::new(InMemorySecretStore::new())
    }

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
            model_type: ModelType::Inference,
            provider: Provider::InMemory,
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

            [requirements.models]
            {} = "memory://test/{}"

            [requirements.services.infer]
            provider = "ollama"
            protocol = "openai"
            models = ["{}"]
        "#, model_alias, model_alias, model_alias);
        Agent::from_toml(&manifest).unwrap()
    }

    fn test_registry_with_agent_and_model(agent: Agent, model_name: &str) -> Arc<dyn Registry> {
        let registry = InMemoryRegistry::new(test_secret_store());
        registry.register_runtime(crate::domain::RuntimeType::Container);
        registry.register_inference_engine(Provider::InMemory);
        registry.register_model(test_model(model_name)).unwrap();
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

        // Send typed RequestMessage (ADR 044)
        let payload = serde_json::json!({
            "model": "test-model",
            "prompt": "Hello"
        });
        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceType::Infer,
            "memory",
            Operation::Run,
            Sequence::first(),
            serde_json::to_vec(&payload).unwrap(),
            None,
            test_request_diag(),
        );

        queue.send_request(request.clone()).unwrap();

        // Process
        assert!(handler.tick());

        // Response is sent via typed queue
        let (response, ack) = queue.receive_response(&request).unwrap();
        assert_eq!(String::from_utf8(response.payload.clone()).unwrap(), "test response");
        ack().unwrap();
    }

    #[test]
    fn infer_response_echoes_state() {
        let queue: Arc<dyn MessageQueue + Send + Sync> = Arc::new(InMemoryQueue::new());
        let registry = test_registry_with_agent_and_model(test_agent_with_model("test-model"), "test-model");
        let handler = InferenceServiceWorker::new(Arc::clone(&queue), registry, "memory");

        let engine = Arc::new(InMemoryInference::new("test response"));
        handler.register("test-model", engine);

        let payload = serde_json::json!({
            "model": "test-model",
            "prompt": "Hello"
        });
        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceType::Infer,
            "memory",
            Operation::Run,
            Sequence::first(),
            serde_json::to_vec(&payload).unwrap(),
            Some("state-abc".to_string()),
            test_request_diag(),
        );

        queue.send_request(request.clone()).unwrap();
        assert!(handler.tick());

        let (response, ack) = queue.receive_response(&request).unwrap();
        ack().unwrap();
        assert_eq!(response.state, Some("state-abc".to_string()), "infer should echo request.state");
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
            "model": "other-model",
            "prompt": "Hello"
        });
        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            test_agent_id(),
            ServiceType::Infer,
            "memory",
            Operation::Run,
            Sequence::first(),
            serde_json::to_vec(&payload).unwrap(),
            None,
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
        let registry: Arc<dyn Registry> = Arc::new(InMemoryRegistry::new(test_secret_store()));
        let handler = InferenceServiceWorker::new(Arc::clone(&queue), registry, "memory");

        // Register mock engine
        let engine = Arc::new(InMemoryInference::new("test response"));
        handler.register("test-model", engine);

        let payload = serde_json::json!({
            "model": "test-model",
            "prompt": "Hello"
        });
        let request = RequestMessage::new(
            TimelineId::main(),
            test_submission(),
            SessionId::new(),
            ResourceId::new("http://127.0.0.1:9000/agents/unknown-agent"),
            ServiceType::Infer,
            "memory",
            Operation::Run,
            Sequence::first(),
            serde_json::to_vec(&payload).unwrap(),
            None,
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
