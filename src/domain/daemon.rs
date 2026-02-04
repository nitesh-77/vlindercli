//! Daemon - the control plane that owns all system components.
//!
//! Like k8s:
//! - Owns Registry (etcd)
//! - Owns Harness (API Server)
//! - Owns Runtime (Kubelet)
//! - Owns Provider (services)
//! - Runs tick loop (controller reconciliation)

use std::sync::Arc;

use crate::config::registry_db_path;
use crate::domain::{EngineType, InMemoryRegistry, ObjectStorageType, Provider, RegistryRepository, Runtime, RuntimeType, VectorStorageType};
use crate::domain::harness::Harness;
use crate::domain::registry::Registry;
use crate::queue::InMemoryQueue;
use crate::runtime::WasmRuntime;
use crate::storage::SqliteRegistryRepository;

/// The daemon - owns all system components.
pub struct Daemon {
    // Components (daemon owns all of these)
    #[allow(dead_code)] // Used via test-only accessor
    registry: Arc<dyn Registry>,
    /// The API surface - use this for deploy/invoke/poll.
    pub harness: Harness,
    runtime: WasmRuntime,
    provider: Provider,
}

#[cfg(test)]
impl Daemon {
    /// Test-only accessor for the registry.
    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }
}

impl Daemon {
    pub fn new() -> Self {
        let queue = Arc::new(InMemoryQueue::new());
        let registry = InMemoryRegistry::new();
        let registry_id = registry.id();

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
        registry.register_inference_engine(EngineType::Ollama);
        registry.register_inference_engine(EngineType::InMemory);

        // Register available embedding engine implementations
        registry.register_embedding_engine(EngineType::Llama);
        registry.register_embedding_engine(EngineType::Ollama);
        registry.register_embedding_engine(EngineType::InMemory);

        // Load registered models from persistent storage
        Self::load_registered_models(&registry);

        let registry: Arc<dyn Registry> = Arc::new(registry);

        Self {
            harness: Harness::new(queue.clone(), Arc::clone(&registry)),
            runtime: WasmRuntime::new(&registry_id, queue.clone(), Arc::clone(&registry)),
            registry: Arc::clone(&registry),
            provider: Provider::new(queue, registry),
        }
    }

    /// Tick all components.
    pub fn tick(&mut self) {
        self.runtime.tick();
        self.provider.tick();
        self.harness.tick();
    }

    /// Load models from the persistent registry.
    fn load_registered_models(registry: &dyn Registry) {
        let db_path = registry_db_path();
        if !db_path.exists() {
            return;
        }

        let repo = match SqliteRegistryRepository::open(&db_path) {
            Ok(r) => r,
            Err(_) => return,
        };

        if let Ok(models) = repo.load_models() {
            for model in models {
                registry.register_model(model);
            }
        }
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
    use crate::domain::{Model, ModelType, ResourceId};
    use tempfile::TempDir;

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

    #[test]
    fn loads_models_from_registry_db() {
        // Create a temp directory with a registry.db
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("registry.db");

        // Pre-populate the registry
        {
            let repo = SqliteRegistryRepository::open(&db_path).unwrap();
            let model = Model {
                id: Model::placeholder_id("test-model"),
                name: "test-model".to_string(),
                model_type: ModelType::Inference,
                engine: EngineType::Ollama,
                model_path: ResourceId::new("ollama://localhost:11434/test-model"),
                digest: "sha256:test-digest".to_string(),
            };
            repo.save_model(&model).unwrap();
        }

        // Set VLINDER_DIR to temp directory
        std::env::set_var("VLINDER_DIR", temp_dir.path());

        let daemon = Daemon::new();

        // Clean up env
        std::env::remove_var("VLINDER_DIR");

        // Check that the model was loaded
        let model = daemon.registry().get_model("test-model");
        assert!(model.is_some(), "test-model should be loaded from registry.db");
        assert_eq!(model.unwrap().name, "test-model");
    }
}
