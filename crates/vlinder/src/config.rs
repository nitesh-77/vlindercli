//! CLI-specific configuration — reads ~/.vlinder/client.toml.
//!
//! Resolution order (highest priority first):
//! 1. Environment variable: VLINDER_DAEMON_<KEY> (or VLINDER_DISTRIBUTED_<KEY> fallback)
//! 2. Config file: ~/.vlinder/client.toml
//! 3. Default value

use serde::Deserialize;
use std::path::PathBuf;

// ============================================================================
// Config Structs
// ============================================================================

/// CLI configuration loaded from ~/.vlinder/client.toml.
///
/// Contains only what the CLI needs: logging level and daemon endpoint
/// addresses. The daemon's own config (worker counts, Podman socket,
/// queue backend, etc.) lives in config.toml and is not loaded here.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    pub logging: LoggingConfig,
    pub daemon: DaemonConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub registry_addr: String,
    pub harness_addr: String,
    pub state_addr: String,
    pub secret_addr: String,
    pub catalog_addr: String,
}

// ============================================================================
// Defaults
// ============================================================================

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "warn".to_string(),
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            registry_addr: "http://127.0.0.1:9090".to_string(),
            harness_addr: "http://127.0.0.1:9091".to_string(),
            state_addr: "http://127.0.0.1:9092".to_string(),
            secret_addr: "http://127.0.0.1:9093".to_string(),
            catalog_addr: "http://127.0.0.1:9094".to_string(),
        }
    }
}

// ============================================================================
// Loading
// ============================================================================

impl CliConfig {
    /// Load config: file → env overrides → return.
    pub fn load() -> Self {
        let mut config = Self::load_from_file();
        config.apply_env_overrides();
        config
    }

    fn load_from_file() -> Self {
        let path = vlinder_dir().join("client.toml");
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    /// Apply environment variable overrides.
    ///
    /// For daemon addresses, `VLINDER_DAEMON_*` takes precedence, falling
    /// back to `VLINDER_DISTRIBUTED_*` for backward compatibility with
    /// scripts that set the old variable names.
    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("VLINDER_LOGGING_LEVEL") {
            self.logging.level = v;
        }

        self.daemon.registry_addr = env_with_fallback(
            "VLINDER_DAEMON_REGISTRY_ADDR",
            "VLINDER_DISTRIBUTED_REGISTRY_ADDR",
            &self.daemon.registry_addr,
        );
        self.daemon.harness_addr = env_with_fallback(
            "VLINDER_DAEMON_HARNESS_ADDR",
            "VLINDER_DISTRIBUTED_HARNESS_ADDR",
            &self.daemon.harness_addr,
        );
        self.daemon.state_addr = env_with_fallback(
            "VLINDER_DAEMON_STATE_ADDR",
            "VLINDER_DISTRIBUTED_STATE_ADDR",
            &self.daemon.state_addr,
        );
        self.daemon.secret_addr = env_with_fallback(
            "VLINDER_DAEMON_SECRET_ADDR",
            "VLINDER_DISTRIBUTED_SECRET_ADDR",
            &self.daemon.secret_addr,
        );
        self.daemon.catalog_addr = env_with_fallback(
            "VLINDER_DAEMON_CATALOG_ADDR",
            "VLINDER_DISTRIBUTED_CATALOG_ADDR",
            &self.daemon.catalog_addr,
        );
    }
}

/// Read `primary` env var; if unset, fall back to `fallback`; if both
/// unset, return `default`.
fn env_with_fallback(primary: &str, fallback: &str, default: &str) -> String {
    std::env::var(primary)
        .or_else(|_| std::env::var(fallback))
        .unwrap_or_else(|_| default.to_string())
}

// ============================================================================
// Path helpers
// ============================================================================

/// Base directory for vlinder data.
///
/// Resolution: VLINDER_DIR env var → ~/.vlinder
pub fn vlinder_dir() -> PathBuf {
    std::env::var("VLINDER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .expect("Could not determine home directory")
                .join(".vlinder")
        })
}

pub fn logs_dir() -> PathBuf {
    vlinder_dir().join("logs")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = CliConfig::default();
        assert_eq!(config.logging.level, "warn");
        assert_eq!(config.daemon.registry_addr, "http://127.0.0.1:9090");
        assert_eq!(config.daemon.harness_addr, "http://127.0.0.1:9091");
        assert_eq!(config.daemon.state_addr, "http://127.0.0.1:9092");
        assert_eq!(config.daemon.secret_addr, "http://127.0.0.1:9093");
        assert_eq!(config.daemon.catalog_addr, "http://127.0.0.1:9094");
    }

    #[test]
    fn env_override_logging_level() {
        std::env::set_var("VLINDER_LOGGING_LEVEL", "debug");
        let mut config = CliConfig::default();
        config.apply_env_overrides();
        std::env::remove_var("VLINDER_LOGGING_LEVEL");

        assert_eq!(config.logging.level, "debug");
    }

    #[test]
    fn env_override_daemon_addr() {
        std::env::set_var("VLINDER_DAEMON_REGISTRY_ADDR", "http://remote:9090");
        let mut config = CliConfig::default();
        config.apply_env_overrides();
        std::env::remove_var("VLINDER_DAEMON_REGISTRY_ADDR");

        assert_eq!(config.daemon.registry_addr, "http://remote:9090");
    }

    #[test]
    fn env_fallback_to_distributed_var() {
        // Old env var name still works when new name is unset
        std::env::set_var("VLINDER_DISTRIBUTED_HARNESS_ADDR", "http://legacy:9091");
        let mut config = CliConfig::default();
        config.apply_env_overrides();
        std::env::remove_var("VLINDER_DISTRIBUTED_HARNESS_ADDR");

        assert_eq!(config.daemon.harness_addr, "http://legacy:9091");
    }

    #[test]
    fn primary_env_beats_fallback() {
        std::env::set_var("VLINDER_DAEMON_STATE_ADDR", "http://new:9092");
        std::env::set_var("VLINDER_DISTRIBUTED_STATE_ADDR", "http://old:9092");
        let mut config = CliConfig::default();
        config.apply_env_overrides();
        std::env::remove_var("VLINDER_DAEMON_STATE_ADDR");
        std::env::remove_var("VLINDER_DISTRIBUTED_STATE_ADDR");

        assert_eq!(config.daemon.state_addr, "http://new:9092");
    }

    #[test]
    fn parses_toml() {
        let toml_str = r#"
[logging]
level = "debug"

[daemon]
registry_addr = "http://10.0.0.1:9090"
harness_addr  = "http://10.0.0.1:9091"
state_addr    = "http://10.0.0.1:9092"
secret_addr   = "http://10.0.0.1:9093"
catalog_addr  = "http://10.0.0.1:9094"
"#;
        let config: CliConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.daemon.registry_addr, "http://10.0.0.1:9090");
        assert_eq!(config.daemon.catalog_addr, "http://10.0.0.1:9094");
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let toml_str = r#"
[logging]
level = "info"
"#;
        let config: CliConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.logging.level, "info");
        // daemon section uses defaults
        assert_eq!(config.daemon.registry_addr, "http://127.0.0.1:9090");
    }
}
