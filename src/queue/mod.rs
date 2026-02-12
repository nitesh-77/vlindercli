//! Message queue implementations (ADR 044).
//!
//! Domain types (messages, traits, diagnostics) live in `crate::domain`.
//! This module contains concrete implementations:
//! - `InMemoryQueue`: Single-process, for local development
//! - `NatsQueue`: Distributed, with JetStream durability

mod in_memory;
mod nats;

use std::sync::Arc;
use crate::config::Config;
use crate::domain::{MessageQueue, QueueError};

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
