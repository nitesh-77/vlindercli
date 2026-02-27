//! Queue factory functions — wires configuration to concrete queue implementations.
//!
//! Extracted from `queue/mod.rs` so that `queue/` can move into vlinder-core
//! without pulling in `config` or `state_service` dependencies.

use std::sync::Arc;

use crate::config::{Config, QueueBackend, StateBackend};
use vlinder_core::domain::{DagStore, MessageQueue, QueueError};
use vlinder_nats::NatsQueue;
use vlinder_core::queue::RecordingQueue;

/// Create a queue from configuration.
///
/// Returns `NatsQueue` in production. In test builds, `Memory` backend
/// returns an `InMemoryQueue` (no network required).
pub fn from_config(config: &Config) -> Result<Arc<dyn MessageQueue + Send + Sync>, QueueError> {
    match config.queue.backend {
        QueueBackend::Nats => {
            let queue = NatsQueue::connect(&config.queue.nats_url)?;
            Ok(Arc::new(queue))
        }
        #[cfg(any(test, feature = "test-support"))]
        QueueBackend::Memory => {
            Ok(Arc::new(vlinder_core::queue::InMemoryQueue::new()))
        }
    }
}

/// Create a queue with synchronous DAG recording (transactional outbox).
///
/// Wraps the configured queue in a `RecordingQueue` that records
/// DAG nodes into the DagStore on every send.
///
/// In production, the DagStore is the gRPC State Service.
/// In test builds with `StateBackend::Memory`, uses `InMemoryDagStore`.
pub fn recording_from_config(config: &Config) -> Result<Arc<dyn MessageQueue + Send + Sync>, QueueError> {
    let inner = from_config(config)?;

    let store: Arc<dyn DagStore> = match config.state.backend {
        StateBackend::Grpc => {
            use vlinder_sql_state::state_service::GrpcStateClient;

            let state_addr = if config.distributed.state_addr.starts_with("http://")
                || config.distributed.state_addr.starts_with("https://") {
                config.distributed.state_addr.clone()
            } else {
                format!("http://{}", config.distributed.state_addr)
            };

            Arc::new(
                GrpcStateClient::connect(&state_addr)
                    .map_err(|e| QueueError::SendFailed(
                        format!("state service at {} unreachable: {}", state_addr, e)
                    ))?
            )
        }
        #[cfg(any(test, feature = "test-support"))]
        StateBackend::Memory => {
            use vlinder_core::domain::InMemoryDagStore;
            Arc::new(InMemoryDagStore::new())
        }
    };

    Ok(Arc::new(RecordingQueue::new(inner, store)))
}
