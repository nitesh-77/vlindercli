//! Daemon - the local-mode control plane that owns all system components.
//!
//! Like k8s:
//! - Owns Registry (etcd)
//! - Owns Harness (API Server)
//! - Owns Runtime (Kubelet)
//! - Owns Provider (services)
//! - Runs tick loop (controller reconciliation)
//!
//! This is the single-process, all-in-one execution model. For distributed
//! multi-process mode, see `Supervisor`.

use std::sync::Arc;

use crate::config::{registry_db_path, Config};
use crate::domain::{ObjectStorageType, PersistentRegistry, Provider, Runtime, RuntimeType, VectorStorageType};
use crate::domain::harness::CliHarness;
use crate::domain::registry::Registry;
use crate::queue;
use crate::runtime::ContainerRuntime;

/// The daemon - owns all system components for local (single-process) mode.
pub struct Daemon {
    #[allow(dead_code)] // Used via test-only accessor
    registry: Arc<dyn Registry>,
    /// The API surface - use this for deploy/invoke/poll.
    pub harness: CliHarness,
    container_runtime: ContainerRuntime,
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
        let config = Config::load();
        let queue = queue::from_config()
            .expect("Failed to create queue from config");
        let db_path = registry_db_path();
        let registry = PersistentRegistry::open(&db_path, &config)
            .unwrap_or_else(|e| panic!("Failed to initialize registry: {}", e));
        let registry_id = registry.id();

        // Register non-engine capabilities (engines are registered by open())
        registry.register_runtime(RuntimeType::Container);
        registry.register_object_storage(ObjectStorageType::Sqlite);
        registry.register_object_storage(ObjectStorageType::InMemory);
        registry.register_vector_storage(VectorStorageType::SqliteVec);
        registry.register_vector_storage(VectorStorageType::InMemory);

        let registry: Arc<dyn Registry> = Arc::new(registry);

        Self {
            harness: CliHarness::new(queue.clone(), Arc::clone(&registry)),
            container_runtime: ContainerRuntime::new(&registry_id, queue.clone(), Arc::clone(&registry)),
            registry: Arc::clone(&registry),
            provider: Provider::new(queue, registry),
        }
    }

    /// Tick all components.
    pub fn tick(&mut self) {
        self.container_runtime.tick();
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
    use crate::domain::harness::Harness;
    use crate::domain::{EngineType, Model, ModelType, RegistryRepository, ResourceId};
    use crate::storage::SqliteRegistryRepository;
    use tempfile::TempDir;

    #[test]
    fn deploy_rejects_agent_with_no_matching_runtime() {
        // Isolate from real registry.db
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("VLINDER_DIR", temp_dir.path());
        let daemon = Daemon::new();
        std::env::remove_var("VLINDER_DIR");

        // Agent with unknown runtime - no runtime supports this
        let manifest = r#"
            name = "remote-agent"
            description = "An agent hosted remotely"
            runtime = "lambda"
            executable = "arn:aws:lambda:us-east-1:123456789:function:my-agent"
            [requirements]
            services = []
        "#;

        let result = daemon.harness.deploy(manifest);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown runtime"));
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
