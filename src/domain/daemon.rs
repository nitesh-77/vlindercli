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

use crate::domain::{Agent, Model, ModelType, ObjectStorageType, Provider, Runtime, RuntimeType};
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
        // Select runtime first - fail fast if no suitable runtime
        let runtime_type = self.registry.select_runtime(agent)
            .ok_or_else(|| format!("no runtime available for agent: {}", agent.id.as_str()))?;

        let name = &agent.name;

        // Storage
        let storage = in_memory_storage();
        let object = open_object_storage(&storage).expect("in-memory always succeeds");
        let vector = open_vector_storage(&storage).expect("in-memory always succeeds");
        self.provider.register_object(name, object);
        self.provider.register_vector(name, vector);

        // Models
        for (model_name, model_uri) in &agent.requirements.models {
            self.register_model(model_name, model_uri.as_str());
        }

        // Register with selected runtime
        match runtime_type {
            RuntimeType::Wasm => self.runtime.register(agent.clone()),
        }

        Ok(())
    }

    fn register_model(&mut self, model_name: &str, model_uri: &str) {
        let path = model_uri.strip_prefix("file://").unwrap_or(model_uri);

        let model = match Model::load(Path::new(path)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[warning] Failed to load model {}: {}", model_name, e);
                return;
            }
        };

        match model.model_type {
            ModelType::Inference => {
                let engine = open_inference_engine(&model).unwrap_or_else(|_| {
                    Arc::new(InMemoryInference::new("[placeholder]"))
                });
                self.provider.register_inference(model_name, engine);
            }
            ModelType::Embedding => {
                let engine = open_embedding_engine(&model).unwrap_or_else(|_| {
                    let canned: Vec<f32> = (0..768).map(|i| i as f32 * 0.001).collect();
                    Arc::new(InMemoryEmbedding::new(canned))
                });
                self.provider.register_embedding(model_name, engine);
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
    use std::path::PathBuf;

    fn fixture_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    fn reverse_agent_toml() -> String {
        let wasm_path = fixture_path("agents/reverse-agent/agent.wasm");
        format!(
            r#"
            name = "reverse-agent"
            description = "Reverses input"
            id = "file://{}"
            [requirements]
            services = []
            "#,
            wasm_path.display()
        )
    }

    #[test]
    fn daemon_owns_harness() {
        let mut daemon = Daemon::new();

        // Invoke via daemon (which delegates to harness)
        let job_id = daemon.invoke(&reverse_agent_toml(), "hello").unwrap();

        // Tick until complete
        let result = loop {
            daemon.tick();
            if let Some(result) = daemon.poll(&job_id) {
                break result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };

        assert_eq!(result, "olleh");
    }

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
