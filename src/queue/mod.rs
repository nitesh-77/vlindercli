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

pub use message::{
    MessageId, SubmissionId, Sequence, SequenceCounter, HarnessType,
    InvokeMessage, RequestMessage, ResponseMessage, CompleteMessage,
    ExpectsReply, ObservableMessage,
};
pub use traits::{MessageQueue, QueueError};
pub use in_memory::InMemoryQueue;
pub use nats::NatsQueue;

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
