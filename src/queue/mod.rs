//! Message queue abstraction.
//!
//! The queue is the universal abstraction for all communication:
//! - Agent to runtime (infer, embed, store)
//! - Agent to agent (call_agent)
//! - Runtime to agent (invoke)
//!
//! Implementations:
//! - `InMemoryQueue`: Single-process, for local development
//! - `NatsQueue`: Distributed, with JetStream durability

mod message;
mod traits;
mod in_memory;
mod nats;
mod worker;

use std::sync::Arc;
use crate::config::Config;

pub use message::{
    // Legacy types (still used during transition)
    Message, MessageId,
    // New observability types (ADR 044)
    SubmissionId, Sequence, HarnessType,
    InvokeMessage, RequestMessage, ResponseMessage, CompleteMessage,
    ExpectsReply, ObservableMessage,
};
pub use traits::{MessageQueue, PendingMessage, QueueError};
pub use in_memory::InMemoryQueue;
pub use nats::NatsQueue;
pub use worker::{process_one, WorkerError};

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
