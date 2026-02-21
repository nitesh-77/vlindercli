//! Queue factory functions — wires configuration to concrete queue implementations.
//!
//! Extracted from `queue/mod.rs` so that `queue/` can move into vlinder-core
//! without pulling in `config` or `state_service` dependencies.

use std::sync::Arc;

use crate::config::Config;
use crate::domain::{DagStore, MessageQueue, QueueError};
use crate::queue::{InMemoryQueue, NatsQueue, RecordingQueue};

/// Create a queue from configuration.
///
/// Returns `InMemoryQueue` for `backend = "memory"` (default),
/// or `NatsQueue` for `backend = "nats"`.
pub fn from_config() -> Result<Arc<dyn MessageQueue + Send + Sync>, QueueError> {
    let config = Config::load();
    match config.queue.backend.as_str() {
        "nats" => {
            let queue = NatsQueue::connect(&config.queue.nats_url)?;
            Ok(Arc::new(queue))
        }
        "memory" | _ => {
            Ok(Arc::new(InMemoryQueue::new()))
        }
    }
}

/// Create a queue with synchronous DAG recording (transactional outbox).
///
/// Wraps the configured queue in a `RecordingQueue` that records
/// DAG nodes into the gRPC State Service on every send.
///
/// Fails if the State Service is unreachable — recording is not optional.
pub fn recording_from_config() -> Result<Arc<dyn MessageQueue + Send + Sync>, QueueError> {
    use crate::state_service::GrpcStateClient;

    let config = Config::load();
    let inner = from_config()?;

    let state_addr = if config.distributed.state_addr.starts_with("http://")
        || config.distributed.state_addr.starts_with("https://") {
        config.distributed.state_addr.clone()
    } else {
        format!("http://{}", config.distributed.state_addr)
    };

    let store: Arc<dyn DagStore> = Arc::new(
        GrpcStateClient::connect(&state_addr)
            .map_err(|e| QueueError::SendFailed(
                format!("state service at {} unreachable: {}", state_addr, e)
            ))?
    );

    Ok(Arc::new(RecordingQueue::new(inner, store)))
}
