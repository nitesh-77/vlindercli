//! Message queue abstraction.
//!
//! The queue is the universal abstraction for all communication:
//! - Agent to runtime (infer, embed, store)
//! - Agent to agent (call_agent)
//! - Runtime to agent (invoke)

mod message;
mod traits;
mod in_memory;

pub use message::{Message, MessageId};
pub use traits::{MessageQueue, QueueError};
pub use in_memory::InMemoryQueue;
