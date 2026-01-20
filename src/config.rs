use serde::Deserialize;
use std::path::PathBuf;

/// Application configuration loaded from .vlinder/config.toml
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// App log level: trace, debug, info, warn, error
    pub level: String,
    /// llama.cpp/ggml log level (usually "error" to suppress noise)
    pub llama_level: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            logging: LoggingConfig::default(),
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

impl Config {
    /// Load config from .vlinder/config.toml, or return defaults if not found
    pub fn load() -> Self {
        let config_path = config_path();
        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(contents) => {
                    toml::from_str(&contents).unwrap_or_default()
                }
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    /// Build tracing EnvFilter from config
    pub fn tracing_filter(&self) -> String {
        format!(
            "{},ggml={},llama={}",
            self.logging.level,
            self.logging.llama_level,
            self.logging.llama_level
        )
    }
}

/// Path to config file
pub fn config_path() -> PathBuf {
    vlinder_dir().join("config.toml")
}

/// Base directory for vlinder data (agents, models, storage)
///
/// Resolution order:
/// 1. Env var VLINDER_DIR
/// 2. Default: .vlinder in current directory
pub fn vlinder_dir() -> PathBuf {
    std::env::var("VLINDER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".vlinder"))
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

pub fn agent_vlinderfile_path(name: &str) -> PathBuf {
    agent_dir(name).join("Vlinderfile")
}

pub fn agent_db_path(name: &str) -> PathBuf {
    agent_dir(name).join(format!("{}.db", name))
}

pub fn model_path(model_name: &str) -> PathBuf {
    models_dir().join(format!("{}.gguf", model_name))
}
