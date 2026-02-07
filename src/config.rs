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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// App log level: trace, debug, info, warn, error
    pub level: String,
    /// llama.cpp/ggml log level (usually "error" to suppress noise)
    pub llama_level: String,
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
    /// Worker process counts by type
    pub workers: WorkerCounts,
}

/// Worker process counts for each service type.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WorkerCounts {
    /// Number of registry worker processes
    pub registry: u32,
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
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            llama_level: "error".to_string(),
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
            workers: WorkerCounts::default(),
        }
    }
}

impl Default for WorkerCounts {
    fn default() -> Self {
        Self {
            registry: 1,
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
        if let Ok(v) = std::env::var("VLINDER_LOGGING_LLAMA_LEVEL") {
            self.logging.llama_level = v;
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

        // Worker counts (flat env vars for simplicity)
        if let Ok(v) = std::env::var("VLINDER_WORKERS_REGISTRY") {
            self.distributed.workers.registry = v.parse().unwrap_or(1);
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
    }

    /// Build tracing EnvFilter from config.
    pub fn tracing_filter(&self) -> String {
        format!(
            "{},ggml={},llama={}",
            self.logging.level,
            self.logging.llama_level,
            self.logging.llama_level
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

pub fn registry_db_path() -> PathBuf {
    vlinder_dir().join("registry.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = Config::default();
        assert_eq!(config.logging.level, "info");
        assert_eq!(config.ollama.endpoint, "http://localhost:11434");
    }

    #[test]
    fn env_override_ollama_endpoint() {
        std::env::set_var("VLINDER_OLLAMA_ENDPOINT", "http://remote:11434");
        let config = Config::load();
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
        assert_eq!(config.distributed.workers.registry, 1);
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
        let config = Config::load();
        std::env::remove_var("VLINDER_DISTRIBUTED_ENABLED");
        std::env::remove_var("VLINDER_DISTRIBUTED_REGISTRY_ADDR");

        assert!(config.distributed.enabled);
        assert_eq!(config.distributed.registry_addr, "http://remote:9090");
    }

    #[test]
    fn env_override_worker_counts() {
        std::env::set_var("VLINDER_WORKERS_AGENT_CONTAINER", "4");
        std::env::set_var("VLINDER_WORKERS_INFERENCE_OLLAMA", "2");
        let config = Config::load();
        std::env::remove_var("VLINDER_WORKERS_AGENT_CONTAINER");
        std::env::remove_var("VLINDER_WORKERS_INFERENCE_OLLAMA");

        assert_eq!(config.distributed.workers.agent.container, 4);
        assert_eq!(config.distributed.workers.inference.ollama, 2);
    }
}
