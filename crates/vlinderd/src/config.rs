//! Application configuration with env override support.
//!
//! Resolution order (highest priority first):
//! 1. Environment variable: VLINDER_<SECTION>_<KEY>
//! 2. Config file: ~/.vlinder/config.toml
//! 3. Default value
//!
//! The `ConfigLoader` trait allows custom loading strategies for testing
//! or alternative config sources.

use serde::Deserialize;
use std::path::PathBuf;

// ============================================================================
// Config Loader Trait
// ============================================================================

/// Trait for loading configuration.
///
/// Implementations can load from different sources:
/// - `DefaultLoader`: File + env overrides (production)
/// - `TestLoader`: In-memory config (testing)
/// - Future: Remote config services
pub trait ConfigLoader: Send + Sync {
    fn load(&self) -> Config;
}

/// Default loader: file + env overrides.
pub struct DefaultLoader;

impl ConfigLoader for DefaultLoader {
    fn load(&self) -> Config {
        let mut config = Config::load_from_file();
        config.apply_env_overrides();
        config
    }
}

/// Test loader: returns a pre-configured Config.
#[cfg(test)]
pub struct TestLoader(pub Config);

#[cfg(test)]
impl ConfigLoader for TestLoader {
    fn load(&self) -> Config {
        self.0.clone()
    }
}

// ============================================================================
// Config Struct (Typed)
// ============================================================================

/// Application configuration loaded from ~/.vlinder/config.toml
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub logging: LoggingConfig,
    pub ollama: OllamaConfig,
    pub openrouter: OpenRouterConfig,
    pub queue: QueueConfig,
    pub distributed: DistributedConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// App log level: trace, debug, info, warn, error
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    /// Ollama server endpoint
    pub endpoint: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OpenRouterConfig {
    /// OpenRouter API endpoint
    pub endpoint: String,
    /// API key for authentication
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QueueConfig {
    /// Queue backend: "memory" or "nats"
    pub backend: String,
    /// NATS server URL (if backend = "nats")
    pub nats_url: String,
}

// ============================================================================
// Distributed Config (ADR 043)
// ============================================================================

/// Configuration for distributed multi-process deployments.
///
/// When `enabled = true`, the daemon spawns separate worker processes
/// for each service type. Workers communicate via NATS queues.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DistributedConfig {
    /// Enable distributed mode (spawn worker processes)
    pub enabled: bool,
    /// Registry gRPC address for worker coordination
    pub registry_addr: String,
    /// State service gRPC address (ADR 079)
    pub state_addr: String,
    /// Harness gRPC address for CLI→daemon agent invocation
    pub harness_addr: String,
    /// Secret store gRPC address
    pub secret_addr: String,
    /// Catalog service gRPC address
    pub catalog_addr: String,
    /// Worker process counts by type
    pub workers: WorkerCounts,
}

/// Worker process counts for each service type.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WorkerCounts {
    /// Number of registry worker processes
    pub registry: u32,
    /// Number of harness worker processes
    pub harness: u32,
    /// Agent runtime workers
    pub agent: AgentWorkerCounts,
    /// Inference service workers
    pub inference: InferenceWorkerCounts,
    /// Embedding service workers
    pub embedding: EmbeddingWorkerCounts,
    /// Storage service workers
    pub storage: StorageWorkerCounts,
}

/// Agent runtime worker counts by backend.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgentWorkerCounts {
    /// Container runtime workers (Podman)
    pub container: u32,
}

/// Inference worker counts by engine.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct InferenceWorkerCounts {
    /// Ollama inference workers
    pub ollama: u32,
    /// OpenRouter inference workers
    pub openrouter: u32,
}

/// Embedding worker counts by engine.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EmbeddingWorkerCounts {
    /// Ollama embedding workers
    pub ollama: u32,
}

/// Storage worker counts by backend.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StorageWorkerCounts {
    /// Object storage workers
    pub object: ObjectStorageWorkerCounts,
    /// Vector storage workers
    pub vector: VectorStorageWorkerCounts,
}

/// Object storage worker counts by backend.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ObjectStorageWorkerCounts {
    /// SQLite object storage workers
    pub sqlite: u32,
    /// In-memory object storage workers
    pub memory: u32,
}

