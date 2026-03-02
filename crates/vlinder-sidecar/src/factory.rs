//! Connection factories — production-only, no test backends.
//!
//! The sidecar always connects to real infrastructure: NATS for messaging,
//! gRPC for registry and state. No in-memory alternatives.

use std::sync::Arc;

use vlinder_core::domain::{DagStore, MessageQueue, QueueError, Registry};
use vlinder_core::queue::RecordingQueue;
use vlinder_nats::NatsQueue;
use vlinder_sql_registry::registry_service::GrpcRegistryClient;
use vlinder_sql_state::state_service::GrpcStateClient;

/// Connect to NATS and wrap with DAG recording via the State Service.
pub fn connect_queue(
    nats_url: &str,
    state_url: &str,
) -> Result<Arc<dyn MessageQueue + Send + Sync>, QueueError> {
    let inner = Arc::new(NatsQueue::connect(nats_url)?);
    let store: Arc<dyn DagStore> = Arc::new(GrpcStateClient::connect(state_url).map_err(|e| {
        QueueError::SendFailed(format!("state service at {} unreachable: {}", state_url, e))
    })?);
    Ok(Arc::new(RecordingQueue::new(inner, store)))
}

/// Connect to the Registry Service via gRPC.
pub fn connect_registry(
    registry_url: &str,
) -> Result<Arc<dyn Registry>, Box<dyn std::error::Error>> {
    let url = if registry_url.starts_with("http://") || registry_url.starts_with("https://") {
        registry_url.to_string()
    } else {
        format!("http://{}", registry_url)
    };
    let client = GrpcRegistryClient::connect(&url)?;
    Ok(Arc::new(client))
}
