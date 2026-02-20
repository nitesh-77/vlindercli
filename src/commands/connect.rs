//! Shared connection helpers for CLI commands.
//!
//! Provides gRPC client construction for registry, harness, and DAG store.
//! Each helper pings the service first and exits with a clear error on failure.

use std::sync::Arc;

use vlindercli::config::Config;
use vlindercli::domain::{DagStore, Harness, Registry};
use vlindercli::harness_service::{GrpcHarnessClient, ping_harness};
use vlindercli::registry_service::{GrpcRegistryClient, ping_registry};
use vlindercli::state_service::GrpcStateClient;

/// Connect to the registry via gRPC, exiting on failure.
pub fn connect_registry(config: &Config) -> Arc<dyn Registry> {
    let registry_addr = normalize_addr(&config.distributed.registry_addr);

    if ping_registry(&registry_addr).is_none() {
        eprintln!("Cannot reach registry at {}. Is the daemon running?", registry_addr);
        std::process::exit(1);
    }

    Arc::new(
        GrpcRegistryClient::connect(&registry_addr)
            .expect("Failed to connect to registry")
    )
}

/// Connect to the harness via gRPC, exiting on failure.
pub fn connect_harness(config: &Config) -> Box<dyn Harness> {
    let harness_addr = normalize_addr(&config.distributed.harness_addr);

    if ping_harness(&harness_addr).is_none() {
        eprintln!("Cannot reach harness at {}. Is the daemon running?", harness_addr);
        std::process::exit(1);
    }

    Box::new(
        GrpcHarnessClient::connect(&harness_addr)
            .expect("Failed to connect to harness")
    )
}

/// Open the appropriate DagStore: local SQLite or remote gRPC.
pub fn open_dag_store(config: &Config) -> Option<Box<dyn DagStore>> {
    if config.distributed.enabled {
        let state_addr = normalize_addr(&config.distributed.state_addr);
        match GrpcStateClient::connect(&state_addr) {
            Ok(client) => Some(Box::new(client)),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to connect to state service, skipping state read");
                None
            }
        }
    } else {
        let db_path = vlindercli::config::dag_db_path();
        if !db_path.exists() {
            return None;
        }
        match vlindercli::storage::dag_store::SqliteDagStore::open(&db_path) {
            Ok(store) => Some(Box::new(store)),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to open DAG store, skipping state read");
                None
            }
        }
    }
}

/// Ensure an address has an http:// or https:// scheme.
fn normalize_addr(addr: &str) -> String {
    if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        format!("http://{}", addr)
    }
}
