//! Registry helpers for vlinderd.
//!
//! Domain types (Registry trait, Job, JobId, etc.) live in `vlinder_core::domain`.
//! Concrete implementations (PersistentRegistry, SqliteRegistryRepository, gRPC
//! service) live in `vlinder_sql_registry`.
//!
//! This module retains:
//! - `InMemoryRegistry` re-export for backward compatibility
//! - `open_registry()` CLI helper (depends on Config)

// Re-export from domain (canonical location) for backward compatibility
pub use vlinder_core::domain::InMemoryRegistry;

use std::sync::Arc;
use crate::config::Config;
use vlinder_core::domain::Registry;
use vlinder_sql_registry::registry_service::{GrpcRegistryClient, ping_registry};

/// Connect to the registry via gRPC.
///
/// CLI commands use this to get a `dyn Registry` without knowing which
/// concrete implementation is behind it. The daemon owns the registry;
/// the CLI always talks to it over gRPC.
pub fn open_registry(config: &Config) -> Option<Arc<dyn Registry>> {
    let registry_addr = if config.distributed.registry_addr.starts_with("http://")
        || config.distributed.registry_addr.starts_with("https://") {
        config.distributed.registry_addr.clone()
    } else {
        format!("http://{}", config.distributed.registry_addr)
    };

    if ping_registry(&registry_addr).is_none() {
        eprintln!("Cannot reach registry at {}. Is the daemon running?", registry_addr);
        return None;
    }

    match GrpcRegistryClient::connect(&registry_addr) {
        Ok(client) => Some(Arc::new(client)),
        Err(e) => {
            eprintln!("Failed to connect to registry: {}", e);
            None
        }
    }
}
