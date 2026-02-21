//! Secret store implementations (ADR 083).
//!
//! Domain types (trait, errors) live in `crate::domain`.
//! This module contains concrete implementations:
//! - `InMemorySecretStore`: Single-process, for testing
//! - `NatsSecretStore`: Distributed, with NATS KV persistence

mod nats;

use std::sync::Arc;
use crate::config::Config;
use crate::domain::{SecretStore, SecretStoreError};

// Re-export from domain (canonical location) for backward compatibility
pub use crate::domain::InMemorySecretStore;
pub use nats::NatsSecretStore;

/// Create a secret store from configuration.
///
/// Requires `backend = "nats"` — secrets must persist across processes.
/// `InMemorySecretStore` exists for unit tests only, not wired here.
///
/// Reuses the queue's `nats_url` config — same NATS cluster, different bucket.
pub fn from_config() -> Result<Arc<dyn SecretStore>, SecretStoreError> {
    let config = Config::load();
    match config.queue.backend.as_str() {
        "nats" => {
            let store = NatsSecretStore::connect(&config.queue.nats_url)?;
            Ok(Arc::new(store))
        }
        other => Err(SecretStoreError::StoreFailed(format!(
            "unsupported secret store backend: '{}' (set queue.backend = \"nats\" in config)",
            other,
        ))),
    }
}
