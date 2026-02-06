//! Message queue abstraction (ADR 044).
//!
//! The queue is the universal abstraction for all communication using typed messages:
//! - `InvokeMessage`: Harness → Runtime (start a submission)
//! - `RequestMessage`: Runtime → Service (agent calls a service)
//! - `ResponseMessage`: Service → Runtime (service replies)
//! - `CompleteMessage`: Runtime → Harness (submission finished)
//!
//! Implementations:
//! - `InMemoryQueue`: Single-process, for local development
//! - `NatsQueue`: Distributed, with JetStream durability

mod message;
mod traits;
mod in_memory;
mod nats;

use std::sync::Arc;
use crate::config::Config;
use crate::domain::ResourceId;

pub use message::{
    MessageId, SubmissionId, Sequence, SequenceCounter, HarnessType,
    InvokeMessage, RequestMessage, ResponseMessage, CompleteMessage,
    ExpectsReply, ObservableMessage,
};
pub use traits::{MessageQueue, QueueError};
pub use in_memory::InMemoryQueue;
pub use nats::NatsQueue;

/// Extract a short routing key from an agent's ResourceId.
///
/// This is the canonical function for computing the NATS subject token
/// for an agent. Both send and receive must use the same key.
///
/// - `container://localhost/echo-container:latest` → "echo-container_latest"
/// - `memory://test-agent` → "test-agent"
/// TODO: This smells. We should just have a queue friendly unique short name
/// in the agent entity
pub fn agent_routing_key(agent_id: &ResourceId) -> String {
    if let Some(path) = agent_id.path() {
        if let Some(filename) = path.rsplit('/').next() {
            let name = filename.strip_suffix(".wasm").unwrap_or(filename);
            // Replace colons with underscores (colons invalid in NATS subjects)
            let name = name.replace(':', "_");
            if !name.is_empty() {
                return name;
            }
        }
    }
    if let Some(authority) = agent_id.authority() {
        return authority.to_string();
    }
    agent_id.as_str().to_string()
}

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
