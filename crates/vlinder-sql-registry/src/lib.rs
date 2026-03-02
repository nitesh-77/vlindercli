pub mod persistent;
pub mod registry_service;
pub mod storage;

pub use persistent::PersistentRegistry;
pub use storage::SqliteRegistryRepository;

use vlinder_core::domain::Provider;

/// Configuration for the registry crate.
///
/// Describes the cluster topology relevant to the registry: which inference
/// and embedding providers are available. Built by the supervisor from its
/// knowledge of spawned workers.
pub struct RegistryConfig {
    pub inference_engines: Vec<Provider>,
    pub embedding_engines: Vec<Provider>,
}
