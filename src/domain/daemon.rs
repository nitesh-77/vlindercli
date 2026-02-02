//! Daemon - the control plane that owns all system components.
//!
//! Like k8s:
//! - Owns Registry (etcd)
//! - Owns Harness (API Server)
//! - Owns Runtime (Kubelet)
//! - Owns Provider (services)
//! - Runs tick loop (controller reconciliation)

use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::domain::{EngineType, Model, ModelType, ObjectStorageType, Provider, Runtime, RuntimeType, VectorStorageType};
use crate::domain::harness::Harness;
use crate::domain::registry::Registry;
use crate::embedding::open_embedding_engine;
use crate::inference::open_inference_engine;
use crate::queue::InMemoryQueue;
use crate::runtime::WasmRuntime;

/// The daemon - owns all system components.
pub struct Daemon {
    // Components (daemon owns all of these)
    registry: Arc<RwLock<Registry>>,
    /// The API surface - use this for deploy/invoke/poll.
    pub harness: Harness,
    runtime: WasmRuntime,
    provider: Provider,
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

        let registry = Arc::new(RwLock::new(registry));

        Self {
            harness: Harness::new(queue.clone(), Arc::clone(&registry)),
            registry: Arc::clone(&registry),
            runtime,
            provider: Provider::new(queue, registry),
        }
    }

    /// Register a model with the system.
    ///
    /// Models must be registered before agents that depend on them can be deployed.
    pub fn register_model(&mut self, model_path: &Path) -> Result<(), String> {
        let model = Model::load(model_path)
            .map_err(|e| format!("failed to load model: {:?}", e))?;

        // Validate engine availability
        {
            let registry = self.registry.read().unwrap();
            match model.model_type {
                ModelType::Inference => {
                    if !registry.has_inference_engine(model.engine) {
                        return Err(format!("inference engine not available: {:?}", model.engine));
                    }
                }
                ModelType::Embedding => {
                    if !registry.has_embedding_engine(model.engine) {
                        return Err(format!("embedding engine not available: {:?}", model.engine));
                    }
                }
            }
        }

        // Open engine and register with provider
        let model_name = model.name.clone();
        match model.model_type {
            ModelType::Inference => {
                let engine = open_inference_engine(&model)
                    .map_err(|e| format!("failed to open inference engine: {}", e))?;
                self.provider.register_inference(&model_name, engine);
            }
            ModelType::Embedding => {
                let engine = open_embedding_engine(&model)
                    .map_err(|e| format!("failed to open embedding engine: {}", e))?;
                self.provider.register_embedding(&model_name, engine);
            }
        }

        // Register model in registry
        self.registry.write().unwrap().register_model(model);

        Ok(())
    }

    /// Register an already-deployed agent with the runtime.
    ///
    /// Call this after harness.deploy() to enable agent execution.
    pub fn activate_agent(&mut self, agent_id: &crate::domain::ResourceId) -> Result<(), String> {
        let registry = self.registry.read().unwrap();
        let agent = registry.get_agent(agent_id)
            .ok_or_else(|| format!("agent not found: {}", agent_id))?;

        // Register with runtime based on type
        let runtime_type = registry.select_runtime(agent)
            .ok_or_else(|| format!("no runtime for agent: {}", agent_id))?;

        match runtime_type {
            RuntimeType::Wasm => self.runtime.register(agent.clone()),
        }

        Ok(())
    }

    /// Tick all components.
    pub fn tick(&mut self) {
        self.runtime.tick();
        self.provider.tick();
        self.harness.tick();
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
    fn deploy_rejects_agent_with_no_matching_runtime() {
        let daemon = Daemon::new();

        // Agent with http:// scheme - no runtime supports this
        let manifest = r#"
            name = "remote-agent"
            description = "An agent hosted remotely"
            id = "http://example.com/agent.wasm"
            [requirements]
            services = []
        "#;

        let result = daemon.harness.deploy(manifest);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no runtime"));
    }
}
