//! Daemon - the control plane that owns all system components.
//!
//! Like k8s:
//! - Owns Registry (etcd)
//! - Owns Harness (API Server)
//! - Owns Runtime (Kubelet)
//! - Owns Provider (services)
//! - Runs tick loop (controller reconciliation)
//!
//! ## Distributed Mode (ADR 043)
//!
//! When `distributed.enabled = true`, the daemon spawns separate worker
//! processes instead of running services in-process. Each worker is a
//! child process with `VLINDER_WORKER_ROLE` set.

use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{registry_db_path, Config};
use crate::domain::{EngineType, InMemoryRegistry, ObjectStorageType, Provider, RegistryRepository, Runtime, RuntimeType, VectorStorageType};
use crate::domain::harness::CliHarness;
use crate::domain::registry::Registry;
use crate::queue;
use crate::runtime::WasmRuntime;
use crate::storage::SqliteRegistryRepository;
use crate::worker_role::WorkerRole;

/// The daemon - owns all system components.
pub struct Daemon {
    // Components (daemon owns all of these)
    #[allow(dead_code)] // Used via test-only accessor
    registry: Arc<dyn Registry>,
    /// The API surface - use this for deploy/invoke/poll.
    pub harness: CliHarness,
    /// Runtime for local mode (None in distributed mode)
    runtime: Option<WasmRuntime>,
    /// Provider for local mode (None in distributed mode)
    provider: Option<Provider>,
    /// Child worker processes (distributed mode only)
    workers: Vec<Child>,
    /// Shutdown signal for graceful termination
    shutdown: Arc<AtomicBool>,
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

        if config.distributed.enabled {
            Self::new_distributed(&config)
        } else {
            Self::new_local(&config)
        }
    }

    /// Create daemon in local mode - all services run in-process.
    fn new_local(_config: &Config) -> Self {
        let queue = queue::from_config()
            .expect("Failed to create queue from config");
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
            harness: CliHarness::new(queue.clone(), Arc::clone(&registry)),
            runtime: Some(WasmRuntime::new(&registry_id, queue.clone(), Arc::clone(&registry))),
            registry: Arc::clone(&registry),
            provider: Some(Provider::new(queue, registry)),
            workers: vec![],
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create daemon in distributed mode - spawn worker processes.
    fn new_distributed(config: &Config) -> Self {
        let queue = queue::from_config()
            .expect("Failed to create queue from config");

        // In distributed mode, registry is minimal - workers connect via gRPC
        let registry = InMemoryRegistry::new();

        // Still register capabilities so harness knows what's available
        registry.register_runtime(RuntimeType::Wasm);
        registry.register_object_storage(ObjectStorageType::Sqlite);
        registry.register_object_storage(ObjectStorageType::InMemory);
        registry.register_vector_storage(VectorStorageType::SqliteVec);
        registry.register_vector_storage(VectorStorageType::InMemory);
        registry.register_inference_engine(EngineType::Ollama);
        registry.register_embedding_engine(EngineType::Ollama);

        Self::load_registered_models(&registry);

        let registry: Arc<dyn Registry> = Arc::new(registry);
        let shutdown = Arc::new(AtomicBool::new(false));

        // Spawn worker processes based on config
        let workers = Self::spawn_workers(config);

        tracing::info!(
            worker_count = workers.len(),
            "Daemon started in distributed mode"
        );

        Self {
            harness: CliHarness::new(queue, Arc::clone(&registry)),
            runtime: None, // Workers handle this
            registry,
            provider: None, // Workers handle this
            workers,
            shutdown,
        }
    }

    /// Spawn worker processes based on config.
    fn spawn_workers(config: &Config) -> Vec<Child> {
        let mut workers = Vec::new();
        let counts = &config.distributed.workers;

        // Registry workers (start first, others depend on it)
        for _ in 0..counts.registry {
            if let Some(child) = Self::spawn_worker(WorkerRole::Registry) {
                workers.push(child);
            }
        }

        // Give registry time to start before other workers connect
        if counts.registry > 0 {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        // Agent WASM workers
        for _ in 0..counts.agent.wasm {
            if let Some(child) = Self::spawn_worker(WorkerRole::AgentWasm) {
                workers.push(child);
            }
        }

        // Inference workers
        for _ in 0..counts.inference.ollama {
            if let Some(child) = Self::spawn_worker(WorkerRole::InferenceOllama) {
                workers.push(child);
            }
        }

        // Embedding workers
        for _ in 0..counts.embedding.ollama {
            if let Some(child) = Self::spawn_worker(WorkerRole::EmbeddingOllama) {
                workers.push(child);
            }
        }

        // Object storage workers
        for _ in 0..counts.storage.object.sqlite {
            if let Some(child) = Self::spawn_worker(WorkerRole::StorageObjectSqlite) {
                workers.push(child);
            }
        }
        for _ in 0..counts.storage.object.memory {
            if let Some(child) = Self::spawn_worker(WorkerRole::StorageObjectMemory) {
                workers.push(child);
            }
        }

        // Vector storage workers
        for _ in 0..counts.storage.vector.sqlite {
            if let Some(child) = Self::spawn_worker(WorkerRole::StorageVectorSqlite) {
                workers.push(child);
            }
        }
        for _ in 0..counts.storage.vector.memory {
            if let Some(child) = Self::spawn_worker(WorkerRole::StorageVectorMemory) {
                workers.push(child);
            }
        }

        workers
    }

    /// Spawn a single worker process with the given role.
    fn spawn_worker(role: WorkerRole) -> Option<Child> {
        use std::process::Stdio;

        let exe = std::env::current_exe().ok()?;

        tracing::debug!(role = %role, "Spawning worker");

        match Command::new(exe)
            .args(["daemon"])
            .env("VLINDER_WORKER_ROLE", role.as_env_value())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(child) => {
                tracing::info!(role = %role, pid = child.id(), "Worker spawned");
                Some(child)
            }
            Err(e) => {
                tracing::error!(role = %role, error = ?e, "Failed to spawn worker");
                None
            }
        }
    }

    /// Tick all components.
    ///
    /// In local mode, this drives all services. In distributed mode,
    /// workers handle their own tick loops - this just ticks the harness.
    pub fn tick(&mut self) {
        if let Some(ref mut runtime) = self.runtime {
            runtime.tick();
        }
        if let Some(ref provider) = self.provider {
            provider.tick();
        }
        self.harness.tick();
    }

    /// Signal shutdown and wait for workers to exit.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);

        // In distributed mode, terminate child workers
        for child in &mut self.workers {
            tracing::debug!(pid = child.id(), "Terminating worker");
            let _ = child.kill();
        }

        // Wait for all workers to exit
        for child in &mut self.workers {
            let _ = child.wait();
        }

        self.workers.clear();
        tracing::info!("Daemon shutdown complete");
    }

    /// Check if daemon is running in distributed mode.
    pub fn is_distributed(&self) -> bool {
        !self.workers.is_empty()
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
    use crate::domain::harness::Harness;
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
