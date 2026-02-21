//! Secret store implementations (ADR 083).
//!
//! Domain types (trait, errors) live in `crate::domain`.
//! This module contains concrete implementations:
//! - `InMemorySecretStore`: Single-process, for testing
//! - `NatsSecretStore`: Distributed, with NATS KV persistence

mod nats;

use std::sync::Arc;
use crate::config::{Config, QueueBackend};
use crate::domain::{SecretStore, SecretStoreError};

// Re-export from domain (canonical location) for backward compatibility
pub use crate::domain::InMemorySecretStore;
pub use nats::NatsSecretStore;

/// Create a secret store from configuration.
///
/// Requires `backend = Nats` — secrets must persist across processes.
/// In test builds with `Memory` queue backend, returns an `InMemorySecretStore`.
///
/// Reuses the queue's `nats_url` config — same NATS cluster, different bucket.
pub fn from_config(config: &Config) -> Result<Arc<dyn SecretStore>, SecretStoreError> {
    match config.queue.backend {
        QueueBackend::Nats => {
            let store = NatsSecretStore::connect(&config.queue.nats_url)?;
            Ok(Arc::new(store))
        }
        #[cfg(any(test, feature = "test-support"))]
        QueueBackend::Memory => {
            Ok(Arc::new(InMemorySecretStore::new()))
        }
    }
}
