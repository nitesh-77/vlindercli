//! Message types for queue communication.

use std::fmt;
use uuid::Uuid;

/// A message that travels through the queue.
///
/// All messages expect a response - no fire-and-forget.
#[derive(Clone, Debug)]
pub struct Message {
    pub id: MessageId,
    pub payload: Vec<u8>,
    pub reply_to: String,
    pub correlation_id: Option<MessageId>,
}

impl Message {
    /// Create a request that expects a response.
    pub fn request(payload: Vec<u8>, reply_to: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            payload,
            reply_to: reply_to.into(),
            correlation_id: None,
        }
    }

    /// Create a response to a request.
    pub fn response(payload: Vec<u8>, reply_to: impl Into<String>, correlation_id: MessageId) -> Self {
        Self {
            id: MessageId::new(),
            payload,
            reply_to: reply_to.into(),
            correlation_id: Some(correlation_id),
        }
    }
}

// --- Supporting types ---

/// Unique identifier for a message.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageId(String);

impl MessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for MessageId {
    fn from(s: String) -> Self {
        Self(s)
    }
}
