//! Queue trait definition.
//!
//! The `PendingMessage` pattern (ADR 043) enables explicit acknowledgment:
//! - `ack()`: Successful processing, remove message from queue
//! - `nack()`: Processing failed, message should be redelivered
//!
//! This is critical for distributed mode where workers may crash
//! between receive and processing completion.

use super::Message;
use std::fmt;

// --- PendingMessage ---

/// A message awaiting acknowledgment.
///
/// Wraps a `Message` with explicit `ack()`/`nack()` methods for JetStream
/// acknowledgment semantics. The `ack()` and `nack()` methods consume `self`,
/// preventing double-acknowledgment at compile time.
///
/// # Example
///
/// ```ignore
/// let pending = queue.receive("my-queue")?;
/// let result = process(&pending.message);
/// if result.is_ok() {
///     pending.ack()?;  // Consumes pending
/// } else {
///     pending.nack()?; // Message will be redelivered
/// }
/// ```
pub struct PendingMessage {
    /// The underlying message.
    pub message: Message,
    ack_fn: Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
    nack_fn: Box<dyn FnOnce() -> Result<(), QueueError> + Send>,
}

impl PendingMessage {
    /// Create a new PendingMessage with custom ack/nack functions.
    pub fn new<A, N>(message: Message, ack: A, nack: N) -> Self
    where
        A: FnOnce() -> Result<(), QueueError> + Send + 'static,
        N: FnOnce() -> Result<(), QueueError> + Send + 'static,
    {
        Self {
            message,
            ack_fn: Box::new(ack),
            nack_fn: Box::new(nack),
        }
    }

    /// Acknowledge successful processing. Consumes self.
    ///
    /// For NATS: Acknowledges the message to JetStream.
    /// For InMemory: No-op (message already removed on receive).
    pub fn ack(self) -> Result<(), QueueError> {
        (self.ack_fn)()
    }

    /// Negative acknowledge. Message will be redelivered. Consumes self.
    ///
    /// For NATS: NAKs the message, causing JetStream to redeliver.
    /// For InMemory: No-op (no redelivery support).
    pub fn nack(self) -> Result<(), QueueError> {
        (self.nack_fn)()
    }
}

impl fmt::Debug for PendingMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingMessage")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

// --- MessageQueue Trait ---

/// A message queue for sending and receiving messages.
pub trait MessageQueue {
    /// Send a message to a named queue.
    fn send(&self, queue: &str, msg: Message) -> Result<(), QueueError>;

    /// Receive a message from a named queue.
    ///
    /// Returns a `PendingMessage` that must be explicitly acknowledged.
    /// Blocks until a message is available or times out.
    fn receive(&self, queue: &str) -> Result<PendingMessage, QueueError>;
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
    /// Receive timed out without a message (not an error, just no work)
    Timeout,
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueError::QueueNotFound(q) => write!(f, "queue not found: {}", q),
            QueueError::SendFailed(msg) => write!(f, "send failed: {}", msg),
            QueueError::ReceiveFailed(msg) => write!(f, "receive failed: {}", msg),
            QueueError::Timeout => write!(f, "receive timed out"),
        }
    }
}

impl std::error::Error for QueueError {}