/// Vector storage worker counts by backend.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VectorStorageWorkerCounts {
    /// SQLite-vec vector storage workers
    pub sqlite: u32,
    /// In-memory vector storage workers
    pub memory: u32,
}

// ============================================================================
// Runtime Config (ADR 073)
// ============================================================================

/// Container runtime configuration.
///
/// Controls how the container runtime resolves OCI images and connects to Podman.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    /// Image resolution policy: "mutable" (default) or "pinned".
    ///
    /// - `mutable`: Uses the tag from `agent.executable` — rebuilt images are
    ///   picked up automatically on next container start. Default for development.
    /// - `pinned`: Uses the content-addressed digest from `agent.image_digest` —
    ///   deterministic execution with the exact image from registration time.
    pub image_policy: String,

    /// Podman socket path: "auto" (probe filesystem), "disabled" (CLI only),
    /// or an absolute path to the socket file.
    pub podman_socket: String,
}

// ============================================================================
// Defaults
// ============================================================================

impl Default for Config {
    fn default() -> Self {
        Self {
            logging: LoggingConfig::default(),
            ollama: OllamaConfig::default(),
            openrouter: OpenRouterConfig::default(),
            queue: QueueConfig::default(),
            distributed: DistributedConfig::default(),
            runtime: RuntimeConfig::default(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "warn".to_string(),
        }
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
        }
    }
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://openrouter.ai/api/v1".to_string(),
            api_key: String::new(),
        }
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            backend: "memory".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
        }
    }
}

impl Default for DistributedConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            registry_addr: "http://127.0.0.1:9090".to_string(),
            state_addr: "http://127.0.0.1:9092".to_string(),
            harness_addr: "http://127.0.0.1:9091".to_string(),
            secret_addr: "http://127.0.0.1:9093".to_string(),
            catalog_addr: "http://127.0.0.1:9094".to_string(),
            workers: WorkerCounts::default(),
        }
    }
}

impl Default for WorkerCounts {
    fn default() -> Self {
        Self {
            registry: 1,
            harness: 1,
            agent: AgentWorkerCounts::default(),
            inference: InferenceWorkerCounts::default(),
            embedding: EmbeddingWorkerCounts::default(),
            storage: StorageWorkerCounts::default(),
        }
    }
}

impl Default for AgentWorkerCounts {
    fn default() -> Self {
        Self { container: 1 }
    }
}

impl Default for InferenceWorkerCounts {
    fn default() -> Self {
        Self { ollama: 1, openrouter: 0 }
    }
}

impl Default for EmbeddingWorkerCounts {
    fn default() -> Self {
        Self { ollama: 1 }
    }
}

impl Default for StorageWorkerCounts {
    fn default() -> Self {
        Self {
            object: ObjectStorageWorkerCounts::default(),
            vector: VectorStorageWorkerCounts::default(),
        }
    }
}

impl Default for ObjectStorageWorkerCounts {
    fn default() -> Self {
        Self { sqlite: 1, memory: 0 }
    }
}

impl Default for VectorStorageWorkerCounts {
    fn default() -> Self {
        Self { sqlite: 1, memory: 0 }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            image_policy: "mutable".to_string(),
            podman_socket: "auto".to_string(),
        }
    }
}

// ============================================================================
// Config Implementation
// ============================================================================

impl Config {
    /// Load config using the default loader (file + env).
    pub fn load() -> Self {
        DefaultLoader.load()
    }

    /// Load config using a custom loader.
    pub fn load_with(loader: &dyn ConfigLoader) -> Self {
        loader.load()
    }

