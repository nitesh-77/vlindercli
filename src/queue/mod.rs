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
    MessageId, SubmissionId, SessionId, Sequence, SequenceCounter, HarnessType,
    InvokeMessage, RequestMessage, ResponseMessage, CompleteMessage,
    ExpectsReply, ObservableMessage,
};
pub use traits::{MessageQueue, QueueError};
pub use in_memory::InMemoryQueue;
pub use nats::NatsQueue;

/// Extract the agent name from a registry-assigned ResourceId.
///
/// Registry IDs have the format `<registry>/agents/<name>`.
/// The last path component is the agent name, used as the NATS subject token.
pub fn agent_routing_key(agent_id: &ResourceId) -> String {
    if let Some(path) = agent_id.path() {
        if let Some(name) = path.rsplit('/').next() {
            if !name.is_empty() {
                return name.to_string();
            }
        }
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
