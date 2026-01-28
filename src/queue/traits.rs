//! Queue trait definition.

use super::Message;
use std::fmt;

/// A message queue for sending and receiving messages.
pub trait MessageQueue {
    /// Send a message to a named queue.
    fn send(&self, queue: &str, msg: Message) -> Result<(), QueueError>;

    /// Receive a message from a named queue.
    /// Blocks until a message is available.
    fn receive(&self, queue: &str) -> Result<Message, QueueError>;
}

// --- Errors ---

#[derive(Debug)]
pub enum QueueError {
    /// Queue does not exist
    QueueNotFound(String),
    /// Failed to send message
    SendFailed(String),
    /// Failed to receive message
    ReceiveFailed(String),
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueError::QueueNotFound(q) => write!(f, "queue not found: {}", q),
            QueueError::SendFailed(msg) => write!(f, "send failed: {}", msg),
            QueueError::ReceiveFailed(msg) => write!(f, "receive failed: {}", msg),
        }
    }
}

impl std::error::Error for QueueError {}
