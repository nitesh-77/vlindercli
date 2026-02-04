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
    pub queue: QueueConfig,
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
pub struct QueueConfig {
    /// Queue backend: "memory" or "nats"
    pub backend: String,
    /// NATS server URL (if backend = "nats")
    pub nats_url: String,
}

// ============================================================================
// Defaults
// ============================================================================

impl Default for Config {
    fn default() -> Self {
        Self {
            logging: LoggingConfig::default(),
            ollama: OllamaConfig::default(),
            queue: QueueConfig::default(),
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

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            backend: "memory".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
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
        if let Ok(v) = std::env::var("VLINDER_LOGGING_LLAMA_LEVEL") {
            self.logging.llama_level = v;
        }

        // Ollama
        if let Ok(v) = std::env::var("VLINDER_OLLAMA_ENDPOINT") {
            self.ollama.endpoint = v;
        }

        // Queue
        if let Ok(v) = std::env::var("VLINDER_QUEUE_BACKEND") {
            self.queue.backend = v;
        }
        if let Ok(v) = std::env::var("VLINDER_QUEUE_NATS_URL") {
            self.queue.nats_url = v;
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

pub fn agent_wasm_path(name: &str) -> PathBuf {
    let wasm_name = name.replace('-', "_");
    agent_dir(name).join(format!("{}.wasm", wasm_name))
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
}
