//! Registry implementations (ADR 076).
//!
//! Domain types (Registry trait, Job, JobId, etc.) live in `crate::domain`.
//! This module contains concrete implementations:
//! - `InMemoryRegistry`: In-process with RwLock, used for tests and as read cache
//! - `PersistentRegistry`: Write-through to SQLite via SqliteRegistryRepository

mod persistent;

// Re-export from domain (canonical location) for backward compatibility
pub use crate::domain::InMemoryRegistry;
pub(crate) use persistent::PersistentRegistry;

use std::sync::Arc;
use crate::config::{registry_db_path, Config};
use crate::domain::{Registry, SecretStore};
use crate::registry_service::{GrpcRegistryClient, ping_registry};

/// Connect to the registry — gRPC in distributed mode, local SQLite otherwise.
///
/// CLI commands use this to get a `dyn Registry` without knowing which
/// concrete implementation is behind it.
pub fn open_registry(config: &Config, secret_store: Arc<dyn SecretStore>) -> Option<Arc<dyn Registry>> {
    if config.distributed.enabled {
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
    } else {
        let db_path = registry_db_path();
        match PersistentRegistry::open(&db_path, config, secret_store) {
            Ok(r) => Some(Arc::new(r)),
            Err(e) => {
                eprintln!("Failed to open registry: {}", e);
                None
            }
        }
    }
}
