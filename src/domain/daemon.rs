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

use crate::domain::{EngineType, InMemoryRegistry, Model, ObjectStorageType, Provider, Runtime, RuntimeType, VectorStorageType};
use crate::domain::harness::Harness;
use crate::domain::registry::Registry;
use crate::queue::InMemoryQueue;
use crate::runtime::WasmRuntime;

/// The daemon - owns all system components.
pub struct Daemon {
    // Components (daemon owns all of these)
    registry: Arc<dyn Registry>,
    /// The API surface - use this for deploy/invoke/poll.
    pub harness: Harness,
    runtime: WasmRuntime,
    provider: Provider,
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

        // Register known models (hardcoded for now)
        Self::register_known_models(&registry);

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

    /// Register known models (hardcoded paths for now).
    fn register_known_models(registry: &dyn Registry) {
        let model_paths = [
            "/Users/ashwnacharya/repos/vlindercli/.vlinder/models/phi3/model.toml",
            "/Users/ashwnacharya/repos/vlindercli/.vlinder/models/nomic-embed/model.toml",
            "/Users/ashwnacharya/repos/vlindercli/.vlinder/models/qwen/model.toml",
        ];

        for path in model_paths {
            let manifest_path = Path::new(path);
            if manifest_path.exists() {
                if let Ok(model) = Model::load(manifest_path) {
                    registry.register_model(model);
                }
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
    fn registers_known_models() {
        let daemon = Daemon::new();

        // Check that known models are registered
        let phi3 = daemon.registry.get_model("phi3");
        assert!(phi3.is_some(), "phi3 should be registered");
        assert_eq!(phi3.unwrap().name, "phi3");

        let nomic = daemon.registry.get_model("nomic-embed");
        assert!(nomic.is_some(), "nomic-embed should be registered");

        let qwen = daemon.registry.get_model("qwen");
        assert!(qwen.is_some(), "qwen should be registered");
    }
}