    /// Load config from file only (no env overrides).
    fn load_from_file() -> Self {
        let config_path = config_path();
        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    /// Apply environment variable overrides.
    fn apply_env_overrides(&mut self) {
        // Logging
        if let Ok(v) = std::env::var("VLINDER_LOGGING_LEVEL") {
            self.logging.level = v;
        }
        // Ollama
        if let Ok(v) = std::env::var("VLINDER_OLLAMA_ENDPOINT") {
            self.ollama.endpoint = v;
        }

        // OpenRouter
        if let Ok(v) = std::env::var("VLINDER_OPENROUTER_ENDPOINT") {
            self.openrouter.endpoint = v;
        }
        if let Ok(v) = std::env::var("VLINDER_OPENROUTER_API_KEY") {
            self.openrouter.api_key = v;
        }

        // Queue
        if let Ok(v) = std::env::var("VLINDER_QUEUE_BACKEND") {
            self.queue.backend = v;
        }
        if let Ok(v) = std::env::var("VLINDER_QUEUE_NATS_URL") {
            self.queue.nats_url = v;
        }

        // Distributed
        if let Ok(v) = std::env::var("VLINDER_DISTRIBUTED_ENABLED") {
            self.distributed.enabled = v.parse().unwrap_or(false);
        }
        if let Ok(v) = std::env::var("VLINDER_DISTRIBUTED_REGISTRY_ADDR") {
            self.distributed.registry_addr = v;
        }
        if let Ok(v) = std::env::var("VLINDER_DISTRIBUTED_STATE_ADDR") {
            self.distributed.state_addr = v;
        }
        if let Ok(v) = std::env::var("VLINDER_DISTRIBUTED_HARNESS_ADDR") {
            self.distributed.harness_addr = v;
        }
        if let Ok(v) = std::env::var("VLINDER_DISTRIBUTED_SECRET_ADDR") {
            self.distributed.secret_addr = v;
        }
        if let Ok(v) = std::env::var("VLINDER_DISTRIBUTED_CATALOG_ADDR") {
            self.distributed.catalog_addr = v;
        }

        // Worker counts (flat env vars for simplicity)
        if let Ok(v) = std::env::var("VLINDER_WORKERS_REGISTRY") {
            self.distributed.workers.registry = v.parse().unwrap_or(1);
        }
        if let Ok(v) = std::env::var("VLINDER_WORKERS_HARNESS") {
            self.distributed.workers.harness = v.parse().unwrap_or(1);
        }
        if let Ok(v) = std::env::var("VLINDER_WORKERS_AGENT_CONTAINER") {
            self.distributed.workers.agent.container = v.parse().unwrap_or(0);
        }
        if let Ok(v) = std::env::var("VLINDER_WORKERS_INFERENCE_OLLAMA") {
            self.distributed.workers.inference.ollama = v.parse().unwrap_or(1);
        }
        if let Ok(v) = std::env::var("VLINDER_WORKERS_INFERENCE_OPENROUTER") {
            self.distributed.workers.inference.openrouter = v.parse().unwrap_or(0);
        }
        if let Ok(v) = std::env::var("VLINDER_WORKERS_EMBEDDING_OLLAMA") {
            self.distributed.workers.embedding.ollama = v.parse().unwrap_or(1);
        }
        if let Ok(v) = std::env::var("VLINDER_WORKERS_STORAGE_OBJECT_SQLITE") {
            self.distributed.workers.storage.object.sqlite = v.parse().unwrap_or(1);
        }
        if let Ok(v) = std::env::var("VLINDER_WORKERS_STORAGE_VECTOR_SQLITE") {
            self.distributed.workers.storage.vector.sqlite = v.parse().unwrap_or(1);
        }
        // Runtime (ADR 073, ADR 077)
        if let Ok(v) = std::env::var("VLINDER_RUNTIME_IMAGE_POLICY") {
            self.runtime.image_policy = v;
        }
        if let Ok(v) = std::env::var("VLINDER_RUNTIME_PODMAN_SOCKET") {
            self.runtime.podman_socket = v;
        }
    }

    /// Build tracing EnvFilter from config.
    ///
    /// The configured level applies to vlinderd only. External crates
    /// (async_nats, tonic, hyper, etc.) are suppressed to warn so they
    /// don't pollute the user's terminal.
    pub fn tracing_filter(&self) -> String {
        format!(
            "warn,vlinderd={}",
            self.logging.level
        )
    }
}

// ============================================================================
// Path helpers
// ============================================================================

/// Path to config file.
pub fn config_path() -> PathBuf {
    vlinder_dir().join("config.toml")
}

/// Base directory for vlinder data (agents, models, storage).
///
/// Resolution order:
/// 1. Env var VLINDER_DIR (for project-local or CI)
/// 2. Default: ~/.vlinder
pub fn vlinder_dir() -> PathBuf {
    std::env::var("VLINDER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .expect("Could not determine home directory")
                .join(".vlinder")
        })
}

