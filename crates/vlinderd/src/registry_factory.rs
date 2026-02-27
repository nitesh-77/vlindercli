//! Registry factory — wires configuration to concrete Registry implementations.
//!
//! Follows the same pattern as `queue_factory` and `secret_store::from_config`.

use std::sync::Arc;

use crate::config::{Config, RegistryBackend};
use crate::domain::Registry;

/// Create a registry client from configuration.
///
/// Returns `GrpcRegistryClient` in production. In test builds, `Memory`
/// backend returns an `InMemoryRegistry` (no network required).
pub fn from_config(config: &Config) -> Result<Arc<dyn Registry>, Box<dyn std::error::Error>> {
    match config.distributed.registry_backend {
        RegistryBackend::Grpc => {
            use vlinder_proto::registry_service::GrpcRegistryClient;

            let addr = if config.distributed.registry_addr.starts_with("http://") {
                config.distributed.registry_addr.clone()
            } else {
                format!("http://{}", config.distributed.registry_addr)
            };

            let client = GrpcRegistryClient::connect(&addr)?;
            Ok(Arc::new(client))
        }
        #[cfg(any(test, feature = "test-support"))]
        RegistryBackend::Memory => {
            use crate::domain::InMemorySecretStore;
            use crate::registry::InMemoryRegistry;

            let secret_store = Arc::new(InMemorySecretStore::new());
            Ok(Arc::new(InMemoryRegistry::new(secret_store)))
        }
    }
}
