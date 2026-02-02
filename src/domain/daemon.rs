//! Daemon - the control plane that owns all system components.
//!
//! Like k8s:
//! - Owns Registry (etcd)
//! - Owns Harness (API Server)
//! - Owns Runtime (Kubelet)
//! - Owns Provider (services)
//! - Runs tick loop (controller reconciliation)

use std::path::Path;
use std::sync::Arc;

use crate::domain::{Agent, EngineType, Model, ModelType, ObjectStorageType, Provider, Runtime, RuntimeType, VectorStorageType};
use crate::domain::harness::Harness;
use crate::domain::registry::{JobId, Registry};
use crate::embedding::{open_embedding_engine, InMemoryEmbedding};
use crate::inference::{open_inference_engine, InMemoryInference};
use crate::queue::InMemoryQueue;
use crate::runtime::WasmRuntime;
use crate::storage::dispatch::{in_memory_storage, open_object_storage, open_vector_storage};

/// The daemon - owns all system components.
pub struct Daemon {
    // Components (daemon owns all of these)
    registry: Registry,
    harness: Harness,
    runtime: WasmRuntime,
    provider: Provider,

    // Shared queue
    queue: Arc<InMemoryQueue>,
}

impl Daemon {
    pub fn new() -> Self {
        let queue = Arc::new(InMemoryQueue::new());
        let mut registry = Registry::new();
        let runtime = WasmRuntime::new(&registry.id, queue.clone());

        // Register available runtimes
        registry.register_runtime(RuntimeType::Wasm);

        // Register available object storage implementations
        registry.register_object_storage(ObjectStorageType::Sqlite);
        registry.register_object_storage(ObjectStorageType::InMemory);

        // Register available vector storage implementations
        registry.register_vector_storage(VectorStorageType::SqliteVec);
        registry.register_vector_storage(VectorStorageType::InMemory);

        // Register available inference engine implementations
        registry.register_inference_engine(EngineType::Llama);
        registry.register_inference_engine(EngineType::InMemory);

        // Register available embedding engine implementations
        registry.register_embedding_engine(EngineType::Llama);
        registry.register_embedding_engine(EngineType::InMemory);

        Self {
            registry,
            harness: Harness::new(queue.clone()),
            runtime,
            provider: Provider::new(queue.clone()),
            queue,
        }
    }

    // ========================================================================
    // API (delegates to Harness)
    // ========================================================================

    /// Submit an agent manifest and input for execution.
    pub fn invoke(&mut self, manifest_toml: &str, input: &str) -> Result<JobId, String> {
        // Parse agent
        let agent = Agent::from_toml(manifest_toml)
            .map_err(|e| format!("failed to parse manifest: {:?}", e))?;

        let agent_id = agent.id.clone();

        // Register infrastructure if new agent
        if self.registry.get_agent(&agent_id).is_none() {
            self.register_agent_infrastructure(&agent)?;
            self.registry.register_agent(agent);
        }

        // Delegate to harness
        self.harness.invoke(&mut self.registry, &agent_id, input)
    }

    /// Poll for job completion.
    pub fn poll(&self, job_id: &JobId) -> Option<String> {
        self.harness.poll(&self.registry, job_id)
    }

    // ========================================================================
    // Control Loop
    // ========================================================================

    /// Tick all components.
    pub fn tick(&mut self) {
        self.runtime.tick();
        self.provider.tick();
        self.harness.tick(&mut self.registry);
    }

    // ========================================================================
    // Infrastructure Setup
    // ========================================================================

    fn register_agent_infrastructure(&mut self, agent: &Agent) -> Result<(), String> {
        // Validate runtime
        let runtime_type = self.registry.select_runtime(agent)
            .ok_or_else(|| format!("no runtime available for agent: {}", agent.id.as_str()))?;

        // Validate object storage if declared
        if let Some(ref storage_id) = agent.object_storage {
            let storage_type = ObjectStorageType::from_scheme(storage_id.scheme())
                .ok_or_else(|| format!("unknown object storage scheme: {:?}", storage_id.scheme()))?;
            if !self.registry.has_object_storage(storage_type) {
                return Err(format!("object storage not available: {:?}", storage_type));
            }
        }

        // Validate vector storage if declared
        if let Some(ref storage_id) = agent.vector_storage {
            let storage_type = VectorStorageType::from_scheme(storage_id.scheme())
                .ok_or_else(|| format!("unknown vector storage scheme: {:?}", storage_id.scheme()))?;
            if !self.registry.has_vector_storage(storage_type) {
                return Err(format!("vector storage not available: {:?}", storage_type));
            }
        }

        // Validate and register models
        for (model_name, model_uri) in &agent.requirements.models {
            self.validate_and_register_model(model_name, model_uri.as_str())?;
        }

        // Setup storage (use in-memory for now, TODO: use declared storage)
        let name = &agent.name;
        let storage = in_memory_storage();
        let object = open_object_storage(&storage).expect("in-memory always succeeds");
        let vector = open_vector_storage(&storage).expect("in-memory always succeeds");
        self.provider.register_object(name, object);
        self.provider.register_vector(name, vector);

        // Register with selected runtime
        match runtime_type {
            RuntimeType::Wasm => self.runtime.register(agent.clone()),
        }

        Ok(())
    }

    fn validate_and_register_model(&mut self, model_name: &str, model_uri: &str) -> Result<(), String> {
        let path = model_uri.strip_prefix("file://").unwrap_or(model_uri);

        let model = Model::load(Path::new(path))
            .map_err(|e| format!("failed to load model {}: {:?}", model_name, e))?;

        // Validate engine availability and register
        match model.model_type {
            ModelType::Inference => {
                if !self.registry.has_inference_engine(model.engine) {
                    return Err(format!("inference engine not available: {:?}", model.engine));
                }
                let engine = open_inference_engine(&model)
                    .map_err(|e| format!("failed to open inference engine: {}", e))?;
                self.provider.register_inference(model_name, engine);
            }
            ModelType::Embedding => {
                if !self.registry.has_embedding_engine(model.engine) {
                    return Err(format!("embedding engine not available: {:?}", model.engine));
                }
                let engine = open_embedding_engine(&model)
                    .map_err(|e| format!("failed to open embedding engine: {}", e))?;
                self.provider.register_embedding(model_name, engine);
            }
        }

        Ok(())
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_rejects_agent_with_no_matching_runtime() {
        let mut daemon = Daemon::new();

        // Agent with http:// scheme - no runtime supports this
        let manifest = r#"
            name = "remote-agent"
            description = "An agent hosted remotely"
            id = "http://example.com/agent.wasm"
            [requirements]
            services = []
        "#;

        let result = daemon.invoke(manifest, "hello");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no runtime available"));
    }
}