pub fn agents_dir() -> PathBuf {
    vlinder_dir().join("agents")
}

pub fn models_dir() -> PathBuf {
    vlinder_dir().join("models")
}

pub fn agent_dir(name: &str) -> PathBuf {
    agents_dir().join(name)
}

pub fn agent_manifest_path(name: &str) -> PathBuf {
    agent_dir(name).join(format!("{}-agent.toml", name))
}

pub fn agent_db_path(name: &str) -> PathBuf {
    agent_dir(name).join(format!("{}.db", name))
}

pub fn conversations_dir() -> PathBuf {
    vlinder_dir().join("conversations")
}

pub fn logs_dir() -> PathBuf {
    vlinder_dir().join("logs")
}

pub fn registry_db_path() -> PathBuf {
    vlinder_dir().join("registry.db")
}

pub fn dag_db_path() -> PathBuf {
    vlinder_dir().join("dag.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = Config::default();
        assert_eq!(config.logging.level, "warn");
        assert_eq!(config.ollama.endpoint, "http://localhost:11434");
    }

    #[test]
    fn env_override_ollama_endpoint() {
        std::env::set_var("VLINDER_OLLAMA_ENDPOINT", "http://remote:11434");
        let mut config = Config::default();
        config.apply_env_overrides();
        std::env::remove_var("VLINDER_OLLAMA_ENDPOINT");

        assert_eq!(config.ollama.endpoint, "http://remote:11434");
    }

    #[test]
    fn test_loader_returns_custom_config() {
        let custom = Config {
            ollama: OllamaConfig {
                endpoint: "http://custom:9999".to_string(),
            },
            ..Default::default()
        };

        let loader = TestLoader(custom);
        let config = Config::load_with(&loader);

        assert_eq!(config.ollama.endpoint, "http://custom:9999");
    }

    #[test]
    fn default_distributed_config() {
        let config = Config::default();
        assert!(!config.distributed.enabled);
        assert_eq!(config.distributed.registry_addr, "http://127.0.0.1:9090");
        assert_eq!(config.distributed.harness_addr, "http://127.0.0.1:9091");
        assert_eq!(config.distributed.secret_addr, "http://127.0.0.1:9093");
        assert_eq!(config.distributed.workers.registry, 1);
        assert_eq!(config.distributed.workers.harness, 1);
        assert_eq!(config.distributed.workers.agent.container, 1);
        assert_eq!(config.distributed.workers.inference.ollama, 1);
        assert_eq!(config.distributed.workers.embedding.ollama, 1);
        assert_eq!(config.distributed.workers.storage.object.sqlite, 1);
        assert_eq!(config.distributed.workers.storage.vector.sqlite, 1);
    }

    #[test]
    fn env_override_distributed_enabled() {
        std::env::set_var("VLINDER_DISTRIBUTED_ENABLED", "true");
        std::env::set_var("VLINDER_DISTRIBUTED_REGISTRY_ADDR", "http://remote:9090");
        let mut config = Config::default();
        config.apply_env_overrides();
        std::env::remove_var("VLINDER_DISTRIBUTED_ENABLED");
        std::env::remove_var("VLINDER_DISTRIBUTED_REGISTRY_ADDR");

        assert!(config.distributed.enabled);
        assert_eq!(config.distributed.registry_addr, "http://remote:9090");
    }

    #[test]
    fn env_override_worker_counts() {
        std::env::set_var("VLINDER_WORKERS_AGENT_CONTAINER", "4");
        std::env::set_var("VLINDER_WORKERS_INFERENCE_OLLAMA", "2");
        let mut config = Config::default();
        config.apply_env_overrides();
        std::env::remove_var("VLINDER_WORKERS_AGENT_CONTAINER");
        std::env::remove_var("VLINDER_WORKERS_INFERENCE_OLLAMA");

        assert_eq!(config.distributed.workers.agent.container, 4);
        assert_eq!(config.distributed.workers.inference.ollama, 2);
    }
}
